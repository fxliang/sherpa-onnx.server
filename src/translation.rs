//! ASR and translation content is logged only with explicit `--log-level debug`
//! (or trace). These levels can expose private speech, prompts, and responses.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration as TokioDuration};
use tracing::{info, warn};

use crate::config::AppConfig;

const DEFAULT_MIN_TRANSLATION_TOKENS: i32 = 96;
const DEFAULT_MAX_TRANSLATION_TOKENS: i32 = 256;
const LOW_BIT_MIN_TRANSLATION_TOKENS: i32 = 64;
const LOW_BIT_MAX_TRANSLATION_TOKENS: i32 = 192;
const SERVER_STARTUP_TIMEOUT_SECS: u64 = 120;
const HEALTH_POLL_INTERVAL_MS: u64 = 500;

#[derive(Clone, Debug)]
pub struct TranslationConfig {
    pub enabled: bool,
    pub mode: TranslationMode,
    pub triggers: Vec<String>,
    pub strip_hints: bool,
    pub target: TranslationLanguage,
    pub model: Option<PathBuf>,
    pub llama_server: PathBuf,
    pub llama_lib_dir: Option<PathBuf>,
    pub gpu_layers: i32,
    pub ctx_size: i32,
    pub temperature: f32,
    pub extra_args: Vec<String>,
    pub num_threads: i32,
    pub prompt_preset: PromptPreset,
    pub custom_system_prompt: Option<String>,
    pub custom_prompt_template: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptPreset {
    Auto,
    #[serde(rename = "hy_mt2")]
    HyMt2,
    Qwen,
    Generic,
    Custom,
}

impl PromptPreset {
    pub fn from_str_lossy(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "auto" => Self::Auto,
            "hy_mt2" => Self::HyMt2,
            "qwen" => Self::Qwen,
            "generic" => Self::Generic,
            "custom" => Self::Custom,
            _ => Self::Auto,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ModelPromptProfile {
    pub system_prompt: String,
    pub prompt_template: String,
}

impl ModelPromptProfile {
    pub fn render(&self, text: &str, target: TranslationLanguage) -> (String, String) {
        let user_prompt = self
            .prompt_template
            .replace("{target}", target.prompt_name)
            .replace("{text}", text);
        (self.system_prompt.clone(), user_prompt)
    }
}

fn preset_profiles() -> Vec<(PromptPreset, ModelPromptProfile)> {
    vec![
        (PromptPreset::HyMt2, ModelPromptProfile {
            system_prompt: String::new(),
            prompt_template: "将以下文本翻译成{target}：\n{text}".to_string(),
        }),
        (PromptPreset::Qwen, ModelPromptProfile {
            system_prompt: "/no_think".to_string(),
            prompt_template: "将下面的内容翻译成{target}，只输出翻译结果，不带任何分析、解释和思考过程：\n{text}".to_string(),
        }),
        (PromptPreset::Generic, ModelPromptProfile {
            system_prompt: String::new(),
            prompt_template: "将下面的内容翻译成{target}，只输出翻译结果，不带任何分析、解释和思考过程：\n{text}".to_string(),
        }),
    ]
}

fn resolve_preset_for_model(model: Option<&Path>) -> PromptPreset {
    let file_name = model
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_lowercase();
    if file_name.contains("hy-mt") {
        PromptPreset::HyMt2
    } else if file_name.contains("qwen") {
        PromptPreset::Qwen
    } else {
        PromptPreset::Generic
    }
}

fn resolve_prompt_profile(config: &TranslationConfig) -> ModelPromptProfile {
    let effective_preset = if config.prompt_preset == PromptPreset::Auto {
        resolve_preset_for_model(config.model.as_deref())
    } else {
        config.prompt_preset
    };

    if effective_preset == PromptPreset::Custom {
        return ModelPromptProfile {
            system_prompt: config.custom_system_prompt.clone().unwrap_or_default(),
            prompt_template: config.custom_prompt_template.clone().unwrap_or_else(|| {
                "将以下文本翻译成{target}：\n{text}".to_string()
            }),
        };
    }

    preset_profiles()
        .into_iter()
        .find(|(p, _)| *p == effective_preset)
        .map(|(_, profile)| profile)
        .unwrap_or_else(|| ModelPromptProfile {
            system_prompt: String::new(),
            prompt_template: "将以下文本翻译成{target}：\n{text}".to_string(),
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranslationMode {
    Always,
    Auto,
}

#[derive(Clone, Copy, Debug)]
pub struct TranslationLanguage {
    pub code: &'static str,
    pub prompt_name: &'static str,
}

#[derive(Deserialize, Debug)]
struct ChatCompletionChoice {
    message: ChatMessage,
}

#[derive(Deserialize, Debug)]
struct ChatMessage {
    content: String,
}

#[derive(Deserialize, Debug)]
struct ChatCompletionResponse {
    choices: Vec<ChatCompletionChoice>,
}

#[derive(Serialize, Debug)]
struct ChatCompletionRequest<'a> {
    messages: Vec<ChatMessageRequest<'a>>,
    #[serde(rename = "max_tokens")]
    max_tokens: i32,
    temperature: f32,
    #[serde(rename = "stream")]
    stream: bool,
    #[serde(rename = "cache_prompt")]
    cache_prompt: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_template_kwargs: Option<serde_json::Value>,
}

#[derive(Serialize, Debug)]
struct ChatMessageRequest<'a> {
    role: &'a str,
    content: &'a str,
}

struct ServerProcess {
    #[allow(dead_code)]
    child: Child,
    port: u16,
    model_path: PathBuf,
}

struct ServerHandle {
    pid: u32,
    port: u16,
}

/// Last spawned llama-server PID, stored for emergency cleanup on Ctrl+C / crash.
/// Reset to 0 when the server is shut down gracefully.
static LAST_LLAMA_SERVER_PID: AtomicU32 = AtomicU32::new(0);

/// Kill the last-spawned llama-server if it is still running.
/// Called from signal handlers on abrupt exit.
pub fn kill_orphan_llama_server() {
    let pid = LAST_LLAMA_SERVER_PID.swap(0, Ordering::Acquire);
    if pid > 0 {
        kill_pid(pid);
    }
}

fn kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        std::thread::sleep(std::time::Duration::from_millis(200));
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
            libc::waitpid(pid as i32, std::ptr::null_mut(), libc::WNOHANG);
        }
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

pub struct TranslationEngine {
    config: TranslationConfig,
    server: Arc<Mutex<Option<ServerProcess>>>,
    handle: std::sync::Arc<std::sync::Mutex<Option<ServerHandle>>>,
    client: reqwest::Client,
    startup: Mutex<()>,
}

impl TranslationEngine {
    pub fn from_app_config(config: &AppConfig) -> Self {
        Self {
            config: TranslationConfig::from_app_config(config),
            server: Arc::new(Mutex::new(None)),
            handle: std::sync::Arc::new(std::sync::Mutex::new(None)),
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            startup: Mutex::new(()),
        }
    }

    /// Pre-start llama-server so the first translation is fast.
    /// Safe to call multiple times; no-op if already running.
    pub async fn warmup(&self) {
        if !self.is_enabled() {
            return;
        }
        let model = match self.config.model.as_ref() {
            Some(p) if p.exists() => p.clone(),
            _ => return,
        };
        let _ = self.ensure_server_running(&model).await;
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    pub async fn translate(&self, text: &str) -> Result<Option<String>> {
        let text = text.trim();
        if !self.config.enabled || text.is_empty() {
            return Ok(None);
        }

        // Resolve input based on translation mode
        let (payload, target) = self.resolve_input(text);
        if payload.is_empty() {
            // AUTO mode with no trigger match — skip translation
            return Ok(None);
        }

        let model = match self.config.model.as_ref() {
            Some(path) if path.exists() => path,
            Some(path) => bail!(
                "translation model does not exist: {}",
                crate::path_display::for_log_path(path)
            ),
            None => bail!("translation enabled but no GGUF model configured"),
        };

        let port = self.ensure_server_running(model).await?;
        let profile = resolve_prompt_profile(&self.config);
        let (system_prompt, user_prompt) = profile.render(&payload, target);
        let max_tokens = effective_max_tokens(model, &payload);
        let is_qwen = resolve_preset_for_model(Some(model)) == PromptPreset::Qwen
            || (self.config.prompt_preset == PromptPreset::Auto
                && resolve_preset_for_model(self.config.model.as_deref()) == PromptPreset::Qwen);

        tracing::debug!(
            "translating to {} via llama-server on port {port} (preset={:?}, system_prompt={}): {payload:?}",
            target.code,
            self.config.prompt_preset,
            if system_prompt.is_empty() { "none" } else { "yes" },
        );

        let mut messages = Vec::new();
        if !system_prompt.is_empty() {
            messages.push(ChatMessageRequest {
                role: "system",
                content: &system_prompt,
            });
        }
        messages.push(ChatMessageRequest {
            role: "user",
            content: &user_prompt,
        });

        let chat_template_kwargs = if is_qwen {
            Some(serde_json::json!({"enable_thinking": false}))
        } else {
            None
        };

        let request = ChatCompletionRequest {
            messages,
            max_tokens,
            temperature: self.config.temperature,
            stream: false,
            cache_prompt: true,
            chat_template_kwargs,
        };

        let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
        let resp = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::debug!("llama-server error response: {body}");
            bail!("llama-server returned {status}");
        }

        let raw_body = resp.text().await.with_context(|| "read chat completion response body")?;
        tracing::debug!("llama-server raw response: {raw_body}");
        let completion: ChatCompletionResponse =
            serde_json::from_str(&raw_body).with_context(|| "parse chat completion response")?;
        let translated = completion
            .choices
            .first()
            .map(|c| c.message.content.as_str())
            .unwrap_or("")
            .to_string();
        let translated = clean_translation_output(&translated);
        if translated.is_empty() {
            warn!("llama-server returned empty translation");
            return Ok(None);
        }
        Ok(Some(translated))
    }

    async fn ensure_server_running(&self, model: &Path) -> Result<u16> {
        // Fast path: already running with the same model
        {
            let guard = self.server.lock().await;
            if let Some(ref sp) = *guard {
                if sp.model_path == model {
                    return Ok(sp.port);
                }
                info!(
                    "model switch detected: {} -> {}, restarting llama-server",
                    crate::path_display::for_log_path(&sp.model_path),
                    crate::path_display::for_log_path(model),
                );
            }
        }

        // Model switched or not running — kill old server if needed
        {
            let mut guard = self.server.lock().await;
            if let Some(ref sp) = *guard {
                if sp.model_path != model {
                    let handle = self.handle.lock().unwrap();
                    if let Some(ref h) = *handle {
                        info!("killing old llama-server pid={}", h.pid);
                        kill_pid(h.pid);
                    }
                }
            }
            *guard = None;
        }

        // Serialize startup to prevent racing multiple server processes
        let _startup_guard = self.startup.lock().await;

        // Double-check after acquiring the startup lock
        {
            let guard = self.server.lock().await;
            if let Some(ref sp) = *guard {
                if sp.model_path == model {
                    return Ok(sp.port);
                }
            }
        }

        // Start llama-server on a random available port
        let port = find_available_port()?;
        info!(
            "starting llama-server on port {port} with model {}",
            crate::path_display::for_log_path(model)
        );

        // Spawn with tokio for async IO draining, but also capture PID
        let mut command = Command::new(&self.config.llama_server);
        command
            .arg("-m")
            .arg(model)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .arg("--ctx-size")
            .arg(self.config.ctx_size.to_string())
            .arg("--threads")
            .arg(self.config.num_threads.max(1).to_string())
            .arg("--gpu-layers")
            .arg(effective_gpu_layers(model, self.config.gpu_layers).to_string())
            .arg("--batch-size")
            .arg("512")
            .arg("--ubatch-size")
            .arg("512")
            .arg("--no-perf")
            .arg("--no-webui")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        for arg in &self.config.extra_args {
            command.arg(arg);
        }

        if let Some(lib_dir) = self.config.llama_lib_dir.as_ref() {
            append_library_path(&mut command, lib_dir);
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("spawn llama-server from {}", crate::path_display::for_log_path(&self.config.llama_server)))?;

        let pid = child.id().expect("child should have PID");
        LAST_LLAMA_SERVER_PID.store(pid, Ordering::Release);

        // Drain stderr/stdout in background
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    tracing::debug!("llama-server: {line}");
                }
            });
        }
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    tracing::debug!("llama-server: {line}");
                }
            });
        }

        // Wait for server to be ready
        let health_url = format!("http://127.0.0.1:{port}/v1/health");
        let deadline = TokioDuration::from_secs(SERVER_STARTUP_TIMEOUT_SECS);
        let poll_interval = TokioDuration::from_millis(HEALTH_POLL_INTERVAL_MS);
        let start = tokio::time::Instant::now();

        loop {
            // If the child exited prematurely, fail fast with the real exit status.
            if let Some(status) = child.try_wait()? {
                bail!(
                    "llama-server exited with {status} before becoming healthy (check that args and model path are correct)"
                );
            }
            if start.elapsed() >= deadline {
                let _ = child.kill().await;
                bail!("llama-server did not become healthy within {SERVER_STARTUP_TIMEOUT_SECS}s");
            }
            match self.client.get(&health_url).timeout(TokioDuration::from_millis(HEALTH_POLL_INTERVAL_MS)).send().await {
                Ok(resp) if resp.status().is_success() => break,
                Ok(_) | Err(_) => {
                    sleep(poll_interval).await;
                }
            }
        }

        info!("llama-server healthy on port {port}");

        // Store PID for reliable cleanup
        {
            let mut h = self.handle.lock().unwrap();
            // Kill any previous orphan before storing new one
            if let Some(ref old) = *h {
                info!("replacing old llama-server pid={} with new pid={pid}", old.pid);
                kill_pid(old.pid);
            }
            *h = Some(ServerHandle { pid, port });
        }

        let mut guard = self.server.lock().await;
        *guard = Some(ServerProcess {
            child,
            port,
            model_path: model.to_path_buf(),
        });
        Ok(port)
    }

    /// Resolve input text based on translation mode.
    /// Returns (payload_to_translate, target_language).
    fn resolve_input(&self, text: &str) -> (String, TranslationLanguage) {
        match self.config.mode {
            TranslationMode::Always => {
                let detected = detect_language(text).unwrap_or(self.config.target);
                (text.to_string(), detected)
            }
            TranslationMode::Auto => {
                let lower = text.to_lowercase();
                let trigger = self
                    .config
                    .triggers
                    .iter()
                    .find(|t| lower.contains(&t.to_lowercase()));

                // If no explicit trigger found, check if a language alias appears at
                // the very start of the text (common when VAD clips the trigger word
                // but keeps the language hint, e.g. "成英语，..." from "翻译成英语，...").
                let implicit_trigger = trigger.is_none()
                    && detect_language(text)
                        .and_then(|l| find_language_alias_range(text, l))
                        .map(|(start, _)| start <= 2)
                        .unwrap_or(false);

                if trigger.is_none() && !implicit_trigger {
                    tracing::debug!("no trigger phrase or language hint matched, skipping translation");
                    return (String::new(), self.config.target);
                }

                let detected = detect_language(text);
                let target = detected.unwrap_or(self.config.target);
                if self.config.strip_hints {
                    let stripped = strip_trigger_and_language(
                        text,
                        trigger.map(|s| s.as_str()).unwrap_or(""),
                        detected,
                    );
                    let payload = if stripped.trim().is_empty() {
                        text.to_string()
                    } else {
                        stripped
                    };
                    (payload, target)
                } else {
                    (text.to_string(), target)
                }
            }
        }
    }
}

