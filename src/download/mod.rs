pub mod client;
pub mod hosts;
pub mod mirror;

use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;

const GITHUB_RELEASE_TAGS: &[(&str, &str)] = &[
    ("asr", "asr-models"),
    ("speaker", "speaker-recongition-models"),
    ("denoiser", "speech-enhancement-models"),
    ("punctuation", "punctuation-models"),
    ("translation", "translation-models"),
];

fn github_release_tag_api(tag: &str) -> String {
    format!("https://api.github.com/repos/k2-fsa/sherpa-onnx/releases/tags/{tag}")
}

/// Hardcoded GGUF translation models from ModelScope.
const GGUF_MODELS: &[(&str, &str)] = &[
    (
        "Hy-MT2-1.8B-1.25Bit.gguf",
        "https://www.modelscope.cn/models/AngelSlim/Hy-MT2-1.8B-1.25Bit-GGUF/resolve/master/Hy-MT2-1.8B-1.25Bit.gguf",
    ),
    (
        "Hy-MT2-1.8B-Q4_K_M.gguf",
        "https://www.modelscope.cn/models/Tencent-Hunyuan/Hy-MT2-1.8B-GGUF/resolve/master/Hy-MT2-1.8B-Q4_K_M.gguf",
    ),
];

fn build_translation_assets() -> Vec<Asset> {
    GGUF_MODELS
        .iter()
        .map(|(name, url)| Asset {
            name: name.to_string(),
            url: url.to_string(),
            size: 0, // ModelScope doesn't provide size in URL
            release: "modelscope".to_string(),
            kind: "translation".to_string(),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub release: String,
    pub kind: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReleaseCache {
    fetched_at: u64,
    assets: Vec<Asset>,
}

pub async fn fetch_release_assets(
    store_dir: &Path,
    cfg: &AppConfig,
    http: &client::HttpClient,
    force_refresh: bool,
) -> Result<Vec<Asset>> {
    let cache_file = store_dir.join("models").join("release-assets-cache.json");
    let ttl = cfg.release_cache_ttl_seconds;

    if !force_refresh && cache_file.exists() && ttl > 0 {
        if let Ok(text) = std::fs::read_to_string(&cache_file) {
            if let Ok(cached) = serde_json::from_str::<ReleaseCache>(&text) {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if now.saturating_sub(cached.fetched_at) < ttl {
                    return Ok(deduplicate_assets(cached.assets));
                }
            }
        }
    }

    let mut assets = Vec::new();
    for &(kind, tag) in GITHUB_RELEASE_TAGS {
        let fetch_result = if kind == "translation" {
            // GGUF models are hardcoded ModelScope URLs, no GitHub API call needed.
            Ok(build_translation_assets())
        } else {
            fetch_release_tag_assets(http, cfg, tag, kind).await
        };
        match fetch_result {
            Ok(mut a) => assets.append(&mut a),
            Err(e) => {
                if kind == "asr" {
                    return Err(e);
                }
                tracing::warn!("failed to fetch release tag {tag}: {e}");
            }
        }
    }

    assets = deduplicate_assets(assets);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let cache = ReleaseCache {
        fetched_at: now,
        assets: assets.clone(),
    };
    std::fs::create_dir_all(store_dir.join("models"))?;
    std::fs::write(&cache_file, serde_json::to_string(&cache)?)?;

    Ok(assets)
}

async fn fetch_release_tag_assets(
    http: &client::HttpClient,
    cfg: &AppConfig,
    tag: &str,
    kind: &str,
) -> Result<Vec<Asset>> {
    let url = github_release_tag_api(tag);
    let resp = http.get_json(&url, cfg, false).await?;
    let release: serde_json::Value = resp;

    let mut assets = Vec::new();
    if let Some(arr) = release.get("assets").and_then(|v| v.as_array()) {
        for item in arr {
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let detected_kind = if kind == "asr" && is_vad_asset_name(name) {
                "vad"
            } else {
                kind
            };
            if !is_downloadable_asset(detected_kind, name) {
                continue;
            }
            let url = item
                .get("browser_download_url")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let size = item.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            let release_tag = release
                .get("tag_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            assets.push(Asset {
                name: name.to_string(),
                url: url.to_string(),
                size,
                release: release_tag.to_string(),
                kind: detected_kind.to_string(),
            });
        }
    }
    Ok(assets)
}

fn is_vad_asset_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("sherpa-onnx-vad-") || lower.contains("silero_vad")
}

fn is_hardware_specific_asset(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("ascend")
        || lower.contains(".rknn")
        || regex::Regex::new(r"(?:^|[-_])rk\d{3,}")
            .unwrap()
            .is_match(&lower)
}

fn is_downloadable_asset(kind: &str, name: &str) -> bool {
    let lower = name.to_lowercase();
    if cfg!(any(target_arch = "x86", target_arch = "x86_64")) && is_hardware_specific_asset(name) {
        return false;
    }
    if kind == "translation" {
        return lower.ends_with(".gguf");
    }
    if kind == "punctuation" {
        return lower.ends_with(".tar.bz2")
            || lower.ends_with(".tar.gz")
            || lower.ends_with(".tgz")
            || lower.ends_with(".zip")
            || lower.ends_with(".onnx")
            || lower.ends_with(".ort");
    }
    if kind == "vad" {
        if !(lower.ends_with(".tar.bz2")
            || lower.ends_with(".tar.gz")
            || lower.ends_with(".tgz")
            || lower.ends_with(".zip")
            || lower.ends_with(".onnx")
            || lower.ends_with(".ort"))
        {
            return false;
        }
        return is_vad_asset_name(name);
    }
    if kind == "denoiser" {
        return lower.ends_with(".tar.bz2")
            || lower.ends_with(".tar.gz")
            || lower.ends_with(".tgz")
            || lower.ends_with(".zip")
            || lower.ends_with(".onnx")
            || lower.ends_with(".ort");
    }
    if kind != "asr" {
        return true;
    }
    if !(lower.ends_with(".tar.bz2")
        || lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".zip"))
    {
        return false;
    }
    let excluded = [
        "vits-",
        "kokoro-",
        "matcha-",
        "piper-",
        "sherpa-onnx-kws-",
        "3dspeaker-",
        "wespeaker-",
        "nemo-speakernet-",
        "sherpa-onnx-pyannote-",
        "sherpa-onnx-reverb-",
        "mms-lid-",
        "sherpa-onnx-language-id-",
        "sherpa-onnx-online-punct-",
        "sherpa-onnx-punct-",
        "sherpa-onnx-zipformer-audio-tagging-",
        "silero_vad",
        "sherpa-onnx-vad-",
        "sherpa-onnx-online-punctuation-",
    ];
    if excluded.iter().any(|t| lower.contains(t)) {
        return false;
    }
    if lower.contains("tts-") && !lower.contains("asr") {
        return false;
    }
    if lower.starts_with("hr-") {
        return false;
    }
    let markers = [
        "paraformer",
        "zipformer",
        "sense-voice",
        "sensevoice",
        "whisper",
        "moonshine",
        "dolphin",
        "qwen3-asr",
        "fire-red-asr",
        "telespeech",
        "wenetspeech",
        "nemo-ctc",
        "nemo-fast-conformer",
        "nemo-parakeet",
        "nemo-transducer",
        "canary",
        "omnilingual",
        "medasr",
        "funasr",
        "icefall-asr",
        "asr",
    ];
    markers.iter().any(|m| lower.contains(m))
}

