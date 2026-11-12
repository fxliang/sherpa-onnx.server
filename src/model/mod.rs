use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::AppConfig;

pub mod descriptor;
pub mod detect;
pub mod import;
pub mod metadata;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelFamily {
    OfflineParaformer,
    OfflineTransducer,
    OfflineWhisper,
    OfflineNemoCtc,
    OfflineNemoTransducer,
    OfflineTelespeech,
    OfflineSenseVoice,
    OfflineZipformerCtc,
    OfflineWenetCtc,
    OfflineFunasrNano,
    OfflineMoonshine,
    OfflineDolphin,
    OfflineFireRedAsr,
    OfflineFireRedAsrCtc,
    OfflineQwen3Asr,
    OfflineCanary,
    OfflineOmnilingual,
    OfflineMedAsr,
    OfflineCohereTranscribe,
    OfflineTdnn,
    OnlineTransducer,
    OnlineParaformer,
    OnlineZipformer2Ctc,
    OnlineNemoCtc,
    OnlineTOneCtc,
    Unsupported,
}

impl ModelFamily {
    pub fn streaming(&self) -> bool {
        matches!(
            self,
            ModelFamily::OnlineTransducer
                | ModelFamily::OnlineParaformer
                | ModelFamily::OnlineZipformer2Ctc
                | ModelFamily::OnlineNemoCtc
                | ModelFamily::OnlineTOneCtc
        )
    }
}

impl fmt::Display for ModelFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone)]
pub struct ModelFiles {
    pub model: Option<PathBuf>,
    pub ctc: Option<PathBuf>,
    pub encoder: Option<PathBuf>,
    pub decoder: Option<PathBuf>,
    pub joiner: Option<PathBuf>,
    pub conv_frontend: Option<PathBuf>,
    pub tokenizer_dir: Option<PathBuf>,
    pub llm: Option<PathBuf>,
    pub embedding: Option<PathBuf>,
    pub preprocessor: Option<PathBuf>,
    pub uncached_decoder: Option<PathBuf>,
    pub cached_decoder: Option<PathBuf>,
    pub merged_decoder: Option<PathBuf>,
    pub tokens: Option<PathBuf>,
    pub vad: Option<PathBuf>,
    pub punctuation: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub dir: PathBuf,
    pub name: String,
    pub family: ModelFamily,
    pub streaming: bool,
    pub files: ModelFiles,
    pub params: HashMap<String, String>,
}

impl fmt::Display for ModelInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mode = if self.streaming {
            "streaming"
        } else {
            "offline"
        };
        write!(f, "{} ({} {})", self.name, mode, self.family)
    }
}

pub fn models_dir(store_dir: &Path) -> PathBuf {
    store_dir.join("models")
}

pub fn find_files(model_dir: &Path) -> ModelFiles {
    let files: Vec<PathBuf> = walkdir(model_dir);

    fn lower_name(p: &Path) -> String {
        p.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    }

    fn find_onnx<F: Fn(&str) -> bool>(
        files: &[PathBuf],
        pred: F,
        prefer_int8: bool,
    ) -> Option<PathBuf> {
        let matches: Vec<&PathBuf> = files
            .iter()
            .filter(|p| {
                let n = lower_name(p);
                (n.ends_with(".onnx") || n.ends_with(".ort")) && pred(&n)
            })
            .collect();
        if matches.is_empty() {
            return None;
        }
        let int8 = matches.iter().find(|p| lower_name(p).contains(".int8."));
        let fp = matches.iter().find(|p| !lower_name(p).contains(".int8."));
        if prefer_int8 {
            int8.or(fp).map(|p| (*p).clone())
        } else {
            fp.or(int8).map(|p| (*p).clone())
        }
    }

    let tokens = files
        .iter()
        .find(|p| {
            let n = lower_name(p);
            n == "tokens.txt" || n.ends_with("-tokens.txt")
        })
        .cloned();

    let tokenizer_dir = find_tokenizer_dir(model_dir);

    ModelFiles {
        model: find_onnx(
            &files,
            |n| n == "model.onnx" || n == "model.int8.onnx",
            true,
        ),
        ctc: find_onnx(&files, |n| n.starts_with("ctc"), true),
        encoder: find_onnx(
            &files,
            |n| {
                (n.starts_with("encoder")
                    && !n.starts_with("encoder_adaptor")
                    && !n.starts_with("encoder_model"))
                    || n.ends_with("-encoder.onnx")
                    || n.ends_with("-encoder.int8.onnx")
                    || n.starts_with("encode")
                    || n.starts_with("encoder_model")
            },
            true,
        ),
        decoder: find_onnx(
            &files,
            |n| {
                (n.starts_with("decoder") && !n.starts_with("decoder_model_merged"))
                    || n.ends_with("-decoder.onnx")
                    || n.ends_with("-decoder.int8.onnx")
            },
            false,
        ),
        joiner: find_onnx(
            &files,
            |n| {
                n.starts_with("joiner")
                    || n.ends_with("-joiner.onnx")
                    || n.ends_with("-joiner.int8.onnx")
            },
            true,
        ),
        conv_frontend: find_onnx(&files, |n| n.starts_with("conv_frontend"), true),
        tokenizer_dir,
        llm: find_onnx(&files, |n| n.starts_with("llm"), true),
        embedding: find_onnx(&files, |n| n.starts_with("embedding"), true),
        preprocessor: find_onnx(&files, |n| n.starts_with("preprocess"), true),
        uncached_decoder: find_onnx(&files, |n| n.starts_with("uncached_decode"), true),
        cached_decoder: find_onnx(&files, |n| n.starts_with("cached_decode"), true),
        merged_decoder: find_onnx(
            &files,
            |n| n.starts_with("decoder_model_merged") || n.contains("decoder_model_merged"),
            true,
        ),
        tokens,
        vad: find_onnx(&files, |n| n.contains("silero") && n.contains("vad"), true),
        punctuation: find_onnx(&files, |n| n.contains("punct") || n.contains("punc"), true),
    }
}

