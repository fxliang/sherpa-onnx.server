pub mod denoiser;
pub mod punctuation;
pub mod recognizer;
pub mod speaker;
pub mod text;
pub mod vad;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use tokio::runtime::Handle;
use tracing::{info, warn};

use crate::config::AppConfig;
use crate::model::{ModelFamily, ModelInfo};

pub struct EngineConfig {
    pub recognizer: Option<sherpa_onnx::OfflineRecognizer>,
    pub online_recognizer: Option<sherpa_onnx::OnlineRecognizer>,
    pub vad: Option<sherpa_onnx::VoiceActivityDetector>,
    pub punctuation: Option<sherpa_onnx::OfflinePunctuation>,
    pub denoiser: Option<sherpa_onnx::OfflineSpeechDenoiser>,
    pub streaming: bool,
    pub sample_rate: i32,
    pub silence_ms: i32,
    pub vad_threshold: f64,
    pub interim_partial: bool,
    pub num_threads: i32,
    pub offline_end_whole_decode_max_seconds: u64,
    pub min_decode_samples: usize,
    pub model_info: Option<ModelInfo>,
    pub translation: Option<Arc<crate::translation::TranslationEngine>>,
    /// Absolute path to a speaker embedding model loaded from config.
    pub speaker_model: Option<String>,
    /// Speaker embedding extractor, loaded once with the model and shared
    /// across all sessions (the type is Send+Sync and its methods take &self,
    /// so concurrent sessions can each use their own stream against it). This
    /// avoids reloading the speaker model from disk on every session.
    pub speaker_extractor: Option<Arc<sherpa_onnx::SpeakerEmbeddingExtractor>>,
}

pub struct ServerRuntime {
    pub config: RwLock<EngineConfig>,
    /// Set to true when the server is stopping/restarting. Session threads
    /// check this flag and stop acquiring new read-locks on `config` so that
    /// `load_model` can acquire the write-lock without writer starvation.
    pub shutting_down: AtomicBool,
    /// Tokio runtime handle for spawning tasks from non-tokio threads
    /// (e.g. import handler, TUI callbacks on std::thread).
    handle: Handle,
}

impl ServerRuntime {
    pub fn new() -> Self {
        Self {
            shutting_down: AtomicBool::new(false),
            handle: Handle::current(),
            config: RwLock::new(EngineConfig {
                recognizer: None,
                online_recognizer: None,
                vad: None,
                punctuation: None,
                denoiser: None,
                streaming: false,
                sample_rate: 16000,
                silence_ms: 3000,
                vad_threshold: 0.5,
                interim_partial: true,
                num_threads: std::thread::available_parallelism()
                    .map(|n| n.get() as i32)
                    .unwrap_or(4)
                    .clamp(1, 8),
                offline_end_whole_decode_max_seconds: 30,
                min_decode_samples: 0,
                model_info: None,
                translation: None,
                speaker_model: None,
                speaker_extractor: None,
            }),
        }
    }

    pub fn load_model(&self, model_info: &ModelInfo, store_cfg: &AppConfig) -> Result<()> {
        crate::log::capture_native_stderr(|| self.load_model_inner(model_info, store_cfg))?;

        // Warmup translation engine AFTER capture_native_stderr returns.
        // Spawning child processes (llama-server) while stderr is redirected
        // on Windows can corrupt the CRT fd table and trigger
        // `_osfile(fh) & FOPEN` assertion failures.
        if let Ok(config) = self.config.read() {
            if let Some(engine) = config.translation.clone() {
                let handle = self.handle.clone();
                handle.spawn(async move {
                    engine.warmup().await;
                });
            }
        }
        Ok(())
    }

