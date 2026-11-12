use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Result};
use tracing::info;

use crate::config::AppConfig;
use crate::download::client::HttpClient;
use crate::engine::ServerRuntime;
use crate::i18n;
use crate::model::{self, ModelInfo};

#[derive(Debug, Clone)]
pub struct StatusLine {
    pub text: String,
    pub warn: bool,
}

impl StatusLine {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            warn: false,
        }
    }
    pub fn warn(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            warn: true,
        }
    }
}

pub struct App {
    pub store_dir: PathBuf,
    pub config: AppConfig,
    pub runtime: Arc<ServerRuntime>,
    pub http: HttpClient,
    pub server_running: bool,
    pub host: String,
    pub port: u16,
}

impl App {
    pub fn new(store_dir: PathBuf, host: String, port: u16) -> Result<Self> {
        let config = AppConfig::load(&store_dir)?;
        let runtime = Arc::new(ServerRuntime::new());
        let http = HttpClient::new()?;

        Ok(Self {
            store_dir,
            config,
            runtime,
            http,
            server_running: false,
            host,
            port,
        })
    }

    pub fn list_models(&self) -> Vec<ModelInfo> {
        model::list_asr_models(&self.store_dir)
    }

    pub fn ws_address(&self) -> String {
        let s = i18n::t();
        let state = if self.server_running {
            s.status_running
        } else {
            s.status_stopped
        };
        format!("ws://{}:{}/v1/asr  [{}]", self.host, self.port, state)
    }

    fn rel_or_none(&self, path: Option<&str>) -> String {
        match path {
            None => i18n::t().status_none.to_string(),
            Some(p) => {
                let pb = Path::new(p);
                let store = self.store_dir.as_path();
                pb.strip_prefix(store)
                    .map(|r| r.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| crate::path_display::for_log_path(pb))
            }
        }
    }

    fn primary_ip() -> String {
        UdpSocket::bind(("0.0.0.0", 0))
            .and_then(|socket| {
                socket.connect(("8.8.8.8", 80))?;
                socket.local_addr()
            })
            .map(|addr| addr.ip().to_string())
            .unwrap_or_else(|_| i18n::t().status_unavailable.to_string())
    }

    pub fn detailed_status_lines(&self) -> Vec<StatusLine> {
        let s = i18n::t();
        let models_dir = self.store_dir.join("models");
        let total_models = self.list_models().len();
        let ip_line = Self::primary_ip();
        let mut active_line = self.rel_or_none(self.config.active_model.as_deref());
        if let Some(active) = self.config.active_model.as_deref() {
            if let Ok(info) =
                crate::model::detect::detect_model(Path::new(active), Some(&self.config))
            {
                let mode = if info.streaming {
                    s.status_mode_streaming
                } else {
                    s.status_mode_offline
                };
                active_line = format!("{} [{} {}]", active_line, mode, info.family);
            }
        }
        let vad_line = self.rel_or_none(self.config.vad_model.as_deref());
        let punct_line = self.rel_or_none(self.config.punctuation_model.as_deref());

        let mut lines = vec![
            StatusLine::plain(format!("{}: {ip_line}", s.status_ip)),
            StatusLine::plain(format!(
                "{}: {}",
                s.status_models_dir,
                crate::path_display::for_log_path(&models_dir)
            )),
            StatusLine::plain(format!(
                "{}: {}",
                s.status_discovered,
                s.status_models_count_fmt
                    .replace("{}", &total_models.to_string())
            )),
            StatusLine::plain(format!("{}: {active_line}", s.status_active_asr)),
            StatusLine::plain(format!("{}: {vad_line}", s.status_vad)),
            StatusLine::plain(format!("{}: {punct_line}", s.status_punctuation)),
            StatusLine::plain(format!("{}: {}", s.status_websocket, self.ws_address())),
        ];

        if total_models == 0 && self.config.active_model.is_none() {
            lines.push(StatusLine::plain(String::new()));
            lines.push(StatusLine::warn(s.status_no_models_hint.to_string()));
        } else if self.config.active_model.is_none() {
            lines.push(StatusLine::plain(String::new()));
            lines.push(StatusLine::warn(s.status_no_active_hint.to_string()));
        }
        lines
    }