fn deduplicate_assets(assets: Vec<Asset>) -> Vec<Asset> {
    let mut seen: HashMap<String, Asset> = HashMap::new();
    for asset in assets {
        let key = dedup_key(&asset);
        let dominated = seen
            .get(&key)
            .map(|e| punctuation_priority(e) > punctuation_priority(&asset))
            .unwrap_or(false);
        if !seen.contains_key(&key) || dominated {
            seen.insert(key, asset);
        }
    }
    seen.into_values().collect()
}

fn dedup_key(asset: &Asset) -> String {
    if asset.kind == "punctuation" {
        let base = regex::Regex::new(r"(?i)\.(tar\.bz2|tar\.gz|tgz|zip|onnx|ort)$")
            .unwrap()
            .replace(&asset.name, "");
        format!("punctuation:{}", base.to_lowercase())
    } else {
        format!("{}:{}", asset.kind, asset.url)
    }
}

fn punctuation_priority(asset: &Asset) -> u8 {
    let lower = asset.name.to_lowercase();
    if lower.ends_with(".onnx") || lower.ends_with(".ort") {
        0
    } else if lower.ends_with(".tar.bz2") {
        1
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        2
    } else if lower.ends_with(".zip") {
        3
    } else {
        4
    }
}

pub const CATEGORIES: &[(&str, &str)] = &[
    ("all", "All assets"),
    ("recommended", "Curated quick picks"),
    ("sense-voice", "SenseVoice — multilingual offline"),
    ("paraformer", "Paraformer / FunASR"),
    ("zipformer", "Zipformer (streaming + offline)"),
    ("whisper", "Whisper variants"),
    ("qwen3", "Qwen3-ASR"),
    ("moonshine", "Moonshine"),
    ("nemo", "NVIDIA NeMo"),
    ("streaming", "Any streaming/online ASR"),
    ("asr", "All ASR archives"),
    ("other", "Other ASR families"),
    ("vad", "VAD (silero)"),
    ("denoiser", "Speech enhancement"),
    ("punctuation", "Punctuation models"),
    ("translation", "Translation models (.gguf)"),
    ("speaker", "Speaker embedding (.onnx)"),
];