fn find_tokenizer_dir(dir: &Path) -> Option<PathBuf> {
    fn visit(dir: &Path, depth: usize) -> Option<PathBuf> {
        if depth > 4 {
            return None;
        }
        let entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .collect();
        let names: Vec<String> = entries
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
            .collect();
        if names.iter().any(|n| n == "vocab.json")
            && names
                .iter()
                .any(|n| n == "tokenizer_config.json" || n == "merges.txt")
        {
            return Some(dir.to_path_buf());
        }
        for path in entries {
            if path.is_dir() {
                if let Some(found) = visit(&path, depth + 1) {
                    return Some(found);
                }
            }
        }
        None
    }

    visit(dir, 0)
}

fn walkdir(dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ty = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ty.is_symlink() {
                continue;
            }
            if ty.is_file() {
                result.push(path);
            } else if ty.is_dir() {
                result.extend(walkdir(&path));
            }
        }
    }
    result
}

/// Like `walkdir`, but keeps symlinked files so imported aliases (common for
/// large GGUF models) are still visible to auxiliary-model detection.
fn walk_entries(dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ty = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ty.is_dir() {
                result.extend(walk_entries(&path));
            } else {
                result.push(path);
            }
        }
    }
    result
}

pub fn list_model_dirs(store_dir: &Path) -> Vec<PathBuf> {
    let dir = models_dir(store_dir);
    if !dir.exists() {
        return Vec::new();
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            if !p.is_dir() {
                return false;
            }
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.is_empty() {
                return false;
            }
            if name.starts_with('.') {
                return false;
            }
            name != "punctuation"
        })
        .collect();
    dirs.sort();
    dirs
}

pub fn is_auxiliary_dir(model_dir: &Path) -> bool {
    let files = find_files(model_dir);
    let has_asr = files.model.is_some()
        || files.ctc.is_some()
        || files.encoder.is_some()
        || files.decoder.is_some()
        || files.joiner.is_some()
        || files.tokens.is_some();
    let has_aux = files.vad.is_some() || files.punctuation.is_some();
    has_aux && !has_asr
}