impl Drop for TranslationEngine {
    fn drop(&mut self) {
        if let Some(h) = self.handle.lock().unwrap().take() {
            info!("shutting down llama-server pid={} port={}", h.pid, h.port);
            LAST_LLAMA_SERVER_PID.store(0, Ordering::Release);
            kill_pid(h.pid);
        }
    }
}

impl TranslationConfig {
    fn from_app_config(config: &AppConfig) -> Self {
        Self {
            enabled: config.translation_enabled,
            mode: match config.translation_mode.as_str() {
                "auto" => TranslationMode::Auto,
                _ => TranslationMode::Always,
            },
            triggers: config
                .translation_triggers
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            strip_hints: config.translation_strip_hints,
            target: language_from_code(&config.translation_target),
            model: config.translation_model.as_ref().map(PathBuf::from),
            llama_server: config
                .translation_llama_server
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(default_llama_server),
            llama_lib_dir: config.translation_llama_lib_dir.as_ref().map(PathBuf::from),
            gpu_layers: config.translation_gpu_layers,
            ctx_size: config.translation_ctx_size.max(512),
            temperature: config.translation_temperature,
            extra_args: config.translation_extra_args.clone(),
            num_threads: config.num_threads,
            prompt_preset: PromptPreset::from_str_lossy(
                config.translation_prompt_preset.as_deref().unwrap_or("auto"),
            ),
            custom_system_prompt: config.translation_custom_system_prompt.clone(),
            custom_prompt_template: config.translation_custom_prompt_template.clone(),
        }
    }
}

