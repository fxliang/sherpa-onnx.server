use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use tracing::{info, warn};

use crate::model::descriptor::ModelDescriptor;

pub fn safe_name(name: &str) -> String {
    let base = regex::Regex::new(r"(?i)\.(zip|tar\.gz|tgz|tar\.bz2|tar\.xz|gguf)$")
        .unwrap()
        .replace(name, "");
    let cleaned = regex::Regex::new(r"[^A-Za-z0-9._-]+")
        .unwrap()
        .replace_all(&base, "_");
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c == '_');
    if trimmed.is_empty() {
        "model".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn import_path(source: &Path, models_dir: &Path) -> Result<PathBuf> {
    if !source.exists() {
        bail!(
            "Source not found: {}",
            crate::path_display::for_log_path(source)
        );
    }
    fs::create_dir_all(models_dir)?;

    let mut dest = models_dir.join(safe_name(&source.file_name().unwrap().to_string_lossy()));
    if dest.exists() {
        let mut suffix = 2;
        loop {
            let candidate = models_dir.join(format!(
                "{}-{}",
                dest.file_name().unwrap().to_string_lossy(),
                suffix
            ));
            if !candidate.exists() {
                dest = candidate;
                break;
            }
            suffix += 1;
        }
    }

    let dest_name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "model".to_string());
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let staging = models_dir.join(format!(".{dest_name}.importing_{ts}"));
    fs::create_dir_all(&staging)?;

    let import_result = (|| -> Result<()> {
        if source.is_dir() {
            info!(
                "Import: copying directory {} -> {}",
                crate::path_display::for_log_path(source),
                crate::path_display::for_log_path(&staging)
            );
            copy_dir_recursive(source, &staging)?;
            flatten_single_top_dir(&staging)?;
        } else {
            let ext = source
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if ext == "onnx" || ext == "ort" || ext == "txt" || ext == "gguf" {
                info!(
                    "Import: copying file {}",
                    crate::path_display::for_log_path(source)
                );
                fs::copy(source, staging.join(source.file_name().unwrap()))?;
            } else {
                extract_archive(source, &staging)?;
            }
        }
        Ok(())
    })();

    if let Err(e) = import_result {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }

    if let Err(rename_err) = fs::rename(&staging, &dest) {
        copy_dir_recursive(&staging, &dest)?;
        fs::remove_dir_all(&staging).ok();
        if !dest.exists() {
            bail!(
                "Import finalize failed: rename error ({rename_err}) and copy fallback did not create destination"
            );
        }
    }

    // Cache model family/params once after import so UI listing/status refreshes
    // don't repeatedly parse large ONNX metadata.
    match crate::model::detect::detect_model_with_metadata(&dest, None) {
        Ok(info) => {
            let desc = ModelDescriptor {
                family: ModelDescriptor::family_to_string(info.family),
                files: Default::default(),
                params: info.params,
                descriptor_version: 1,
            };
            if let Err(e) = ModelDescriptor::write(&dest, &desc) {
                warn!(
                    "Failed to write model descriptor at {}: {e}",
                    crate::path_display::for_log_path(&dest)
                );
            }
        }
        Err(e) => {
            warn!(
                "Imported path {} is not a detectable ASR model yet: {e}",
                crate::path_display::for_log_path(&dest)
            );
        }
    }

    Ok(dest)
}

fn files_equal(a: &Path, b: &Path) -> bool {
    let (Ok(meta_a), Ok(meta_b)) = (fs::metadata(a), fs::metadata(b)) else {
        return false;
    };
    if meta_a.len() != meta_b.len() {
        return false;
    }
    let (Ok(mut file_a), Ok(mut file_b)) = (fs::File::open(a), fs::File::open(b)) else {
        return false;
    };
    let mut buf_a = [0u8; 64 * 1024];
    let mut buf_b = [0u8; 64 * 1024];
    loop {
        let read_a = match file_a.read(&mut buf_a) {
            Ok(n) => n,
            Err(_) => return false,
        };
        let read_b = match file_b.read(&mut buf_b) {
            Ok(n) => n,
            Err(_) => return false,
        };
        if read_a != read_b || buf_a[..read_a] != buf_b[..read_b] {
            return false;
        }
        if read_a == 0 {
            return true;
        }
    }
}

pub fn import_voice_sample(source: &Path, store_dir: &Path) -> Result<PathBuf> {
    if !source.is_file() {
        bail!(
            "Voice sample not found: {}",
            crate::path_display::for_log_path(source)
        );
    }
    if !source
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
    {
        bail!("Voice sample must be a .wav file");
    }

    let dir = store_dir.join("voice-samples");
    fs::create_dir_all(&dir)?;

    let file_name = safe_name(
        &source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "voice-sample.wav".to_string()),
    );
    let mut dest = dir.join(&file_name);
    if dest.exists() {
        if files_equal(source, &dest) {
            return Ok(dest);
        }
        let stem = Path::new(&file_name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "voice-sample".to_string());
        let ext = Path::new(&file_name)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let mut suffix = 2;
        loop {
            let candidate = dir.join(format!("{stem}-{suffix}{ext}"));
            if !candidate.exists() {
                dest = candidate;
                break;
            }
            suffix += 1;
        }
    }

    info!(
        "Import voice sample: {} -> {}",
        crate::path_display::for_log_path(source),
        crate::path_display::for_log_path(&dest)
    );
    fs::copy(source, &dest)?;
    Ok(dest)
}

