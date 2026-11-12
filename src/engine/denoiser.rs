use sherpa_onnx::{
    OfflineSpeechDenoiser, OfflineSpeechDenoiserConfig, OfflineSpeechDenoiserDpdfNetModelConfig,
    OfflineSpeechDenoiserGtcrnModelConfig, OfflineSpeechDenoiserModelConfig,
    OfflineSpeechDenoiserVoxLockModelConfig,
};
use std::path::{Path, PathBuf};
use tracing::info;
use tracing::warn;

use crate::config::AppConfig;
use crate::engine::ServerRuntime;

pub fn process_audio(runtime: &ServerRuntime, samples: &[f32], sample_rate: i32) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let denoised = runtime
        .config
        .read()
        .ok()
        .and_then(|config| {
            config
                .denoiser
                .as_ref()
                .map(|denoiser| denoiser.run(samples, sample_rate).samples)
        })
        .filter(|output| !output.is_empty());
    denoised.unwrap_or_else(|| samples.to_vec())
}

pub fn create_denoiser(store_cfg: &AppConfig) -> Option<OfflineSpeechDenoiser> {
    let configured_path = store_cfg
        .denoiser_model
        .as_deref()
        .map(PathBuf::from)?;
    let files = model_files(&configured_path);
    let chunk = files.iter().find(|p| {
        let name = file_name(p);
        name.contains("chunk") && is_onnx(p)
    });
    let enroll = files.iter().find(|p| {
        let name = file_name(p);
        name.contains("enroll") && is_onnx(p)
    });

    let config = if let Some(chunk) = chunk {
        let Some(enroll) = enroll else {
            warn!(
                "VoxLock denoiser is incomplete: no enroll model found near {}",
                crate::path_display::for_log_path(&configured_path)
            );
            return None;
        };
        let Some(voice_sample) = store_cfg
            .voxlock_voice_sample
            .as_deref()
            .map(PathBuf::from)
            .filter(|p| valid_voice_sample(p))
        else {
            warn!("VoxLock denoiser disabled: --voxlock-voice-sample must point to a valid 16 kHz WAV with at least 3 seconds of audio");
            return None;
        };
        info!(
            "Creating VoxLock denoiser from chunk={} enroll={}",
            crate::path_display::for_log_path(chunk),
            crate::path_display::for_log_path(enroll)
        );
        OfflineSpeechDenoiserConfig {
            model: OfflineSpeechDenoiserModelConfig {
                dpdfnet: Default::default(),
                gtcrn: Default::default(),
                voxlock: OfflineSpeechDenoiserVoxLockModelConfig {
                    enroll_model: Some(enroll.to_string_lossy().into_owned()),
                    chunk_model: Some(chunk.to_string_lossy().into_owned()),
                    voice_sample: Some(voice_sample.to_string_lossy().into_owned()),
                },
                num_threads: 2,
                debug: false,
                provider: Some("cpu".to_string()),
            },
        }
    } else {
        let denoiser_path = files.iter().find(|p| is_onnx(p))?;
        let name = file_name(denoiser_path);
        info!("Creating {} denoiser from: {denoiser_path:?}", if name.starts_with("dpdfnet") { "DPDFNet" } else { "GTCRN" });
        let (dpdfnet, gtcrn) = if name.starts_with("dpdfnet") {
            (
                OfflineSpeechDenoiserDpdfNetModelConfig {
                    model: Some(denoiser_path.to_string_lossy().into_owned()),
                    attenuation_limit_db: 0.0,
                },
                Default::default(),
            )
        } else {
            (
                Default::default(),
                OfflineSpeechDenoiserGtcrnModelConfig {
                    model: Some(denoiser_path.to_string_lossy().into_owned()),
                },
            )
        };
        OfflineSpeechDenoiserConfig {
            model: OfflineSpeechDenoiserModelConfig {
                dpdfnet,
                gtcrn,
                voxlock: Default::default(),
                num_threads: 2,
                debug: false,
                provider: Some("cpu".to_string()),
            },
        }
    };

    OfflineSpeechDenoiser::create(&config)
}

fn is_onnx(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("onnx") || ext.eq_ignore_ascii_case("ort"))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn model_files(path: &Path) -> Vec<PathBuf> {
    if path.is_file() {
        if file_name(path).contains("voxlock") {
            let mut files = Vec::new();
            if let Some(parent) = path.parent() {
                collect_files(parent, &mut files);
            }
            return files;
        }
        return vec![path.to_path_buf()];
    }
    let mut files = Vec::new();
    collect_files(path, &mut files);
    files
}

fn valid_voice_sample(path: &Path) -> bool {
    let Ok(data) = std::fs::read(path) else {
        return false;
    };
    if data.len() < 44 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return false;
    }
    let channels = u16::from_le_bytes([data[22], data[23]]);
    let sample_rate = u32::from_le_bytes([data[24], data[25], data[26], data[27]]);
    let bits = u16::from_le_bytes([data[34], data[35]]);
    let data_size = data.windows(4).position(|w| w == b"data")
        .and_then(|offset| data.get(offset + 4..offset + 8))
        .map(|size| u32::from_le_bytes([size[0], size[1], size[2], size[3]]) as usize)
        .unwrap_or(0);
    channels > 0 && sample_rate == 16000 && bits == 16
        && data_size >= sample_rate as usize * channels as usize * 2 * 3
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            files.push(path);
        }
    }
}
