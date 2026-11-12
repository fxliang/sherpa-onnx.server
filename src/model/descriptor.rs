use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::ModelFamily;

pub const FILE_NAME: &str = "model.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDescriptor {
    pub family: String,
    #[serde(default)]
    pub files: HashMap<String, String>,
    #[serde(default)]
    pub params: HashMap<String, String>,
    #[serde(default = "default_version")]
    #[serde(rename = "descriptorVersion")]
    pub descriptor_version: i32,
}

fn default_version() -> i32 {
    1
}

impl ModelDescriptor {
    pub fn read(model_dir: &Path) -> Option<Self> {
        let path = model_dir.join(FILE_NAME);
        if !path.exists() {
            return None;
        }
        let text = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn write(model_dir: &Path, desc: &Self) -> Result<()> {
        let path = model_dir.join(FILE_NAME);
        let text = serde_json::to_string_pretty(desc)?;
        std::fs::write(&path, text)?;
        Ok(())
    }

    pub fn family(&self) -> ModelFamily {
        match self.family.as_str() {
            "OFFLINE_SENSE_VOICE" => ModelFamily::OfflineSenseVoice,
            "OFFLINE_PARAFORMER" => ModelFamily::OfflineParaformer,
            "OFFLINE_TRANSDUCER" => ModelFamily::OfflineTransducer,
            "OFFLINE_WHISPER" => ModelFamily::OfflineWhisper,
            "OFFLINE_NEMO_CTC" => ModelFamily::OfflineNemoCtc,
            "OFFLINE_NEMO_TRANSDUCER" => ModelFamily::OfflineNemoTransducer,
            "OFFLINE_TELESPEECH" => ModelFamily::OfflineTelespeech,
            "OFFLINE_ZIPFORMER_CTC" => ModelFamily::OfflineZipformerCtc,
            "OFFLINE_WENET_CTC" => ModelFamily::OfflineWenetCtc,
            "OFFLINE_FUNASR_NANO" => ModelFamily::OfflineFunasrNano,
            "OFFLINE_MOONSHINE" => ModelFamily::OfflineMoonshine,
            "OFFLINE_DOLPHIN" => ModelFamily::OfflineDolphin,
            "OFFLINE_FIRE_RED_ASR" => ModelFamily::OfflineFireRedAsr,
            "OFFLINE_FIRE_RED_ASR_CTC" => ModelFamily::OfflineFireRedAsrCtc,
            "OFFLINE_QWEN3_ASR" => ModelFamily::OfflineQwen3Asr,
            "OFFLINE_CANARY" => ModelFamily::OfflineCanary,
            "OFFLINE_OMNILINGUAL" => ModelFamily::OfflineOmnilingual,
            "OFFLINE_MEDASR" => ModelFamily::OfflineMedAsr,
            "OFFLINE_COHERE_TRANSCRIBE" => ModelFamily::OfflineCohereTranscribe,
            "OFFLINE_TDNN" => ModelFamily::OfflineTdnn,
            "ONLINE_TRANSDUCER" => ModelFamily::OnlineTransducer,
            "ONLINE_PARAFORMER" => ModelFamily::OnlineParaformer,
            "ONLINE_ZIPFORMER2_CTC" => ModelFamily::OnlineZipformer2Ctc,
            "ONLINE_NEMO_CTC" => ModelFamily::OnlineNemoCtc,
            "ONLINE_TONE_CTC" => ModelFamily::OnlineTOneCtc,
            _ => ModelFamily::Unsupported,
        }
    }

    pub fn family_to_string(family: ModelFamily) -> String {
        match family {
            ModelFamily::OfflineSenseVoice => "OFFLINE_SENSE_VOICE",
            ModelFamily::OfflineParaformer => "OFFLINE_PARAFORMER",
            ModelFamily::OfflineTransducer => "OFFLINE_TRANSDUCER",
            ModelFamily::OfflineWhisper => "OFFLINE_WHISPER",
            ModelFamily::OfflineNemoCtc => "OFFLINE_NEMO_CTC",
            ModelFamily::OfflineNemoTransducer => "OFFLINE_NEMO_TRANSDUCER",
            ModelFamily::OfflineTelespeech => "OFFLINE_TELESPEECH",
            ModelFamily::OfflineZipformerCtc => "OFFLINE_ZIPFORMER_CTC",
            ModelFamily::OfflineWenetCtc => "OFFLINE_WENET_CTC",
            ModelFamily::OfflineFunasrNano => "OFFLINE_FUNASR_NANO",
            ModelFamily::OfflineMoonshine => "OFFLINE_MOONSHINE",
            ModelFamily::OfflineDolphin => "OFFLINE_DOLPHIN",
            ModelFamily::OfflineFireRedAsr => "OFFLINE_FIRE_RED_ASR",
            ModelFamily::OfflineFireRedAsrCtc => "OFFLINE_FIRE_RED_ASR_CTC",
            ModelFamily::OfflineQwen3Asr => "OFFLINE_QWEN3_ASR",
            ModelFamily::OfflineCanary => "OFFLINE_CANARY",
            ModelFamily::OfflineOmnilingual => "OFFLINE_OMNILINGUAL",
            ModelFamily::OfflineMedAsr => "OFFLINE_MEDASR",
            ModelFamily::OfflineCohereTranscribe => "OFFLINE_COHERE_TRANSCRIBE",
            ModelFamily::OfflineTdnn => "OFFLINE_TDNN",
            ModelFamily::OnlineTransducer => "ONLINE_TRANSDUCER",
            ModelFamily::OnlineParaformer => "ONLINE_PARAFORMER",
            ModelFamily::OnlineZipformer2Ctc => "ONLINE_ZIPFORMER2_CTC",
            ModelFamily::OnlineNemoCtc => "ONLINE_NEMO_CTC",
            ModelFamily::OnlineTOneCtc => "ONLINE_TONE_CTC",
            ModelFamily::Unsupported => "UNSUPPORTED",
        }
        .to_string()
    }
}