    fn load_model_inner(&self, model_info: &ModelInfo, store_cfg: &AppConfig) -> Result<()> {
        let fmt_path =
            |p: Option<&std::path::PathBuf>| p.map(|x| crate::path_display::for_log_path(x));
        info!(
            "Loading model: {} ({} families, streaming={})",
            model_info.name, model_info.family, model_info.streaming
        );
        info!(
            "  files: model={:?} ctc={:?} encoder={:?} decoder={:?} joiner={:?} tokens={:?}",
            fmt_path(model_info.files.model.as_ref()),
            fmt_path(model_info.files.ctc.as_ref()),
            fmt_path(model_info.files.encoder.as_ref()),
            fmt_path(model_info.files.decoder.as_ref()),
            fmt_path(model_info.files.joiner.as_ref()),
            fmt_path(model_info.files.tokens.as_ref()),
        );

        let sample_rate = model_info
            .params
            .get("sample_rate")
            .and_then(|s| s.parse().ok())
            .unwrap_or(16000);

        let mut config = self
            .config
            .write()
            .map_err(|e| anyhow::anyhow!("lock: {e}"))?;

        let mut effective_info = model_info.clone();

        if model_info.streaming {
            info!(
                "Creating online recognizer for family={}",
                model_info.family
            );
            let online = recognizer::create_online_recognizer(model_info, store_cfg)?;
            config.online_recognizer = Some(online);
            config.recognizer = None;
        } else {
            info!(
                "Creating offline recognizer for family={}",
                model_info.family
            );
            match recognizer::create_offline_recognizer(model_info, store_cfg) {
                Ok(offline) => {
                    config.recognizer = Some(offline);
                    config.online_recognizer = None;
                }
                Err(e) if model_info.family == ModelFamily::OfflineTransducer => {
                    let msg = e.to_string().to_lowercase();
                    let has_type_mismatch = (msg.contains("actual tensor(int64)")
                        && msg.contains("expected tensor(int32)"))
                        || (msg.contains("actual tensor(int32)")
                            && msg.contains("expected tensor(int64)"));
                    if !has_type_mismatch {
                        return Err(e);
                    }
                    let current = model_info
                        .params
                        .get("model_type_string")
                        .map(|s| s.as_str())
                        .unwrap_or("zipformer2");
                    let switched = if current == "zipformer2" {
                        "transducer"
                    } else {
                        "zipformer2"
                    };
                    warn!(
                        "Offline transducer dtype mismatch, switching model_type: {} -> {}",
                        current, switched
                    );
                    effective_info
                        .params
                        .insert("model_type_string".to_string(), switched.to_string());
                    let offline =
                        recognizer::create_offline_recognizer(&effective_info, store_cfg)?;
                    config.recognizer = Some(offline);
                    config.online_recognizer = None;
                }
                Err(e) => return Err(e),
            }
        }

        if !model_info.streaming {
            if let Some(ref recognizer) = config.recognizer {
                let cached = effective_info
                    .params
                    .get("min_decode_samples")
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|&v| v > 0);
                if let Some(v) = cached {
                    config.min_decode_samples = v;
                    info!("Using cached min_decode_samples: {v}");
                } else {
                    let probed = probe_min_decode_samples(recognizer, sample_rate);
                    config.min_decode_samples = probed;
                    if probed > 0 {
                        effective_info
                            .params
                            .insert("min_decode_samples".to_string(), probed.to_string());
                    }
                    info!(
                        "Probed min_decode_samples: {probed} ({:.2}s)",
                        probed as f32 / sample_rate as f32
                    );
                }
            }
        }

        config.vad = vad::create_vad(model_info, store_cfg, sample_rate);
        config.punctuation = punctuation::create_punctuation(&effective_info, store_cfg);
        config.denoiser = denoiser::create_denoiser(store_cfg);
        config.speaker_model = store_cfg.speaker_model.clone()
            .and_then(|p| resolve_model_file(&p));
        // Load the speaker embedding model once here so sessions can share it
        // instead of reloading it from disk on every session.
        config.speaker_extractor = config
            .speaker_model
            .as_deref()
            .and_then(speaker::SpeakerVerifier::load_extractor)
            .map(Arc::new);
        config.streaming = model_info.streaming;
        config.sample_rate = sample_rate;
        config.vad_threshold = store_cfg.vad_threshold;
        config.silence_ms = store_cfg.silence_ms;
        config.num_threads = store_cfg.num_threads;
        config.offline_end_whole_decode_max_seconds =
            store_cfg.offline_end_whole_decode_max_seconds;
        config.model_info = Some(effective_info.clone());
        let translation = crate::translation::TranslationEngine::from_app_config(store_cfg);
        config.translation = translation.is_enabled().then(|| Arc::new(translation));

