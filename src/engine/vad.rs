use std::path::{Path, PathBuf};

use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use tracing::{info, warn};

use crate::config::AppConfig;
use crate::model::{find_files, ModelInfo};

fn resolve_vad_model(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    if path.is_dir() {
        let files = find_files(path);
        return files.vad.or_else(|| files.model).or_else(|| files.ctc);
    }
    None
}

pub fn create_vad(
    model_info: &ModelInfo,
    store_cfg: &AppConfig,
    sample_rate: i32,
) -> Option<VoiceActivityDetector> {
    let vad_path = store_cfg.vad_model.as_ref().cloned().or_else(|| {
        model_info
            .files
            .vad
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
    });

    let vad_path = vad_path?;
    if vad_path.is_empty() {
        return None;
    }
    let vad_path_log = crate::path_display::for_log_path(Path::new(&vad_path));

    let Some(vad_model) = resolve_vad_model(&vad_path) else {
        warn!("VAD model not found under: {vad_path_log}");
        return None;
    };
    let vad_model_log = crate::path_display::for_log_path(&vad_model);
    let vad_model = vad_model.to_string_lossy().into_owned();

    info!(
        "Creating VAD from: {vad_path_log} (resolved: {vad_model_log}), threshold={}, silence_ms={}",
        store_cfg.vad_threshold,
        store_cfg.silence_ms
    );

    let config = VadModelConfig {
        silero_vad: SileroVadModelConfig {
            model: Some(vad_model),
            threshold: store_cfg.vad_threshold as f32,
            min_silence_duration: store_cfg.silence_ms as f32 / 1000.0,
            min_speech_duration: 0.25,
            window_size: 512,
            max_speech_duration: 30.0,
        },
        sample_rate,
        num_threads: 1,
        provider: Some("cpu".to_string()),
        debug: false,
        ..Default::default()
    };

    VoiceActivityDetector::create(&config, 30.0)
}
