use std::path::Path;

use anyhow::{bail, Result};
use sherpa_onnx::{
    OfflineCanaryModelConfig, OfflineCohereTranscribeModelConfig, OfflineDolphinModelConfig,
    OfflineFireRedAsrCtcModelConfig, OfflineFireRedAsrModelConfig, OfflineFunASRNanoModelConfig,
    OfflineMedAsrCtcModelConfig, OfflineModelConfig, OfflineMoonshineModelConfig,
    OfflineNemoEncDecCtcModelConfig, OfflineOmnilingualAsrCtcModelConfig,
    OfflineParaformerModelConfig, OfflineQwen3ASRModelConfig, OfflineRecognizer,
    OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, OfflineTdnnModelConfig,
    OfflineTransducerModelConfig, OfflineWenetCtcModelConfig, OfflineWhisperModelConfig,
    OfflineZipformerCtcModelConfig, OnlineModelConfig, OnlineNemoCtcModelConfig,
    OnlineParaformerModelConfig, OnlineRecognizer, OnlineRecognizerConfig,
    OnlineToneCtcModelConfig, OnlineTransducerModelConfig, OnlineZipformer2CtcModelConfig,
};

use crate::config::AppConfig;
use crate::model::{ModelFamily, ModelInfo};

fn opt_str(s: &Option<impl AsRef<Path>>) -> Option<String> {
    s.as_ref()
        .map(|p| p.as_ref().to_string_lossy().into_owned())
}

fn looks_like_zipformer2_model(info: &ModelInfo) -> bool {
    let name_lower = info.name.to_lowercase();
    let encoder_name = info
        .files
        .encoder
        .as_ref()
        .map(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase()
        })
        .unwrap_or_default();
    name_lower.contains("zipformer")
        || name_lower.contains("x-asr")
        || encoder_name.contains("zipformer")
}