    pub fn delete_model(&mut self, index: usize) -> Result<()> {
        let models = self.list_models();
        if index >= models.len() {
            bail!("Invalid model index");
        }
        let model = &models[index];
        let dir = model.dir.clone();

        if self
            .config
            .active_model
            .as_ref()
            .map(|p| p == &dir.to_string_lossy().as_ref())
            .unwrap_or(false)
        {
            self.config.active_model = None;
            self.config.save(&self.store_dir)?;
        }

        model::import::delete_model_dir(&dir)?;
        info!("Deleted model: {}", model.name);
        Ok(())
    }

    pub fn list_aux_models(&self) -> Vec<model::AuxModel> {
        model::list_aux_models(&self.store_dir)
    }

    pub fn delete_aux_model(&mut self, dir: &Path) -> Result<()> {
        let models_root = model::models_dir(&self.store_dir);
        if !dir.starts_with(&models_root) {
            bail!("Refusing to delete outside the models directory");
        }

        fn clear_under(slot: &mut Option<String>, dir: &Path) -> bool {
            let matches = slot.as_deref().is_some_and(|p| {
                let path = Path::new(p);
                path.starts_with(dir)
                    || path.parent().and_then(|parent| parent.file_name()) == dir.file_name()
            });
            if matches {
                *slot = None;
                true
            } else {
                false
            }
        }

        let mut changed = false;
        changed |= clear_under(&mut self.config.vad_model, dir);
        changed |= clear_under(&mut self.config.punctuation_model, dir);
        changed |= clear_under(&mut self.config.denoiser_model, dir);
        changed |= clear_under(&mut self.config.speaker_model, dir);
        changed |= clear_under(&mut self.config.voxlock_voice_sample, dir);
        if clear_under(&mut self.config.translation_model, dir) {
            self.config.translation_enabled = false;
            changed = true;
        }
        if changed {
            self.config.save(&self.store_dir)?;
        }

        model::import::delete_model_dir(dir)?;
        info!(
            "Deleted auxiliary model: {}",
            crate::path_display::for_log_path(dir)
        );
        Ok(())
    }

    pub fn load_active_model(&mut self) -> Result<()> {
        let active = self.config.active_model.clone();
        if let Some(path) = active {
            let model_dir = PathBuf::from(&path);
            if model_dir.exists() {
                let info =
                    model::detect::detect_model_with_metadata(&model_dir, Some(&self.config))?;
                self.runtime.load_model(&info, &self.config)?;
                return Ok(());
            }
        }
        bail!("No active model configured");
    }

    pub fn reload_active_runtime(&self) -> Result<()> {
        let active = self.config.active_model.clone();
        if let Some(path) = active {
            let model_dir = PathBuf::from(&path);
            if model_dir.exists() {
                let info =
                    model::detect::detect_model_with_metadata(&model_dir, Some(&self.config))?;
                self.runtime.load_model(&info, &self.config)?;
                return Ok(());
            }
        }
        bail!("No active model configured");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[tokio::test]
    async fn delete_aux_model_removes_dir_and_clears_config() {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let store = std::env::temp_dir().join(format!(
            "sherpa-aux-delete-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let models = crate::model::models_dir(&store);
        std::fs::create_dir_all(&models).unwrap();
        let dir = models.join("Hy-MT2-1.8B-Q4_K_M");
        std::fs::create_dir(&dir).unwrap();
        let gguf = dir.join("Hy-MT2-1.8B-Q4_K_M.gguf");
        std::fs::write(&gguf, b"x").unwrap();

        let mut app = App::new(store.clone(), "127.0.0.1".to_string(), 0).unwrap();
        app.config.translation_model = Some(gguf.to_string_lossy().into_owned());
        app.config.translation_enabled = true;
        app.config.vad_model = Some(dir.join("silero_vad.onnx").to_string_lossy().into_owned());
        app.config.save(&store).unwrap();

        assert!(app.delete_aux_model(Path::new("/")).is_err());
        app.delete_aux_model(&dir).unwrap();
        assert!(!dir.exists());
        assert!(app.config.translation_model.is_none());
        assert!(!app.config.translation_enabled);
        assert!(app.config.vad_model.is_none());

        let reloaded = AppConfig::load(&store).unwrap();
        assert!(reloaded.translation_model.is_none());
        assert!(!reloaded.translation_enabled);
        assert!(reloaded.vad_model.is_none());

        std::fs::remove_dir_all(store).unwrap();
    }
}
