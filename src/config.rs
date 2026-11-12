use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorRule {
    pub pattern: String,
    pub replacement: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub active_model: Option<String>,
    pub vad_model: Option<String>,
    pub punctuation_model: Option<String>,
    pub denoiser_model: Option<String>,
    /// VoxLock reference voice sample WAV. Must contain at least 3 seconds of speech.
    #[serde(default)]
    pub voxlock_voice_sample: Option<String>,
    pub github_token: Option<String>,
    #[serde(default = "default_ttl")]
    pub release_cache_ttl_seconds: u64,
    #[serde(default)]
    pub hosts_enabled: bool,
    #[serde(default)]
    pub hosts_source_url: String,
    #[serde(default)]
    pub hosts: HashMap<String, String>,
    #[serde(default)]
    pub mirrors: Vec<MirrorRule>,
    #[serde(default = "default_max_decode_secs")]
    pub offline_end_whole_decode_max_seconds: u64,
    /// VAD silero threshold (0.0–1.0). Default 0.5.
    #[serde(default = "default_vad_threshold")]
    pub vad_threshold: f64,
    /// Silence duration in ms that triggers a speech-end boundary (applied to VAD min_silence_duration).
    #[serde(default = "default_silence_ms")]
    pub silence_ms: i32,
    /// Path to a speaker embedding model (3dspeaker / wespeaker / nemo-speakernet)
    /// used together with client-provided reference embeddings for per-session voice verification.
    #[serde(default)]
    pub speaker_model: Option<String>,
    /// Number of threads for ASR inference. Auto-detected from CPU cores.
    #[serde(default = "default_num_threads")]
    pub num_threads: i32,
    /// Schema version for config migration. Bump when defaults change.
    #[serde(default)]
    pub config_version: u32,
    /// Interface language code: "en", "zh-CN", "zh-TW". None = auto-detect from locale.
    #[serde(default)]
    pub language: Option<String>,
    /// Enable local llama.cpp translation for final ASR results.
    #[serde(default)]
    pub translation_enabled: bool,
    /// Target language code for translation output.
    #[serde(default = "default_translation_target")]
    pub translation_target: String,
    /// Path to the GGUF translation model.
    #[serde(default)]
    pub translation_model: Option<String>,
    /// Path to llama.cpp server binary, for example build/bin/llama-server.
    #[serde(default, alias = "translation_llama_cli")]
    pub translation_llama_server: Option<String>,
    /// Optional directory containing llama.cpp shared libraries.
    #[serde(default)]
    pub translation_llama_lib_dir: Option<String>,
    /// Number of GPU layers passed to llama.cpp. 0 = CPU.
    #[serde(default)]
    pub translation_gpu_layers: i32,
    /// llama.cpp context size for translation.
    #[serde(default = "default_translation_ctx_size")]
    pub translation_ctx_size: i32,
    /// llama.cpp generation temperature for translation.
    #[serde(default = "default_translation_temperature")]
    pub translation_temperature: f32,
    /// Extra arguments appended to the llama.cpp command.
    #[serde(default)]
    pub translation_extra_args: Vec<String>,
    /// Translation mode: "always" (translate every segment) or "auto" (only when trigger phrase detected).
    #[serde(default = "default_translation_mode")]
    pub translation_mode: String,
    /// Trigger phrases for "auto" mode, comma-separated. Default: "翻译,翻訳,translate".
    #[serde(default = "default_translation_triggers")]
    pub translation_triggers: String,
    /// When true, strip the trigger phrase and language hint from input before sending to model.
    #[serde(default = "default_true")]
    pub translation_strip_hints: bool,
    /// Prompt preset for translation: "auto", "hy_mt2", "qwen", "generic", "custom".
    #[serde(default)]
    pub translation_prompt_preset: Option<String>,
    /// Custom system prompt (only used when preset is "custom").
    #[serde(default)]
    pub translation_custom_system_prompt: Option<String>,
    /// Custom prompt template with {text} and {target} placeholders (only used when preset is "custom").
    #[serde(default)]
    pub translation_custom_prompt_template: Option<String>,
}

fn default_ttl() -> u64 {
    21600
}

fn default_max_decode_secs() -> u64 {
    30
}

fn default_vad_threshold() -> f64 {
    0.5
}

fn default_silence_ms() -> i32 {
    3000
}

fn default_num_threads() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
        .clamp(1, 8)
}

fn default_translation_target() -> String {
    "en".to_string()
}

fn default_translation_ctx_size() -> i32 {
    2048
}

fn default_translation_temperature() -> f32 {
    0.3
}

fn default_translation_mode() -> String {
    "always".to_string()
}

fn default_translation_triggers() -> String {
    "翻译,翻訳,translate".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            active_model: None,
            vad_model: None,
            punctuation_model: None,
            denoiser_model: None,
            voxlock_voice_sample: None,
            github_token: None,
            release_cache_ttl_seconds: default_ttl(),
            hosts_enabled: false,
            hosts_source_url: String::new(),
            hosts: HashMap::new(),
            mirrors: Vec::new(),
            offline_end_whole_decode_max_seconds: default_max_decode_secs(),
            vad_threshold: default_vad_threshold(),
            silence_ms: default_silence_ms(),
            speaker_model: None,
            num_threads: default_num_threads(),
            config_version: CURRENT_CONFIG_VERSION,
            language: None,
            translation_enabled: false,
            translation_target: default_translation_target(),
            translation_model: None,
            translation_llama_server: None,
            translation_llama_lib_dir: None,
            translation_gpu_layers: 0,
            translation_ctx_size: default_translation_ctx_size(),
            translation_temperature: default_translation_temperature(),
            translation_extra_args: Vec::new(),
            translation_mode: default_translation_mode(),
            translation_triggers: default_translation_triggers(),
            translation_strip_hints: true,
            translation_prompt_preset: None,
            translation_custom_system_prompt: None,
            translation_custom_prompt_template: None,
        }
    }
}