pub fn create_offline_recognizer(
    info: &ModelInfo,
    store_cfg: &AppConfig,
) -> Result<OfflineRecognizer> {
    let f = &info.files;
    let num_threads: i32 = info
        .params
        .get("num_threads")
        .and_then(|s| s.parse().ok())
        .unwrap_or(store_cfg.num_threads);
    let feature_dim: i32 = info
        .params
        .get("feature_dim")
        .and_then(|s| s.parse().ok())
        .unwrap_or(80);
    let tokens = opt_str(&f.tokens).unwrap_or_default();
    let provider = Some("cpu".to_string());

    let mut model_config = OfflineModelConfig {
        tokens: Some(tokens),
        num_threads,
        debug: false,
        provider: provider.clone(),
        ..Default::default()
    };

    match info.family {
        ModelFamily::OfflineSenseVoice => {
            let language = info
                .params
                .get("language")
                .cloned()
                .unwrap_or_else(|| "auto".to_string());
            model_config.sense_voice = OfflineSenseVoiceModelConfig {
                model: opt_str(&f.model).or_else(|| opt_str(&f.ctc)),
                language: Some(language),
                use_itn: false,
            };
        }
        ModelFamily::OfflineTransducer => {
            model_config.transducer = OfflineTransducerModelConfig {
                encoder: opt_str(&f.encoder),
                decoder: opt_str(&f.decoder),
                joiner: opt_str(&f.joiner),
            };
            let explicit = info.params.get("model_type_string").cloned();
            let looks_like_zipformer2 = looks_like_zipformer2_model(info);
            let model_type_str = match explicit.as_deref() {
                Some("transducer") if looks_like_zipformer2 => "zipformer2".to_string(),
                Some(v) => v.to_string(),
                None if looks_like_zipformer2 => "zipformer2".to_string(),
                None => "transducer".to_string(),
            };
            model_config.model_type = Some(model_type_str);
        }
        ModelFamily::OfflineParaformer => {
            model_config.paraformer = OfflineParaformerModelConfig {
                model: opt_str(&f.model),
            };
        }
        ModelFamily::OfflineWhisper => {
            let language = info
                .params
                .get("language")
                .cloned()
                .unwrap_or_else(|| "en".to_string());
            model_config.whisper = OfflineWhisperModelConfig {
                encoder: opt_str(&f.encoder),
                decoder: opt_str(&f.decoder),
                language: Some(language),
                ..Default::default()
            };
        }
        ModelFamily::OfflineNemoCtc => {
            model_config.nemo_ctc = OfflineNemoEncDecCtcModelConfig {
                model: opt_str(&f.model).or_else(|| opt_str(&f.ctc)),
                ..Default::default()
            };
        }
        ModelFamily::OfflineZipformerCtc => {
            model_config.zipformer_ctc = OfflineZipformerCtcModelConfig {
                model: opt_str(&f.model).or_else(|| opt_str(&f.ctc)),
                ..Default::default()
            };
        }
        ModelFamily::OfflineWenetCtc => {
            model_config.wenet_ctc = OfflineWenetCtcModelConfig {
                model: opt_str(&f.model).or_else(|| opt_str(&f.ctc)),
                ..Default::default()
            };
        }
        ModelFamily::OfflineNemoTransducer => {
            model_config.nemo_ctc = OfflineNemoEncDecCtcModelConfig {
                model: opt_str(&f.model).or_else(|| opt_str(&f.ctc)),
                ..Default::default()
            };
            model_config.model_type = Some("nemo_transducer".to_string());
        }
        ModelFamily::OfflineTelespeech => {
            model_config.telespeech_ctc = opt_str(&f.model).or_else(|| opt_str(&f.ctc));
            model_config.model_type = Some("telespeech_ctc".to_string());
        }
        ModelFamily::OfflineFunasrNano => {
            model_config.funasr_nano = OfflineFunASRNanoModelConfig {
                encoder_adaptor: opt_str(&f.encoder),
                llm: opt_str(&f.llm),
                embedding: opt_str(&f.embedding),
                tokenizer: f
                    .tokenizer_dir
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned()),
                ..Default::default()
            };
        }
        ModelFamily::OfflineMoonshine => {
            model_config.moonshine = OfflineMoonshineModelConfig {
                preprocessor: opt_str(&f.preprocessor),
                encoder: opt_str(&f.encoder),
                uncached_decoder: opt_str(&f.uncached_decoder),
                cached_decoder: opt_str(&f.cached_decoder),
                merged_decoder: opt_str(&f.merged_decoder),
            };
        }
        ModelFamily::OfflineDolphin => {
            model_config.dolphin = OfflineDolphinModelConfig {
                model: opt_str(&f.model).or_else(|| opt_str(&f.ctc)),
            };
        }
        ModelFamily::OfflineFireRedAsr => {
            model_config.fire_red_asr = OfflineFireRedAsrModelConfig {
                encoder: opt_str(&f.encoder),
                decoder: opt_str(&f.decoder),
            };
        }
        ModelFamily::OfflineFireRedAsrCtc => {
            model_config.fire_red_asr_ctc = OfflineFireRedAsrCtcModelConfig {
                model: opt_str(&f.model).or_else(|| opt_str(&f.ctc)),
            };
        }
        ModelFamily::OfflineQwen3Asr => {
            model_config.qwen3_asr = OfflineQwen3ASRModelConfig {
                conv_frontend: opt_str(&f.conv_frontend),
                encoder: opt_str(&f.encoder),
                decoder: opt_str(&f.decoder),
                tokenizer: f
                    .tokenizer_dir
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned()),
                ..Default::default()
            };
        }
        ModelFamily::OfflineCanary => {
            let language = info
                .params
                .get("language")
                .cloned()
                .unwrap_or_else(|| "en".to_string());
            model_config.canary = OfflineCanaryModelConfig {
                encoder: opt_str(&f.encoder),
                decoder: opt_str(&f.decoder),
                src_lang: Some(language.clone()),
                tgt_lang: Some(language),
                use_pnc: info
                    .params
                    .get("has_punctuation")
                    .map(|s| s == "true")
                    .unwrap_or(false),
            };
        }
        ModelFamily::OfflineOmnilingual => {
            model_config.omnilingual = OfflineOmnilingualAsrCtcModelConfig {
                model: opt_str(&f.model).or_else(|| opt_str(&f.ctc)),
            };
        }
        ModelFamily::OfflineMedAsr => {
            model_config.medasr = OfflineMedAsrCtcModelConfig {
                model: opt_str(&f.model).or_else(|| opt_str(&f.ctc)),
            };
        }
        ModelFamily::OfflineCohereTranscribe => {
            let language = info
                .params
                .get("language")
                .cloned()
                .unwrap_or_else(|| "en".to_string());
            model_config.cohere_transcribe = OfflineCohereTranscribeModelConfig {
                encoder: opt_str(&f.encoder),
                decoder: opt_str(&f.decoder),
                language: Some(language),
                use_punct: info
                    .params
                    .get("has_punctuation")
                    .map(|s| s == "true")
                    .unwrap_or(false),
                use_itn: false,
            };
        }
        ModelFamily::OfflineTdnn => {
            model_config.tdnn = OfflineTdnnModelConfig {
                model: opt_str(&f.model).or_else(|| opt_str(&f.ctc)),
            };
        }
        _ => bail!("Unsupported offline model family: {}", info.family),
    }

    let mut config = OfflineRecognizerConfig::default();
    config.model_config = model_config;
    config.feat_config.feature_dim = feature_dim;

    OfflineRecognizer::create(&config)
        .ok_or_else(|| anyhow::anyhow!("Failed to create offline recognizer"))
}