pub fn extract_archive(archive: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    let name = archive
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_lowercase();

    info!(
        "Import: extracting {}...",
        crate::path_display::for_log_path(archive)
    );

    if name.ends_with(".zip") {
        extract_zip(archive, dest)?;
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let file = fs::File::open(archive)?;
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive.unpack(dest).context("extract tar.gz")?;
    } else if name.ends_with(".tar.bz2") {
        let file = fs::File::open(archive)?;
        let decoder = bzip2::read::BzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive.unpack(dest).context("extract tar.bz2")?;
    } else if name.ends_with(".tar.xz") {
        let file = fs::File::open(archive)?;
        let decoder = xz2::read::XzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive.unpack(dest).context("extract tar.xz")?;
    } else if name.ends_with(".tar") {
        let file = fs::File::open(archive)?;
        let mut archive = tar::Archive::new(file);
        archive.unpack(dest).context("extract tar")?;
    } else {
        bail!(
            "Unsupported archive format: {}",
            crate::path_display::for_log_path(archive)
        );
    }

    info!("Import: normalizing directory...");
    flatten_single_top_dir(dest)?;
    Ok(())
}

fn extract_zip(archive_path: &Path, dest: &Path) -> Result<()> {
    let file = fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let outpath = dest.join(entry.mangled_name());
        if entry.is_dir() {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile = fs::File::create(&outpath)?;
            std::io::copy(&mut entry, &mut outfile)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                fs::set_permissions(&outpath, fs::Permissions::from_mode(mode))?;
            }
        }
    }
    Ok(())
}

pub fn flatten_single_top_dir(path: &Path) -> Result<()> {
    let entries: Vec<PathBuf> = fs::read_dir(path)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.file_name().map(|n| n.to_str().unwrap_or("")) != Some("__MACOSX"))
        .collect();

    if entries.len() == 1 && entries[0].is_dir() {
        let inner = &entries[0];
        let tmp = path.parent().unwrap().join(".flatten_tmp");
        fs::create_dir_all(&tmp)?;
        let tmp_inner = tmp.join("inner");
        fs::rename(inner, &tmp_inner)?;

        for child in fs::read_dir(path)? {
            let child = child?.path();
            if child != tmp {
                if child.is_dir() {
                    fs::remove_dir_all(&child)?;
                } else {
                    fs::remove_file(&child)?;
                }
            }
        }

        for child in fs::read_dir(&tmp_inner)? {
            let child = child?.path();
            let dest = path.join(child.file_name().unwrap());
            fs::rename(&child, &dest)?;
        }

        fs::remove_dir_all(&tmp)?;
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let ty = entry.file_type()?;
        if ty.is_symlink() {
            continue;
        }
        if ty.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if ty.is_file() {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

pub fn delete_model_dir(model_dir: &Path) -> Result<()> {
    if model_dir.exists() && model_dir.is_dir() {
        fs::remove_dir_all(model_dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(prefix: &str) -> PathBuf {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn import_voice_sample_copies_and_deduplicates() {
        let store = temp_dir("sherpa-voice-store");
        let src_dir = temp_dir("sherpa-voice-src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let source = src_dir.join("reference.wav");
        std::fs::write(&source, b"wav-data").unwrap();

        let first = import_voice_sample(&source, &store).unwrap();
        assert_eq!(first, store.join("voice-samples").join("reference.wav"));
        assert_eq!(std::fs::read(&first).unwrap(), b"wav-data");

        let second = import_voice_sample(&source, &store).unwrap();
        assert_eq!(first, second);

        let other_dir = temp_dir("sherpa-voice-src-other");
        std::fs::create_dir_all(&other_dir).unwrap();
        let other = other_dir.join("reference.wav");
        std::fs::write(&other, b"other").unwrap();
        let third = import_voice_sample(&other, &store).unwrap();
        assert_eq!(third, store.join("voice-samples").join("reference-2.wav"));
        assert_eq!(std::fs::read(&third).unwrap(), b"other");

        let invalid = other_dir.join("reference.txt");
        std::fs::write(&invalid, b"nope").unwrap();
        assert!(import_voice_sample(&invalid, &store).is_err());

        std::fs::remove_dir_all(store).unwrap();
        std::fs::remove_dir_all(src_dir).unwrap();
        std::fs::remove_dir_all(other_dir).unwrap();
    }
}