fn find_available_port() -> Result<u16> {
    let listener =
        TcpListener::bind("127.0.0.1:0").context("find available port for llama-server")?;
    Ok(listener.local_addr()?.port())
}

pub fn language_from_code(code: &str) -> TranslationLanguage {
    match code {
        "zh" | "zh-CN" | "zh-TW" => TranslationLanguage {
            code: "zh",
            prompt_name: "中文",
        },
        "ja" => TranslationLanguage {
            code: "ja",
            prompt_name: "日文",
        },
        "ko" => TranslationLanguage {
            code: "ko",
            prompt_name: "韩文",
        },
        "fr" => TranslationLanguage {
            code: "fr",
            prompt_name: "法文",
        },
        "de" => TranslationLanguage {
            code: "de",
            prompt_name: "德文",
        },
        "es" => TranslationLanguage {
            code: "es",
            prompt_name: "西班牙文",
        },
        "ru" => TranslationLanguage {
            code: "ru",
            prompt_name: "俄文",
        },
        "pt" => TranslationLanguage {
            code: "pt",
            prompt_name: "葡萄牙文",
        },
        "it" => TranslationLanguage {
            code: "it",
            prompt_name: "意大利文",
        },
        "th" => TranslationLanguage {
            code: "th",
            prompt_name: "泰文",
        },
        "vi" => TranslationLanguage {
            code: "vi",
            prompt_name: "越南文",
        },
        "ar" => TranslationLanguage {
            code: "ar",
            prompt_name: "阿拉伯文",
        },
        _ => TranslationLanguage {
            code: "en",
            prompt_name: "英文",
        },
    }
}