pub fn create_online_recognizer(
    info: &ModelInfo,
    store_cfg: &AppConfig,
) -> Result<OnlineRecognizer> {
    let f = &info.files;
    let num_threads: i32 = info
        .params
        .get("num_threads")
        .and_then(|s| s.parse().ok())
        .unwrap_or(store_cfg.num_threads);
    let feature_dim: i32 = info
        .params
        .get("feature_dim")
        .and_then(|s| s.parse().ok())
        .unwrap_or(80);
    let tokens = opt_str(&f.tokens).unwrap_or_default();
    let provider = Some("cpu".to_string());

    let mut model_config = OnlineModelConfig {
        tokens: Some(tokens),
        num_threads,
        provider,
        ..Default::default()
    };

    match info.family {
        ModelFamily::OnlineTransducer => {
            model_config.transducer = OnlineTransducerModelConfig {
                encoder: opt_str(&f.encoder),
                decoder: opt_str(&f.decoder),
                joiner: opt_str(&f.joiner),
            };
            // Android always passes empty modelType for online recognizer,
            // letting sherpa-onnx auto-detect from the ONNX model metadata.
            model_config.model_type = Some(String::new());
        }
        ModelFamily::OnlineParaformer => {
            model_config.paraformer = OnlineParaformerModelConfig {
                encoder: opt_str(&f.encoder),
                decoder: opt_str(&f.decoder),
            };
        }
        ModelFamily::OnlineZipformer2Ctc => {
            model_config.zipformer2_ctc = OnlineZipformer2CtcModelConfig {
                model: opt_str(&f.model).or_else(|| opt_str(&f.ctc)),
            };
        }
        ModelFamily::OnlineNemoCtc => {
            model_config.nemo_ctc = OnlineNemoCtcModelConfig {
                model: opt_str(&f.model).or_else(|| opt_str(&f.ctc)),
            };
        }
        ModelFamily::OnlineTOneCtc => {
            model_config.t_one_ctc = OnlineToneCtcModelConfig {
                model: opt_str(&f.model).or_else(|| opt_str(&f.ctc)),
            };
        }
        _ => bail!("Unsupported online model family: {}", info.family),
    }

    let mut config = OnlineRecognizerConfig::default();
    config.model_config = model_config;
    config.feat_config.feature_dim = feature_dim;
    config.enable_endpoint = true;
    config.rule1_min_trailing_silence = 2.4;
    config.rule2_min_trailing_silence = 1.5;
    config.rule3_min_utterance_length = 20.0;

    OnlineRecognizer::create(&config)
        .ok_or_else(|| anyhow::anyhow!("Failed to create online recognizer"))
}
