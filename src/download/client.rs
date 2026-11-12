use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use tracing::info;

use super::mirror;
use crate::config::AppConfig;

#[derive(Clone)]
pub struct HttpClient {
    inner: reqwest::Client,
}

impl HttpClient {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            // Keep reqwest's redirect limit and cross-host credential stripping,
            // but never carry credentials through a TLS downgrade on the same port.
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.url().scheme() != "https"
                    && attempt
                        .previous()
                        .last()
                        .is_some_and(|url| url.scheme() == "https")
                {
                    attempt.stop()
                } else {
                    reqwest::redirect::Policy::default().redirect(attempt)
                }
            }))
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_secs(1800)) // 30 min for large models
            .pool_max_idle_per_host(1)
            .build()?;
        Ok(Self { inner: client })
    }

    pub async fn get_json(
        &self,
        url: &str,
        cfg: &AppConfig,
        apply_mirror: bool,
    ) -> Result<serde_json::Value> {
        let final_url = if apply_mirror {
            mirror::rewrite_url(url, cfg)
        } else {
            url.to_string()
        };

        let headers = auth_headers(cfg, &final_url);

        let resp = self
            .inner
            .get(&final_url)
            .headers(headers)
            .send()
            .await
            .with_context(|| format!("GET {final_url}"))?;
        if !resp.status().is_success() {
            anyhow::bail!("HTTP {} for {}", resp.status(), final_url);
        }
        let body = resp
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("read body: {e}"))?;
        let value: serde_json::Value = serde_json::from_str(&body)?;
        Ok(value)
    }

    pub async fn download_file(
        &self,
        url: &str,
        dest: &std::path::Path,
        cfg: &AppConfig,
        on_progress: impl Fn(u64, u64),
    ) -> Result<()> {
        use futures_util::StreamExt;
        use std::io::Write;

        // Only apply mirror rewriting to GitHub URLs; ModelScope etc. are direct.
        let final_url = if is_github_destination(url) {
            mirror::rewrite_url(url, cfg)
        } else {
            url.to_string()
        };
        let headers = auth_headers(cfg, &final_url);

        if final_url != url {
            info!("Using rewritten URL: {final_url}");
        }

        let resp = self
            .inner
            .get(&final_url)
            .headers(headers)
            .send()
            .await
            .context("download request failed")?;

        if !resp.status().is_success() {
            anyhow::bail!("HTTP {} for {}", resp.status(), final_url);
        }

        let total = resp.content_length().unwrap_or(0);
        let mut done: u64 = 0;
        let mut stream = resp.bytes_stream();

        let mut file = std::fs::File::create(dest)?;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("read chunk")?;
            file.write_all(&chunk)?;
            done += chunk.len() as u64;
            on_progress(done, total);
        }

        Ok(())
    }

    pub async fn get_text(&self, url: &str, cfg: &AppConfig) -> Result<String> {
        let headers = auth_headers(cfg, url);
        let resp = self
            .inner
            .get(url)
            .headers(headers)
            .send()
            .await
            .context("HTTP GET failed")?;
        if !resp.status().is_success() {
            anyhow::bail!("HTTP {} for {}", resp.status(), url);
        }
        Ok(resp.text().await?)
    }
}

fn is_github_destination(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|url| {
        url.scheme() == "https"
            && url.port_or_known_default() == Some(443)
            && matches!(url.host_str(), Some("api.github.com" | "github.com"))
    })
}

fn auth_headers(cfg: &AppConfig, url: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static("sherpa-onnx-server"));
    if let Some(token) = cfg
        .github_token
        .as_ref()
        .filter(|_| is_github_destination(url))
    {
        if !token.is_empty() {
            if let Ok(mut val) = HeaderValue::from_str(&format!("Bearer {token}")) {
                val.set_sensitive(true);
                headers.insert(AUTHORIZATION, val);
            }
        }
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MirrorRule;

    #[test]
    fn token_only_goes_to_https_github_hosts() {
        let cfg = AppConfig {
            github_token: Some("test-token".into()),
            ..AppConfig::default()
        };
        for url in [
            "https://api.github.com/repos/k2-fsa/sherpa-onnx/releases",
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/model.tar.gz",
            "https://API.GITHUB.COM:443/repos/example",
        ] {
            let headers = auth_headers(&cfg, url);
            assert_eq!(headers[AUTHORIZATION], "Bearer test-token", "{url}");
            assert!(headers[AUTHORIZATION].is_sensitive());
        }
        for url in [
            "http://api.github.com/repos/example",
            "http://github.com:443/example",
            "https://github.com:8443/example",
            "https://api.github.com:8443/example",
            "https://github.com.evil.example/model",
            "https://api.github.com.evil.example/model",
            "https://evilgithub.com/model",
            "https://github.com@evil.example/model",
            "https://evil.example/github.com/model",
            "https://evil.example/?url=https://api.github.com",
            "https://github.com./model",
            "https://uploads.github.com/model",
            "https://modelscope.cn/models/example",
            "https://mirror.example/https://github.com/model",
            "https://custom.example/model",
            "not a URL",
        ] {
            let headers = auth_headers(&cfg, url);
            assert!(!headers.contains_key(AUTHORIZATION), "{url}");
            assert!(headers.contains_key(USER_AGENT), "{url}");
        }
    }

    #[test]
    fn token_uses_destination_after_mirror_rewrite() {
        let cfg = AppConfig {
            github_token: Some("test-token".into()),
            mirrors: vec![MirrorRule {
                pattern: "^https://github\\.com/".into(),
                replacement: "https://mirror.example/".into(),
                enabled: true,
            }],
            ..AppConfig::default()
        };
        let url = mirror::rewrite_url("https://github.com/example/model", &cfg);
        assert_eq!(url, "https://mirror.example/example/model");
        assert!(!auth_headers(&cfg, &url).contains_key(AUTHORIZATION));
    }

    #[test]
    fn missing_empty_or_invalid_tokens_are_not_sent() {
        for token in [None, Some(String::new()), Some("invalid\ntoken".into())] {
            let cfg = AppConfig {
                github_token: token,
                ..AppConfig::default()
            };
            assert!(!auth_headers(&cfg, "https://api.github.com/").contains_key(AUTHORIZATION));
        }
    }
}

pub async fn update_hosts_from_source(cfg: &mut AppConfig, http: &HttpClient) -> Result<usize> {
    let url = cfg.hosts_source_url.clone();
    if url.is_empty() {
        anyhow::bail!("hosts_source_url is empty");
    }
    let text = http.get_text(&url, cfg).await?;
    let hosts = super::hosts::parse_hosts_text(&text);
    let count = hosts.len();
    cfg.hosts = hosts;
    Ok(count)
}
