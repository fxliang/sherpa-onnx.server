use crate::config::AppConfig;

/// Rewrite URL using mirror rules from config.
/// Android/Kotlin uses $1 for capture groups; Rust regex uses $1 too, so direct compat.
pub fn rewrite_url(url: &str, cfg: &AppConfig) -> String {
    for mirror in &cfg.mirrors {
        if !mirror.enabled {
            continue;
        }
        if mirror.pattern.is_empty() || mirror.replacement.is_empty() {
            continue;
        }
        if let Ok(re) = regex::Regex::new(&mirror.pattern) {
            let new_url = re.replace(url, mirror.replacement.as_str());
            if new_url != url {
                return new_url.into_owned();
            }
        }
    }
    url.to_string()
}