        // Persist probed params (min_decode_samples, model_type_string, etc.)
        // back to model.json so they survive restarts.
        if effective_info.params != model_info.params {
            use crate::model::descriptor::ModelDescriptor;
            let existing = ModelDescriptor::read(&effective_info.dir);
            if let Some(mut desc) = existing {
                for (k, v) in &effective_info.params {
                    desc.params.insert(k.clone(), v.clone());
                }
                if let Err(e) = ModelDescriptor::write(&effective_info.dir, &desc) {
                    warn!("Failed to persist model params: {e}");
                } else {
                    info!("Persisted probed params to model.json");
                }
            }
        }

        info!(
            "Model loaded: {} (type={}, streaming={})",
            model_info.name, model_info.family, model_info.streaming
        );

        Ok(())
    }

    /// Rebuild the translation engine from the current [AppConfig].
    /// Replaces the existing `Arc<TranslationEngine>` (whose `Drop` impl kills
    /// the old llama-server child) and kicks off async warmup if enabled.
    /// Does not touch the ASR recognizer or any other runtime state.
    pub fn reload_translation(&self, store_cfg: &AppConfig) -> Result<()> {
        let translation = crate::translation::TranslationEngine::from_app_config(store_cfg);
        let next = translation.is_enabled().then(|| {
            let engine = Arc::new(translation);
            let warmup = engine.clone();
            let handle = self.handle.clone();
            handle.spawn(async move {
                warmup.warmup().await;
            });
            engine
        });
        let mut config = self
            .config
            .write()
            .map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let prev = config.translation.take();
        config.translation = next;
        drop(config);
        // Drop the old engine outside the write lock so its Drop (which may
        // block briefly killing llama-server) doesn't hold the runtime lock.
        drop(prev);
        info!("Translation engine reloaded from config");
        Ok(())
    }

    pub fn snapshot_streaming(&self) -> bool {
        self.config.read().map(|c| c.streaming).unwrap_or(false)
    }

    pub fn snapshot_sample_rate(&self) -> i32 {
        self.config.read().map(|c| c.sample_rate).unwrap_or(16000)
    }

    pub fn model_name(&self) -> String {
        self.config
            .read()
            .ok()
            .and_then(|c| c.model_info.as_ref().map(|m| m.name.clone()))
            .unwrap_or_else(|| "(none)".to_string())
    }
}

pub fn resolve_model_file(path: &str) -> Option<String> {
    let p = std::path::Path::new(path);
    if !p.exists() {
        return None;
    }
    if p.is_file() {
        return Some(path.to_string());
    }
    // Path is a directory (created by import when the archive contains a
    // single file). Find the first .onnx inside.
    if p.is_dir() {
        if let Ok(entries) = std::fs::read_dir(p) {
            for entry in entries.flatten() {
                let ep = entry.path();
                if ep.extension().and_then(|e| e.to_str()) == Some("onnx") {
                    return Some(ep.to_string_lossy().into_owned());
                }
            }
        }
    }
    None
}

fn probe_min_decode_samples(
    recognizer: &sherpa_onnx::OfflineRecognizer,
    sample_rate: i32,
) -> usize {
    let max_samples = (sample_rate as usize) * 5;

    let warmup_stream = recognizer.create_stream();
    warmup_stream.accept_waveform(sample_rate, &vec![0.0f32; max_samples]);
    recognizer.decode(&warmup_stream);
    drop(warmup_stream);

    // Cannot binary-search the real minimum from Rust: sherpa-onnx's C API
    // does not wrap DecodeStream in try-catch, so a too-short input throws a
    // C++ exception through FFI (undefined behavior / process abort).
    // Android catches it via JNI and probes to ~37200 for x-asr zipformer2.
    // 3 seconds at the model sample rate is a safe conservative lower bound.
    (sample_rate as usize) * 3
}
