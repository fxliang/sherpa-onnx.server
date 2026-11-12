use std::path::{Path, PathBuf};

pub fn for_log_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if path == home {
            return "~".to_string();
        }
        if let Ok(rel) = path.strip_prefix(&home) {
            if rel.as_os_str().is_empty() {
                "~".to_string()
            } else {
                PathBuf::from("~").join(rel).display().to_string()
            }
        } else {
            path.display().to_string()
        }
    } else {
        path.display().to_string()
    }
}
