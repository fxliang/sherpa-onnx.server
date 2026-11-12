use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use super::descriptor::ModelDescriptor;
use super::metadata;
use super::{find_files, ModelFamily, ModelInfo};
use crate::config::AppConfig;
use crate::path_display::for_log_path;

pub fn detect_model(model_dir: &Path, store_cfg: Option<&AppConfig>) -> Result<ModelInfo> {
    detect_model_internal(model_dir, store_cfg, false)
}

pub fn detect_model_with_metadata(
    model_dir: &Path,
    store_cfg: Option<&AppConfig>,
) -> Result<ModelInfo> {
    detect_model_internal(model_dir, store_cfg, true)
}

fn detect_model_internal(
    model_dir: &Path,
    store_cfg: Option<&AppConfig>,
    with_metadata: bool,
) -> Result<ModelInfo> {
    if let Some(desc) = ModelDescriptor::read(model_dir) {
        return from_descriptor(model_dir, &desc, store_cfg);
    }
    from_file_layout(model_dir, store_cfg, with_metadata)
}

fn from_descriptor(
    model_dir: &Path,
    desc: &ModelDescriptor,
    _store_cfg: Option<&AppConfig>,
) -> Result<ModelInfo> {
    let family = desc.family();
    if family == ModelFamily::Unsupported {
        bail!("model.json family {} is not supported", desc.family);
    }
    let files = find_files(model_dir);
    let mut params = desc.params.clone();

    if params.get("metadata_extracted").map(|s| s.as_str()) != Some("1") {
        let onnx_files: Vec<PathBuf> = [
            files.model.as_ref(),
            files.ctc.as_ref(),
            files.encoder.as_ref(),
            files.decoder.as_ref(),
            files.joiner.as_ref(),
            files.preprocessor.as_ref(),
            files.conv_frontend.as_ref(),
            files.uncached_decoder.as_ref(),
            files.cached_decoder.as_ref(),
            files.merged_decoder.as_ref(),
            files.llm.as_ref(),
            files.embedding.as_ref(),
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect();

        let meta = metadata::read_onnx_metadata(&onnx_files);
        let meta_params = metadata::to_descriptor_params(&meta);

        if !meta_params.contains_key("has_punctuation") && !params.contains_key("has_punctuation") {
            let inferred = files
                .tokens
                .as_ref()
                .and_then(|t| infer_has_punctuation_from_tokens(t))
                .unwrap_or_else(|| {
                    let name_lower = model_dir
                        .file_name()
                        .map(|n| n.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    infer_has_punctuation_from_name(&name_lower)
                });
            params.insert("has_punctuation".to_string(), inferred.to_string());
        }

        params.extend(meta_params);
        params.insert("metadata_extracted".to_string(), "1".to_string());

        let updated = ModelDescriptor {
            family: desc.family.clone(),
            files: desc.files.clone(),
            params: params.clone(),
            descriptor_version: desc.descriptor_version,
        };
        let _ = ModelDescriptor::write(model_dir, &updated);
    }

    Ok(ModelInfo {
        dir: model_dir.to_path_buf(),
        name: model_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        family,
        streaming: family.streaming(),
        files,
        params,
    })
}

fn from_file_layout(
    model_dir: &Path,
    _store_cfg: Option<&AppConfig>,
    with_metadata: bool,
) -> Result<ModelInfo> {
    let name_lower = model_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let f = find_files(model_dir);
    let streaming_hint = name_lower.contains("streaming") || name_lower.contains("online");
    let has_ctc_or_model = f.ctc.is_some() || f.model.is_some();

    let (family, streaming) = if streaming_hint
        && f.encoder.is_some()
        && f.decoder.is_some()
        && f.joiner.is_some()
        && f.tokens.is_some()
    {
        (ModelFamily::OnlineTransducer, true)
    } else if streaming_hint
        && name_lower.contains("paraformer")
        && f.encoder.is_some()
        && f.decoder.is_some()
        && f.tokens.is_some()
    {
        (ModelFamily::OnlineParaformer, true)
    } else if streaming_hint
        && name_lower.contains("nemo")
        && has_ctc_or_model
        && f.tokens.is_some()
    {
        (ModelFamily::OnlineNemoCtc, true)
    } else if streaming_hint
        && (name_lower.contains("t_one")
            || name_lower.contains("t-one")
            || name_lower.contains("tone"))
        && has_ctc_or_model
        && f.tokens.is_some()
    {
        (ModelFamily::OnlineTOneCtc, true)
    } else if streaming_hint && has_ctc_or_model && f.tokens.is_some() {
        (ModelFamily::OnlineZipformer2Ctc, true)
    } else if f.llm.is_some() && f.embedding.is_some() && f.tokenizer_dir.is_some() {
        (ModelFamily::OfflineFunasrNano, false)
    } else if f.conv_frontend.is_some()
        && f.encoder.is_some()
        && f.decoder.is_some()
        && f.tokenizer_dir.is_some()
    {
        (ModelFamily::OfflineQwen3Asr, false)
    } else if f.preprocessor.is_some()
        && f.encoder.is_some()
        && (f.uncached_decoder.is_some() || f.cached_decoder.is_some())
    {
        (ModelFamily::OfflineMoonshine, false)
    } else if name_lower.contains("moonshine") && f.merged_decoder.is_some() {
        (ModelFamily::OfflineMoonshine, false)
    } else if name_lower.contains("whisper")
        && f.encoder.is_some()
        && f.decoder.is_some()
        && f.tokens.is_some()
    {
        (ModelFamily::OfflineWhisper, false)
    } else if name_lower.contains("canary")
        && f.encoder.is_some()
        && f.decoder.is_some()
        && f.tokens.is_some()
    {
        (ModelFamily::OfflineCanary, false)
    } else if name_lower.contains("cohere")
        && f.encoder.is_some()
        && f.decoder.is_some()
        && f.tokens.is_some()
    {
        (ModelFamily::OfflineCohereTranscribe, false)
    } else if name_lower.contains("fire_red")
        && f.encoder.is_some()
        && f.decoder.is_some()
        && f.joiner.is_none()
        && f.tokens.is_some()
    {
        (ModelFamily::OfflineFireRedAsr, false)
    } else if name_lower.contains("nemo")
        && f.encoder.is_some()
        && f.decoder.is_some()
        && f.joiner.is_some()
        && f.tokens.is_some()
    {
        (ModelFamily::OfflineNemoTransducer, false)
    } else if f.encoder.is_some() && f.decoder.is_some() && f.joiner.is_some() && f.tokens.is_some()
    {
        (ModelFamily::OfflineTransducer, false)
    } else if name_lower.contains("paraformer") && f.model.is_some() && f.tokens.is_some() {
        (ModelFamily::OfflineParaformer, false)
    } else if name_lower.contains("fire_red") && has_ctc_or_model && f.tokens.is_some() {
        (ModelFamily::OfflineFireRedAsrCtc, false)
    } else if name_lower.contains("dolphin") && has_ctc_or_model && f.tokens.is_some() {
        (ModelFamily::OfflineDolphin, false)
    } else if name_lower.contains("nemo") && has_ctc_or_model && f.tokens.is_some() {
        (ModelFamily::OfflineNemoCtc, false)
    } else if name_lower.contains("wenet") && has_ctc_or_model && f.tokens.is_some() {
        (ModelFamily::OfflineWenetCtc, false)
    } else if name_lower.contains("telespeech") && has_ctc_or_model && f.tokens.is_some() {
        (ModelFamily::OfflineTelespeech, false)
    } else if name_lower.contains("omnilingual") && has_ctc_or_model && f.tokens.is_some() {
        (ModelFamily::OfflineOmnilingual, false)
    } else if name_lower.contains("medasr") && has_ctc_or_model && f.tokens.is_some() {
        (ModelFamily::OfflineMedAsr, false)
    } else if name_lower.contains("tdnn") && has_ctc_or_model && f.tokens.is_some() {
        (ModelFamily::OfflineTdnn, false)
    } else if (name_lower.contains("zipformer") || name_lower.contains("ctc"))
        && has_ctc_or_model
        && f.tokens.is_some()
    {
        (ModelFamily::OfflineZipformerCtc, false)
    } else if name_lower.contains("sense_voice") || name_lower.contains("sensevoice") {
        if f.model.is_some() && f.tokens.is_some() {
            (ModelFamily::OfflineSenseVoice, false)
        } else {
            bail!(
                "Cannot auto-detect model type under {}",
                for_log_path(model_dir)
            );
        }
    } else if f.model.is_some() && f.tokens.is_some() {
        (ModelFamily::OfflineSenseVoice, false)
    } else {
        bail!(
            "Cannot auto-detect model type under {}",
            for_log_path(model_dir)
        );
    };

    let mut params = HashMap::new();
    match family {
        ModelFamily::OfflineSenseVoice => {
            params.insert("language".to_string(), "auto".to_string());
        }
        ModelFamily::OfflineQwen3Asr | ModelFamily::OfflineFunasrNano => {
            params.insert("num_threads".to_string(), "3".to_string());
        }
        _ => {}
    }

    let looks_like_zipformer2 = name_lower.contains("zipformer") || name_lower.contains("x-asr");
    match family {
        ModelFamily::OnlineTransducer => {
            let mt = if looks_like_zipformer2 {
                "zipformer2"
            } else {
                "transducer"
            };
            params.insert("model_type_string".to_string(), mt.to_string());
        }
        ModelFamily::OfflineNemoTransducer => {
            params.insert(
                "model_type_string".to_string(),
                "nemo_transducer".to_string(),
            );
        }
        ModelFamily::OfflineTelespeech => {
            params.insert(
                "model_type_string".to_string(),
                "telespeech_ctc".to_string(),
            );
        }
        _ => {}
    }

    if with_metadata {
        let onnx_files: Vec<PathBuf> = [
            f.model.as_ref(),
            f.ctc.as_ref(),
            f.encoder.as_ref(),
            f.decoder.as_ref(),
            f.joiner.as_ref(),
            f.preprocessor.as_ref(),
            f.conv_frontend.as_ref(),
            f.uncached_decoder.as_ref(),
            f.cached_decoder.as_ref(),
            f.merged_decoder.as_ref(),
            f.llm.as_ref(),
            f.embedding.as_ref(),
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect();

        let meta = metadata::read_onnx_metadata(&onnx_files);
        let meta_params = metadata::to_descriptor_params(&meta);
        params.extend(meta_params);
    }

    if !params.contains_key("has_punctuation") {
        let inferred = f
            .tokens
            .as_ref()
            .and_then(|t| infer_has_punctuation_from_tokens(t))
            .unwrap_or_else(|| infer_has_punctuation_from_name(&name_lower));
        params.insert("has_punctuation".to_string(), inferred.to_string());
    }

    if with_metadata {
        params.insert("metadata_extracted".to_string(), "1".to_string());
    }

    Ok(ModelInfo {
        dir: model_dir.to_path_buf(),
        name: model_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        family,
        streaming,
        files: f,
        params,
    })
}

fn infer_has_punctuation_from_tokens(tokens_path: &Path) -> Option<bool> {
    let content = std::fs::read_to_string(tokens_path).ok()?;
    let punct_markers: &[&str] = &[
        "44CC", // 。
        "77yM", // ，
        "77yB", // ！
        "77yf", // ？
        "44CB", // 、
        "77yb", // ；
        "77ya", // ：
        "44CM", // 「
        "44CN", // 」
        "44CK", // 《
        "44CL", // 》
        "77yI", // （
        "77yJ", // ）
    ];
    for line in content.lines() {
        if let Some(token) = line.split_whitespace().next() {
            if punct_markers.contains(&token) {
                return Some(true);
            }
        }
    }
    None
}

fn infer_has_punctuation_from_name(name_lower: &str) -> bool {
    let normalized = name_lower.replace('_', "-");
    [
        "-punct-",
        "-punc-",
        "-punctuation-",
        "-with-punct-",
        "-with-punctuation-",
    ]
    .iter()
    .any(|pat| normalized.contains(pat))
}
