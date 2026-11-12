use std::path::{Path, PathBuf};

use sherpa_onnx::{OfflinePunctuation, OfflinePunctuationConfig, OfflinePunctuationModelConfig};
use tracing::{info, warn};

use crate::config::AppConfig;
use crate::model::{find_files, ModelInfo};

fn resolve_punctuation_model(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    if path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("onnx") || e.eq_ignore_ascii_case("ort"))
    {
        return Some(path.to_path_buf());
    }
    if path.is_dir() {
        let files = find_files(path);
        if let Some(p) = files.punctuation {
            return Some(p);
        }
        return files.model;
    }
    None
}

pub fn create_punctuation(
    model_info: &ModelInfo,
    store_cfg: &AppConfig,
) -> Option<OfflinePunctuation> {
    if model_info
        .params
        .get("has_punctuation")
        .is_some_and(|v| v == "true")
    {
        info!("Model has built-in punctuation, skipping external punctuation model");
        return None;
    }

    let punct_path = store_cfg.punctuation_model.as_ref().cloned().or_else(|| {
        model_info
            .files
            .punctuation
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
    });

    let punct_path = punct_path?;
    if punct_path.is_empty() {
        return None;
    }
    let punct_path_log = crate::path_display::for_log_path(Path::new(&punct_path));

    let Some(punct_model) = resolve_punctuation_model(&punct_path) else {
        warn!("Punctuation model not found under: {punct_path_log}");
        return None;
    };
    let punct_model_log = crate::path_display::for_log_path(&punct_model);

    let punct_model = punct_model.to_string_lossy().into_owned();
    info!("Creating punctuation from: {punct_model_log}");

    let config = OfflinePunctuationConfig {
        model: OfflinePunctuationModelConfig {
            ct_transformer: Some(punct_model.clone()),
            num_threads: 1,
            debug: false,
            provider: Some("cpu".to_string()),
        },
    };

    let punctuation = OfflinePunctuation::create(&config);
    if punctuation.is_none() {
        warn!("Failed to create punctuation from: {punct_model_log}");
    }
    punctuation
}