fn default_llama_server() -> PathBuf {
    let exe_name = if cfg!(target_os = "windows") {
        "llama-server.exe"
    } else {
        "llama-server"
    };

    // Prefer the copy next to our binary (build.rs copies it there).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let next_to_exe = dir.join(exe_name);
            if next_to_exe.exists() {
                return next_to_exe;
            }
        }
    }

    // Fallback: CWD (for `cargo run` development).
    let cwd = PathBuf::from(exe_name);
    if cwd.exists() {
        return cwd;
    }

    // Fallback: local build tree.
    let local_name = if cfg!(target_os = "windows") {
        "llama.cpp-master/build/bin/llama-server.exe"
    } else {
        "llama.cpp-master/build/bin/llama-server"
    };
    let local = PathBuf::from(local_name);
    if local.exists() {
        return local;
    }

    // Last resort: bare filename (Command::new will search PATH).
    PathBuf::from(exe_name)
}

fn effective_gpu_layers(model: &Path, configured: i32) -> i32 {
    if is_low_bit_model(model) {
        0
    } else {
        configured.max(0)
    }
}

fn effective_max_tokens(model: &Path, text: &str) -> i32 {
    let estimated_input_tokens = (text.chars().count() as i32 / 2).max(16);
    let estimated_output_tokens = estimated_input_tokens * 2;
    if is_low_bit_model(model) {
        estimated_output_tokens.clamp(
            LOW_BIT_MIN_TRANSLATION_TOKENS,
            LOW_BIT_MAX_TRANSLATION_TOKENS,
        )
    } else {
        estimated_output_tokens.clamp(
            DEFAULT_MIN_TRANSLATION_TOKENS,
            DEFAULT_MAX_TRANSLATION_TOKENS,
        )
    }
}