pub fn list_asr_models(store_dir: &Path) -> Vec<ModelInfo> {
    let cfg = AppConfig::load(store_dir).ok();
    list_model_dirs(store_dir)
        .into_iter()
        .filter(|d| !is_auxiliary_dir(d))
        .filter_map(|d| detect::detect_model(&d, cfg.as_ref()).ok())
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuxKind {
    Vad,
    Punctuation,
    Denoiser,
    Speaker,
    Translation,
}

impl AuxKind {
    fn sort_key(self) -> u8 {
        match self {
            AuxKind::Vad => 0,
            AuxKind::Punctuation => 1,
            AuxKind::Denoiser => 2,
            AuxKind::Speaker => 3,
            AuxKind::Translation => 4,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuxModel {
    pub dir: PathBuf,
    pub name: String,
    pub kind: AuxKind,
}

fn config_path_under(path: Option<&str>, dir: &Path) -> bool {
    path.map(|p| Path::new(p).starts_with(dir)).unwrap_or(false)
}

fn classify_aux_model(dir: &Path, cfg: Option<&AppConfig>) -> Option<AuxKind> {
    let files = walk_entries(dir);
    let names: Vec<String> = files
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .collect();
    let has_ext = |ext: &str| {
        files
            .iter()
            .any(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)))
    };

    if cfg.is_some_and(|c| config_path_under(c.translation_model.as_deref(), dir))
        || has_ext("gguf")
    {
        return Some(AuxKind::Translation);
    }
    if cfg.is_some_and(|c| config_path_under(c.denoiser_model.as_deref(), dir))
        || names
            .iter()
            .any(|n| n.contains("gtcrn") || n.contains("dpdfnet") || n.contains("voxlock"))
    {
        return Some(AuxKind::Denoiser);
    }
    if cfg.is_some_and(|c| config_path_under(c.speaker_model.as_deref(), dir))
        || names
            .iter()
            .any(|n| n.contains("3dspeaker") || n.contains("wespeaker") || n.contains("speakernet"))
    {
        return Some(AuxKind::Speaker);
    }
    if cfg.is_some_and(|c| config_path_under(c.vad_model.as_deref(), dir))
        || names
            .iter()
            .any(|n| n.contains("silero") || n.contains("-vad") || n.contains("_vad"))
    {
        return Some(AuxKind::Vad);
    }
    if cfg.is_some_and(|c| config_path_under(c.punctuation_model.as_deref(), dir))
        || names
            .iter()
            .any(|n| n.contains("punct") || n.contains("punc"))
    {
        return Some(AuxKind::Punctuation);
    }
    None
}

pub fn list_aux_models(store_dir: &Path) -> Vec<AuxModel> {
    let root = models_dir(store_dir);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let cfg = AppConfig::load(store_dir).ok();
    let mut models = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        if detect::detect_model(&dir, cfg.as_ref()).is_ok() {
            continue;
        }
        if let Some(kind) = classify_aux_model(&dir, cfg.as_ref()) {
            models.push(AuxModel { dir, name, kind });
        }
    }
    models.sort_by(|a, b| {
        a.kind
            .sort_key()
            .cmp(&b.kind.sort_key())
            .then_with(|| a.name.cmp(&b.name))
    });
    models
}

#[cfg(test)]
mod aux_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn lists_installed_non_asr_models_only() {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let store = std::env::temp_dir().join(format!(
            "sherpa-aux-list-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let models = models_dir(&store);
        std::fs::create_dir_all(&models).unwrap();

        let cases = [
            ("vad-model", "silero_vad.onnx"),
            ("denoiser-model", "gtcrn_simple.onnx"),
            ("speaker-model", "3dspeaker.onnx"),
            ("punctuation-model", "punctuation.onnx"),
            ("translation-model", "Hy-MT2-1.8B-Q4_K_M.gguf"),
        ];
        for (dir, file) in cases {
            let d = models.join(dir);
            std::fs::create_dir(&d).unwrap();
            std::fs::write(d.join(file), b"").unwrap();
        }

        let asr = models.join("asr-model");
        std::fs::create_dir(&asr).unwrap();
        std::fs::write(asr.join("model.onnx"), b"").unwrap();
        std::fs::write(asr.join("tokens.txt"), b"").unwrap();

        let found = list_aux_models(&store);
        let names: Vec<&str> = found.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(found.len(), 5, "unexpected models: {names:?}");
        assert!(!names.contains(&"asr-model"));
        for (kind, name) in [
            (AuxKind::Vad, "vad-model"),
            (AuxKind::Denoiser, "denoiser-model"),
            (AuxKind::Speaker, "speaker-model"),
            (AuxKind::Punctuation, "punctuation-model"),
            (AuxKind::Translation, "translation-model"),
        ] {
            assert!(
                found.iter().any(|m| m.kind == kind && m.name == name),
                "missing {kind:?} {name}"
            );
        }

        std::fs::remove_dir_all(store).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn lists_symlinked_translation_model() {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let store = std::env::temp_dir().join(format!(
            "sherpa-aux-symlink-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let models = models_dir(&store);
        std::fs::create_dir_all(&models).unwrap();

        let target = store.join("Hy-MT2-1.8B-Q4_K_M.gguf");
        std::fs::write(&target, b"").unwrap();
        let dir = models.join("Hy-MT2-1.8B-Q4_K_M.gguf");
        std::fs::create_dir(&dir).unwrap();
        std::os::unix::fs::symlink(&target, dir.join("Hy-MT2-1.8B-Q4_K_M.gguf")).unwrap();

        let found = list_aux_models(&store);
        assert_eq!(
            found.len(),
            1,
            "found: {:?}",
            found.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
        assert_eq!(found[0].kind, AuxKind::Translation);
        assert_eq!(found[0].name, "Hy-MT2-1.8B-Q4_K_M.gguf");

        std::fs::remove_dir_all(store).unwrap();
    }
}