/// Bump when defaults change and existing configs need migration.
const CURRENT_CONFIG_VERSION: u32 = 1;

impl AppConfig {
    pub fn load(store_dir: &Path) -> Result<Self> {
        let config_file = store_dir.join("models").join("config.json");
        if !config_file.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&config_file)
            .with_context(|| format!("read {}", crate::path_display::for_log_path(&config_file)))?;
        let mut cfg: AppConfig = serde_json::from_str(&text).with_context(|| {
            format!("parse {}", crate::path_display::for_log_path(&config_file))
        })?;
        cfg.resolve_paths(store_dir);
        // Apply migrations for outdated config files.
        if cfg.config_version < CURRENT_CONFIG_VERSION {
            cfg.migrate();
            // Persist migrated values so they survive restarts.
            let _ = cfg.save(store_dir);
        }
        Ok(cfg)
    }

    pub fn save(&self, store_dir: &Path) -> Result<()> {
        use std::io::Write;

        let models_dir = store_dir.join("models");
        std::fs::create_dir_all(&models_dir)?;
        let config_file = models_dir.join("config.json");
        let mut copy = self.clone();
        copy.config_version = CURRENT_CONFIG_VERSION;
        copy.relativize_paths(store_dir);
        let text = serde_json::to_string_pretty(&copy)?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&config_file)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Restrict existing files before truncating or writing secrets.
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.set_len(0)?;
        file.write_all(text.as_bytes())?;
        Ok(())
    }

    fn resolve_paths(&mut self, store_dir: &Path) {
        let models_dir = store_dir.join("models");
        for field in [
            &mut self.active_model,
            &mut self.vad_model,
            &mut self.punctuation_model,
            &mut self.denoiser_model,
            &mut self.voxlock_voice_sample,
            &mut self.translation_model,
            &mut self.translation_llama_server,
            &mut self.translation_llama_lib_dir,
        ] {
            if let Some(ref p) = field {
                let path = expand_tilde(p);
                let path_ref: &Path = path.as_ref();
                if path_ref.is_relative() {
                    let store_candidate = store_dir.join(path_ref);
                    let model_candidate = models_dir.join(path_ref);
                    let resolved = if store_candidate.exists() {
                        store_candidate
                    } else {
                        model_candidate
                    };
                    *field = Some(resolved.to_string_lossy().into_owned());
                } else {
                    *field = Some(path);
                }
            }
        }
    }

    /// Apply config migrations for outdated config files.
    /// Call after loading and before using the config.
    fn migrate(&mut self) {
        if self.config_version < 1 {
            // v0→v1: num_threads was hardcoded to 2; re-detect from CPU cores.
            self.num_threads = default_num_threads();
        }
        // Future migrations:
        // if self.config_version < 2 { ... }
    }

    fn relativize_paths(&mut self, store_dir: &Path) {
        let models_dir = store_dir.join("models");
        for field in [
            &mut self.active_model,
            &mut self.vad_model,
            &mut self.punctuation_model,
            &mut self.denoiser_model,
            &mut self.voxlock_voice_sample,
            &mut self.translation_model,
            &mut self.translation_llama_server,
            &mut self.translation_llama_lib_dir,
        ] {
            if let Some(ref p) = field {
                let path = PathBuf::from(p);
                if let Ok(rel) = path.strip_prefix(store_dir) {
                    *field = Some(rel.to_string_lossy().into_owned());
                } else if let Ok(rel) = path.strip_prefix(&models_dir) {
                    *field = Some(rel.to_string_lossy().into_owned());
                }
            }
        }
    }
}

/// Expand `~` and `~user` at the start of a path string.
/// Falls back to the original string if the home directory cannot be determined.
fn expand_tilde(path: &str) -> String {
    if !path.starts_with('~') {
        return path.to_string();
    }
    let (tilde_part, rest) = if let Some(slash_pos) = path.find('/') {
        (&path[..slash_pos], &path[slash_pos..])
    } else {
        (path, "")
    };
    let home = if tilde_part == "~" {
        dirs::home_dir()
    } else {
        // ~username — not supported on all platforms, fall back to original
        None
    };
    match home {
        Some(h) => format!("{}{}", h.to_string_lossy(), rest),
        None => path.to_string(),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn save_restricts_new_and_existing_config_without_changing_models_permissions() {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let store = std::env::temp_dir().join(format!(
            "sherpa-config-permissions-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&store).unwrap();
        let models = store.join("models");
        std::fs::create_dir(&models).unwrap();
        std::fs::set_permissions(&models, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = models.join("config.json");
        let cfg = AppConfig {
            github_token: Some("test-secret".into()),
            ..AppConfig::default()
        };

        for existing in [false, true] {
            if existing {
                // Also ensure saving removes any trailing bytes of the old file.
                std::fs::write(&path, "x".repeat(16384)).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            }
            cfg.save(&store).unwrap();
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(&models).unwrap().permissions().mode() & 0o777,
                0o755
            );
            assert_eq!(
                AppConfig::load(&store).unwrap().github_token,
                cfg.github_token
            );
        }
        std::fs::remove_dir_all(store).unwrap();
    }
}