fn is_low_bit_model(model: &Path) -> bool {
    let file_name = model
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_lowercase();
    ["1.25bit", "1.25-bit", "1_25bit", "iq1", "iq1_", "i1_"]
        .iter()
        .any(|needle| file_name.contains(needle))
}

fn append_library_path(command: &mut Command, lib_dir: &Path) {
    #[cfg(target_os = "linux")]
    append_env_path(command, "LD_LIBRARY_PATH", lib_dir);
    #[cfg(target_os = "macos")]
    append_env_path(command, "DYLD_LIBRARY_PATH", lib_dir);
    #[cfg(target_os = "windows")]
    append_env_path(command, "PATH", lib_dir);
}

fn append_env_path(command: &mut Command, key: &str, dir: &Path) {
    let current = std::env::var_os(key).unwrap_or_default();
    let mut paths: Vec<PathBuf> = std::env::split_paths(&current).collect();
    paths.insert(0, dir.to_path_buf());
    if let Ok(joined) = std::env::join_paths(paths) {
        command.env(key, joined);
    }
}

fn clean_translation_output(output: &str) -> String {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Return the (start, end) byte range of the first alias match in [text] for [lang], or None.
fn find_language_alias_range(text: &str, lang: TranslationLanguage) -> Option<(usize, usize)> {
    let lower = text.to_lowercase();
    let mut best_start = usize::MAX;
    let mut best_end = 0usize;
    for alias in lang_aliases(lang) {
        let a = alias.to_lowercase();
        if let Some(idx) = lower.find(&a) {
            if idx < best_start {
                best_start = idx;
                best_end = idx + a.len();
            }
        }
    }
    if best_end == 0 {
        None
    } else {
        Some((best_start, best_end))
    }
}

fn lang_aliases(lang: TranslationLanguage) -> &'static [&'static str] {
    match lang.code {
        "zh" => &["中文", "汉语", "普通话", "chinese", "mandarin"],
        "ja" => &["日文", "日语", "japanese"],
        "ko" => &["韩文", "韩语", "朝鲜语", "korean"],
        "fr" => &["法文", "法语", "french"],
        "de" => &["德文", "德语", "german"],
        "es" => &["西班牙文", "西班牙语", "西语", "spanish"],
        "ru" => &["俄文", "俄语", "russian"],
        "pt" => &["葡萄牙文", "葡萄牙语", "葡语", "portuguese"],
        "it" => &["意大利文", "意大利语", "意语", "italian"],
        "th" => &["泰文", "泰语", "thai"],
        "vi" => &["越南文", "越南语", "越语", "vietnamese"],
        "ar" => &["阿拉伯文", "阿拉伯语", "arabic"],
        _ => &["英文", "英语", "english"],
    }
}