const ASR_FAMILY_MARKERS: &[&str] = &[
    "paraformer",
    "funasr",
    "zipformer",
    "sense-voice",
    "sensevoice",
    "whisper",
    "moonshine",
    "qwen3-asr",
    "nemo-",
    "dolphin",
    "fire-red-asr",
    "telespeech",
    "wenetspeech",
    "canary",
    "omnilingual",
    "medasr",
    "icefall-asr",
];

pub fn filter_assets_by_category<'a>(assets: &'a [Asset], category: &str) -> Vec<&'a Asset> {
    if category == "all" {
        return assets.iter().collect();
    }
    let needles: &[&str] = match category {
        "recommended" => &[
            "sense-voice",
            "sensevoice",
            "paraformer",
            "zipformer-streaming",
        ],
        "sense-voice" => &["sense-voice", "sensevoice"],
        "paraformer" => &["paraformer", "funasr"],
        "zipformer" => &["zipformer"],
        "whisper" => &["whisper"],
        "qwen3" => &["qwen3-asr"],
        "moonshine" => &["moonshine"],
        "nemo" => &["nemo-"],
        "streaming" => &["streaming", "online"],
        "vad" => &["vad", "silero"],
        "punctuation" => &["punct", "punc"],
        "speaker" => &[
            "speaker",
            "3dspeaker",
            "wespeaker",
            "speakernet",
            "nemo-speakernet",
        ],
        _ => &[],
    };
    assets
        .iter()
        .filter(|a| {
            if category == "punctuation" {
                return a.kind == "punctuation";
            }
            if category == "vad" {
                return a.kind == "vad";
            }
            if category == "denoiser" {
                return a.kind == "denoiser";
            }
            if category == "translation" {
                return a.kind == "translation";
            }
            if category == "speaker" {
                return a.kind == "speaker";
            }
            if category == "asr" {
                return a.kind == "asr";
            }
            if a.kind != "asr" {
                return false;
            }
            let lower = a.name.to_lowercase();
            if category == "other" {
                return !ASR_FAMILY_MARKERS.iter().any(|n| lower.contains(n));
            }
            needles.iter().any(|n| lower.contains(n))
        })
        .collect()
}

pub fn clear_release_cache(store_dir: &Path) -> Result<()> {
    let cache_file = store_dir.join("models").join("release-assets-cache.json");
    if cache_file.exists() {
        std::fs::remove_file(&cache_file)?;
    }
    Ok(())
}

pub fn format_bytes(value: u64) -> String {
    let mut size = value as f64;
    let units = ["B", "KB", "MB", "GB"];
    for unit in &units {
        if size < 1024.0 || *unit == "GB" {
            return if *unit == "B" {
                format!("{:.0}B", size)
            } else {
                format!("{:.1}{}", size, unit)
            };
        }
        size /= 1024.0;
    }
    format!("{:.1}GB", size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str, kind: &str) -> Asset {
        Asset {
            name: name.to_string(),
            url: String::new(),
            size: 0,
            release: String::new(),
            kind: kind.to_string(),
        }
    }

    #[test]
    fn all_category_keeps_every_asset() {
        let assets = vec![asset("a.tar.bz2", "asr"), asset("b.onnx", "vad")];
        assert_eq!(filter_assets_by_category(&assets, "all").len(), 2);
    }

    #[test]
    fn family_categories_are_populated() {
        let assets = vec![
            asset("sherpa-onnx-qwen3-asr-0.6B.tar.bz2", "asr"),
            asset("sherpa-onnx-dolphin-base.tar.bz2", "asr"),
            asset("sherpa-onnx-brand-new-family.tar.bz2", "asr"),
        ];
        assert_eq!(filter_assets_by_category(&assets, "qwen3").len(), 1);
        assert_eq!(filter_assets_by_category(&assets, "other").len(), 1);
        assert_eq!(
            filter_assets_by_category(&assets, "other")[0].name,
            "sherpa-onnx-brand-new-family.tar.bz2"
        );
    }

    #[test]
    fn translation_category_keeps_both_hunyuan_models() {
        assert!(CATEGORIES.iter().any(|(key, _)| *key == "translation"));
        let assets = build_translation_assets();
        assert_eq!(assets.len(), 2);
        let filtered = filter_assets_by_category(&assets, "translation");
        assert_eq!(filtered.len(), 2);
        assert!(filtered
            .iter()
            .all(|a| a.name.starts_with("Hy-MT2-1.8B-") && a.name.ends_with(".gguf")));
    }

    #[test]
    fn hardware_specific_assets_are_filtered_on_x86() {
        if !cfg!(any(target_arch = "x86", target_arch = "x86_64")) {
            return;
        }
        assert!(!is_downloadable_asset(
            "asr",
            "sherpa-onnx-ascend-310P3-sense-voice-2024-07-17.tar.bz2"
        ));
        assert!(!is_downloadable_asset(
            "asr",
            "sherpa-onnx-rk3588-5-seconds-sense-voice-2024-07-17.tar.bz2"
        ));
        assert!(is_downloadable_asset(
            "asr",
            "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17.tar.bz2"
        ));
    }
}