/// Detect target language from text, returning the one whose alias appears earliest.
fn detect_language(text: &str) -> Option<TranslationLanguage> {
    let lower = text.to_lowercase();
    // Ordered list of languages to check (English last since "英语" is common)
    let candidates: &[TranslationLanguage] = &[
        language_from_code("zh"),
        language_from_code("ja"),
        language_from_code("ko"),
        language_from_code("fr"),
        language_from_code("de"),
        language_from_code("es"),
        language_from_code("ru"),
        language_from_code("pt"),
        language_from_code("it"),
        language_from_code("th"),
        language_from_code("vi"),
        language_from_code("ar"),
        language_from_code("en"),
    ];
    let mut best_lang: Option<TranslationLanguage> = None;
    let mut best_idx = usize::MAX;
    for lang in candidates {
        for alias in lang_aliases(*lang) {
            if let Some(idx) = lower.find(&alias.to_lowercase()) {
                if idx < best_idx {
                    best_idx = idx;
                    best_lang = Some(*lang);
                }
            }
        }
    }
    best_lang
}

/// Strip the trigger phrase and detected language hint from text.
fn strip_trigger_and_language(
    text: &str,
    trigger: &str,
    lang: Option<TranslationLanguage>,
) -> String {
    let lower = text.to_lowercase();

    // Find start and end of the trigger phrase
    let trigger_start = if trigger.is_empty() {
        0
    } else {
        lower.find(&trigger.to_lowercase()).unwrap_or(0)
    };
    let trigger_end = trigger_start + trigger.len();

    // Find end of language alias (if present and after trigger)
    let lang_range = lang.and_then(|l| find_language_alias_range(text, l));
    let cut_end = match lang_range {
        Some((_, end)) if end > trigger_end => end,
        _ => trigger_end,
    };

    if cut_end <= trigger_start {
        return text.to_string();
    }

    // Strip from trigger_start to cut_end (removing both trigger and language hint)
    let mut result =
        String::with_capacity(text.len().saturating_sub(cut_end - trigger_start));
    result.push_str(&text[..trigger_start.min(text.len())]);
    result.push_str(&text[cut_end.min(text.len())..]);

    let trimmed = result.trim();
    trimmed
        .trim_start_matches(|c: char| c == '，' || c == ',' || c == '。' || c == '.' || c == ' ')
        .to_string()
}
