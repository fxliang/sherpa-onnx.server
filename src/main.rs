mod app;
mod config;
mod download;
mod engine;
mod i18n;
mod log;
mod mdns;
mod model;
mod path_display;
mod server;
mod translation;
mod tui;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use clap::Parser;
use tokio::sync::{mpsc, watch};
use tracing::info;

use crate::app::App;
use crate::config::MirrorRule;
use crate::log::LogBuffer;
use crate::tui::TuiCommand;

#[derive(Parser, Debug)]
#[command(
    name = "sherpa-onnx.server",
    about = "Cross-platform TUI server for sherpa-onnx ASR"
)]
struct Cli {
    /// WebSocket listen host
    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    /// WebSocket listen port
    #[arg(long, default_value_t = 8765)]
    port: u16,

    /// Models directory
    #[arg(long)]
    models_dir: Option<PathBuf>,

    /// Skip TUI, start server directly from active model
    #[arg(long)]
    start: bool,

    /// Disable TUI, run headless with stdout logging
    #[arg(long)]
    no_tui: bool,

    /// Denoiser model path or imported model directory
    #[arg(long)]
    denoiser_model: Option<PathBuf>,

    /// VoxLock reference voice sample WAV (at least 3 seconds)
    #[arg(long)]
    voxlock_voice_sample: Option<PathBuf>,

    /// Log level
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Enable server-side llama.cpp translation for final ASR results
    #[arg(long)]
    translate: bool,

    /// Translation GGUF model path
    #[arg(long)]
    translate_model: Option<PathBuf>,

    /// llama.cpp server binary path
    #[arg(long)]
    llama_server: Option<PathBuf>,

    /// Directory containing llama.cpp shared libraries
    #[arg(long)]
    llama_lib_dir: Option<PathBuf>,

    /// Target language code for translation output
    #[arg(long)]
    translate_target: Option<String>,

    /// Number of GPU layers for llama.cpp translation
    #[arg(long)]
    translate_gpu_layers: Option<i32>,

    /// Prompt preset for translation: auto, hy_mt2, qwen, generic, custom
    #[arg(long)]
    prompt_preset: Option<String>,

    /// Custom system prompt (for --prompt-preset custom)
    #[arg(long)]
    prompt_system: Option<String>,

    /// Custom prompt template with {text} and {target} (for --prompt-preset custom)
    #[arg(long)]
    prompt_template: Option<String>,
}

fn default_store_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".sherpa-onnx.server"))
        .unwrap_or_else(|| PathBuf::from(".sherpa-onnx.server"))
}

/// Restore terminal to a sane state even after a crash.
/// Sends escape sequences to disable mouse tracking, show cursor,
/// and exit alternate screen.
fn restore_terminal() {
    use std::io::Write;
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(
        b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?25h\x1b[?1049l\x1b[0m",
    );
    let _ = stderr.flush();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let store_dir = cli.models_dir.clone().unwrap_or_else(default_store_dir);
    std::fs::create_dir_all(&store_dir)?;

    let log_buffer = Arc::new(LogBuffer::new());
    let to_stdout = cli.no_tui || cli.start;
    let _guard = log::setup_logging(log_buffer.clone(), to_stdout, &cli.log_level);

    let mut app_state = App::new(store_dir, cli.host.clone(), cli.port)?;
    apply_cli_translation_overrides(&mut app_state.config, &cli);
    if let Some(path) = cli.denoiser_model.as_ref() {
        app_state.config.denoiser_model = Some(path.to_string_lossy().into_owned());
    }
    if let Some(path) = cli.voxlock_voice_sample.as_ref() {
        app_state.config.voxlock_voice_sample = Some(path.to_string_lossy().into_owned());
    }
    let app = Arc::new(Mutex::new(app_state));

    // Apply UI language from config (or auto-detect from locale).
    {
        let a = app.lock().unwrap();
        let lang = a
            .config
            .language
            .as_deref()
            .and_then(crate::i18n::Lang::from_code)
            .unwrap_or_else(crate::i18n::detect_default_lang);
        crate::i18n::set_lang(lang);
    }

    // Register signal handler to kill llama-server on abrupt exit (Ctrl+C, SIGTERM).
    // On Windows child processes are not auto-killed when the parent exits.
    tokio::spawn(async {
        let _ = tokio::signal::ctrl_c().await;
        crate::translation::kill_orphan_llama_server();
        std::process::exit(1);
    });
    #[cfg(unix)]
    tokio::spawn(async {
        let _ = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .unwrap()
            .recv()
            .await;
        crate::translation::kill_orphan_llama_server();
        std::process::exit(1);
    });

    if cli.start || cli.no_tui {
        run_headless(app, cli.host, cli.port).await
    } else {
        run_with_tui(app, log_buffer, cli.host, cli.port).await
    }
}

fn apply_cli_translation_overrides(config: &mut crate::config::AppConfig, cli: &Cli) {
    if cli.translate {
        config.translation_enabled = true;
    }
    if let Some(path) = cli.translate_model.as_ref() {
        config.translation_model = Some(path.to_string_lossy().into_owned());
    }
    if let Some(path) = cli.llama_server.as_ref() {
        config.translation_llama_server = Some(path.to_string_lossy().into_owned());
    }
    if let Some(path) = cli.llama_lib_dir.as_ref() {
        config.translation_llama_lib_dir = Some(path.to_string_lossy().into_owned());
    }
    if let Some(target) = cli.translate_target.as_ref() {
        config.translation_target = target.clone();
    }
    if let Some(gpu_layers) = cli.translate_gpu_layers {
        config.translation_gpu_layers = gpu_layers;
    }
    if let Some(preset) = cli.prompt_preset.as_ref() {
        config.translation_prompt_preset = Some(preset.clone());
    }
    if let Some(system) = cli.prompt_system.as_ref() {
        config.translation_custom_system_prompt = Some(system.clone());
    }
    if let Some(template) = cli.prompt_template.as_ref() {
        config.translation_custom_prompt_template = Some(template.clone());
    }
}

/// Load the active model from config WITHOUT holding the App mutex.
/// This runs the expensive ONNX loading on the calling thread but keeps
/// the TUI responsive (the polling thread can still acquire the lock).
fn load_active_from_config(
    runtime: &crate::engine::ServerRuntime,
    config: &crate::config::AppConfig,
    store_dir: &std::path::Path,
) -> anyhow::Result<()> {
    let active = config.active_model.as_deref();
    let path = match active {
        Some(p) => std::path::PathBuf::from(p),
        None => anyhow::bail!("No active model configured"),
    };
    if !path.exists() {
        anyhow::bail!(
            "Active model path does not exist: {}",
            crate::path_display::for_log_path(&path)
        );
    }
    // Resolve relative paths against store_dir (config may store relative paths).
    let model_dir = if path.is_absolute() {
        path
    } else {
        // config.active_model may be just the model directory name
        // (e.g. "sherpa-onnx-sense-voice-funasr-nano-int8-2025-12-17").
        // Models always live under store_dir/models/.
        let models = crate::model::models_dir(store_dir);
        let candidate = models.join(&path);
        if candidate.exists() {
            candidate
        } else if path.exists() {
            // Fallback: relative to CWD (backward compat)
            path
        } else {
            anyhow::bail!(
                "Active model path does not exist: {} (looked in {} and CWD)",
                crate::path_display::for_log_path(&path),
                crate::path_display::for_log_path(&models),
            );
        }
    };
    let info = crate::model::detect::detect_model_with_metadata(&model_dir, Some(config))?;
    runtime.load_model(&info, config)?;
    Ok(())
}

async fn run_headless(app: Arc<Mutex<App>>, host: String, port: u16) -> Result<()> {
    let runtime = {
        let mut app = app.lock().unwrap();
        app.load_active_model()?;
        app.runtime.clone()
    };

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    info!("Starting server in headless mode...");
    let result = server::run_server(&host, port, runtime.clone(), shutdown_rx, None).await;

    // Kill translation child before exiting
    let translation = {
        let a = app.lock().unwrap();
        a.runtime
            .config
            .write()
            .ok()
            .and_then(|mut cfg| cfg.translation.take())
    };
    drop(translation);

    let _ = shutdown_tx.send(true);
    result.map_err(Into::into)
}

async fn run_with_tui(
    app: Arc<Mutex<App>>,
    log_buffer: Arc<LogBuffer>,
    host: String,
    port: u16,
) -> Result<()> {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<TuiCommand>(64);

    let app_tui = app.clone();
    let log_buf_tui = log_buffer.clone();
    let cmd_tx_tui = cmd_tx.clone();

    let tui_join = std::thread::spawn(move || {
        let mut siv = cursive::default();
        tui::build_ui(&mut siv, app_tui, log_buf_tui, cmd_tx_tui);
        tui::run_with_terminal_selection(&mut siv);
    });
    // Do not keep an extra sender alive in this task thread; otherwise if
    // TUI exits unexpectedly, cmd_rx.recv() may wait forever.
    drop(cmd_tx);

    // Per-server-lifecycle shutdown channel + join handle. We hold both so
    // StopServer / Quit / the next StartServer can signal the previous task
    // AND wait for it to finish — otherwise the next `TcpListener::bind`
    // races the still-alive old listener and fails with EADDRINUSE.
    let mut current_server: Option<(watch::Sender<bool>, tokio::task::JoinHandle<()>)> = None;
    // Lifecycle token for start/stop races. New start/stop bumps the token;
    // only the newest server task is allowed to flip running=false on exit.
    let server_lifecycle = Arc::new(AtomicU64::new(0));

    async fn stop_current(
        current: &mut Option<(watch::Sender<bool>, tokio::task::JoinHandle<()>)>,
    ) {
        if let Some((tx, handle)) = current.take() {
            let _ = tx.send(true);
            // Await full task exit so the TcpListener/mDNS handle are dropped
            // before any subsequent rebind. Errors here (e.g. JoinError on
            // panic) are not actionable; the task is gone either way.
            let _ = handle.await;
        }
    }

    loop {
        match cmd_rx.recv().await {
            None => break,
            Some(cmd) => match cmd {
                TuiCommand::Quit => {
                    // Kill llama-server before draining connections — take the
                    // translation engine out of the runtime and drop it explicitly.
                    let translation = {
                        let a = app.lock().unwrap();
                        a.runtime
                            .config
                            .write()
                            .ok()
                            .and_then(|mut cfg| cfg.translation.take())
                    };
                    drop(translation);
                    stop_current(&mut current_server).await;
                    break;
                }
                TuiCommand::SelectModel(idx) => {
                    // Snapshot what we need, save config, then release the lock
                    // before the expensive runtime.load_model call.
                    let (runtime, config, model_dir, store_dir, select_ok) = {
                        let mut a = app.lock().unwrap();
                        let models = a.list_models();
                        if idx >= models.len() {
                            (None, None, None, None, false)
                        } else {
                            let model = &models[idx];
                            let model_dir = model.dir.clone();
                            a.config.active_model = Some(model_dir.to_string_lossy().into_owned());
                            let _ = a.config.save(&a.store_dir);
                            info!("Selected model: {}", model.name);
                            (
                                Some(a.runtime.clone()),
                                Some(a.config.clone()),
                                Some(model_dir),
                                Some(a.store_dir.clone()),
                                true,
                            )
                        }
                    }; // lock released
                    if !select_ok {
                        tracing::error!("Select model failed: invalid index");
                        continue;
                    }
                    let runtime = runtime.unwrap();
                    let config = config.unwrap();
                    let model_dir = model_dir.unwrap();
                    let store_dir = store_dir.unwrap();

                    // Reload if server is already running (expensive, no lock held).
                    {
                        let a = app.lock().unwrap();
                        if a.server_running {
                            drop(a);
                            let model_dir = if model_dir.is_absolute() {
                                model_dir
                            } else {
                                store_dir.join(&model_dir)
                            };
                            match crate::model::detect::detect_model_with_metadata(
                                &model_dir,
                                Some(&config),
                            ) {
                                Ok(info) => {
                                    if let Err(e) = runtime.load_model(&info, &config) {
                                        tracing::error!("Runtime reload failed: {e}");
                                    } else {
                                        info!("Runtime reloaded with model: {}", info.name);
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Model detection failed after select: {e}");
                                }
                            }
                        }
                    }
                }
                TuiCommand::DeleteModel(idx) => {
                    let mut app = app.lock().unwrap();
                    if let Err(e) = app.delete_model(idx) {
                        tracing::error!("Delete model failed: {e}");
                    }
                }
                TuiCommand::DeleteAuxModel(dir) => {
                    let mut app = app.lock().unwrap();
                    if let Err(e) = app.delete_aux_model(Path::new(&dir)) {
                        tracing::error!("Delete auxiliary model failed: {e}");
                    }
                }
                TuiCommand::StartServer => {
                    // Clone what we need and RELEASE the app lock BEFORE the
                    // expensive model-loading step.
                    let (runtime, config, store_dir, already_running) = {
                        let a = app.lock().unwrap();
                        if a.server_running {
                            (None, None, None, true)
                        } else {
                            (
                                Some(a.runtime.clone()),
                                Some(a.config.clone()),
                                Some(a.store_dir.clone()),
                                false,
                            )
                        }
                    }; // lock released
                    if already_running {
                        continue;
                    }
                    let runtime = runtime.unwrap();
                    let config = config.unwrap();
                    let store_dir = store_dir.unwrap();

                    // Signal old session threads (from any prior server) to
                    // stop acquiring new read-locks so load_model below can
                    // acquire the write-lock without writer starvation
                    // (pthread_rwlock on Linux prefers readers by default).
                    runtime.shutting_down.store(true, Ordering::Release);

                    // Make sure any prior task has fully exited (its listener
                    // dropped) before we try to bind the new one.
                    stop_current(&mut current_server).await;

                    // Wait for old session threads to drain their read-locks.
                    // Without this, load_model→config.write() can block
                    // indefinitely because pthread_rwlock starves writers.
                    {
                        use std::time::Duration;
                        let mut acquired = false;
                        for i in 0..30 {
                            match runtime.config.try_write() {
                                Ok(_guard) => {
                                    // Write lock acquired → no readers active.
                                    drop(_guard);
                                    acquired = true;
                                    break;
                                }
                                Err(_) => {
                                    // Readers still active, wait and retry.
                                    let ms = (50 * (i + 1)).min(500);
                                    tokio::time::sleep(Duration::from_millis(ms as u64)).await;
                                }
                            }
                        }
                        if !acquired {
                            tracing::error!(
                                "Timed out waiting for old sessions to drain; restart may fail"
                            );
                        }
                    }

                    match load_active_from_config(&runtime, &config, &store_dir) {
                        Ok(()) => {
                            let server_epoch = server_lifecycle.fetch_add(1, Ordering::AcqRel) + 1;
                            info!("Server lifecycle epoch {server_epoch} starting");
                            // Allow new sessions to acquire read-locks.
                            runtime.shutting_down.store(false, Ordering::Release);
                            let (tx, rx) = watch::channel(false);
                            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
                            let h = host.clone();
                            let app_for_server = app.clone();
                            let runtime_for_server = runtime.clone();
                            let lifecycle_for_server = server_lifecycle.clone();
                            info!(
                                "Waiting for server bind readiness on ws://{}:{}/v1/asr",
                                host, port
                            );
                            let handle = tokio::spawn(async move {
                                if let Err(e) = server::run_server(
                                    &h,
                                    port,
                                    runtime_for_server,
                                    rx,
                                    Some(ready_tx),
                                )
                                .await
                                {
                                    tracing::error!("Server error: {e}");
                                }
                                let mut a = app_for_server.lock().unwrap();
                                if lifecycle_for_server.load(Ordering::Acquire) == server_epoch {
                                    a.server_running = false;
                                }
                            });

                            use tokio::time::{timeout, Duration};
                            match timeout(Duration::from_secs(10), ready_rx).await {
                                Ok(Ok(Ok(()))) => {
                                    let mut a = app.lock().unwrap();
                                    if server_lifecycle.load(Ordering::Acquire) == server_epoch {
                                        a.server_running = true;
                                    }
                                    drop(a);
                                    info!(
                                        "Server lifecycle epoch {server_epoch} entered running state"
                                    );
                                    current_server = Some((tx, handle));
                                    info!("Server started on ws://{}:{}/v1/asr", host, port);
                                }
                                Ok(Ok(Err(e))) => {
                                    tracing::error!(
                                        "Server lifecycle epoch {server_epoch} bind failed: {e}"
                                    );
                                    runtime.shutting_down.store(false, Ordering::Release);
                                    let _ = handle.await;
                                    tracing::error!("Failed to start server: {e}");
                                }
                                Ok(Err(_)) => {
                                    tracing::error!(
                                        "Server lifecycle epoch {server_epoch} startup signal channel closed"
                                    );
                                    runtime.shutting_down.store(false, Ordering::Release);
                                    let _ = handle.await;
                                    tracing::error!(
                                        "Failed to start server: startup signal channel closed"
                                    );
                                }
                                Err(_) => {
                                    tracing::error!(
                                        "Server lifecycle epoch {server_epoch} startup readiness timeout"
                                    );
                                    runtime.shutting_down.store(false, Ordering::Release);
                                    let _ = tx.send(true);
                                    let _ = handle.await;
                                    tracing::error!(
                                        "Failed to start server: startup timed out before bind"
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            runtime.shutting_down.store(false, Ordering::Release);
                            tracing::error!("Failed to start server: {e}");
                        }
                    }
                }
                TuiCommand::StopServer => {
                    // Flip running flag first so a quick second 'r' press while
                    // we're waiting for shutdown will enqueue StartServer.
                    let translation = {
                        let mut a = app.lock().unwrap();
                        let epoch = server_lifecycle.fetch_add(1, Ordering::AcqRel) + 1;
                        info!("Server lifecycle epoch advanced to {}", epoch);
                        a.server_running = false;
                        a.runtime.shutting_down.store(true, Ordering::Release);
                        a.runtime
                            .config
                            .write()
                            .ok()
                            .and_then(|mut cfg| cfg.translation.take())
                    };
                    drop(translation);
                    stop_current(&mut current_server).await;
                    info!("Server stopped");
                }
                TuiCommand::ImportLocal(path) => {
                    let app = app.clone();
                    let p = PathBuf::from(&path);
                    if !p.exists() {
                        tracing::error!("Import path does not exist: {path}");
                        continue;
                    }
                    let import_label = p.to_string_lossy().into_owned();
                    let app_clone = app.clone();
                    let p_clone = p.clone();
                    std::thread::spawn(move || {
                        info!("Import started: {import_label}");

                        // 1) Heavy import work without holding the app lock.
                        let store_dir = {
                            let a = app_clone.lock().unwrap();
                            a.store_dir.clone()
                        };
                        let imported = match crate::model::import::import_path(
                            &p_clone,
                            &crate::model::models_dir(&store_dir),
                        ) {
                            Ok(dir) => dir,
                            Err(e) => {
                                tracing::error!("Import failed: {e}");
                                return;
                            }
                        };
                        let files = crate::model::find_files(&imported);

                        // Check if this is a GGUF translation model
                        let is_gguf = std::fs::read_dir(&imported)
                            .ok()
                            .map(|mut entries| {
                                entries.any(|e| {
                                    e.ok()
                                        .and_then(|e| {
                                            e.path()
                                                .extension()
                                                .and_then(|ext| ext.to_str().map(|s| s.to_string()))
                                        })
                                        .map(|ext| ext.eq_ignore_ascii_case("gguf"))
                                        .unwrap_or(false)
                                })
                            })
                            .unwrap_or(false);

                        // Find GGUF file path if applicable
                        let gguf_file = if is_gguf {
                            std::fs::read_dir(&imported).ok().and_then(|mut entries| {
                                entries.find_map(|e| {
                                    let e = e.ok()?;
                                    let is_gguf = e
                                        .path()
                                        .extension()
                                        .and_then(|ext| ext.to_str())
                                        .map(|ext| ext.eq_ignore_ascii_case("gguf"))
                                        .unwrap_or(false);
                                    if is_gguf {
                                        Some(e.path())
                                    } else {
                                        None
                                    }
                                })
                            })
                        } else {
                            None
                        };

                        // 2) Short lock: persist config + snapshot runtime state.
                        let (runtime, config, should_reload_asr, should_reload_translation) = {
                            let mut a = app_clone.lock().unwrap();
                            let should_reload;
                            let mut reload_translation = false;
                            if is_gguf {
                                a.config.translation_model = gguf_file
                                    .as_ref()
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .or_else(|| Some(imported.to_string_lossy().into_owned()));
                                a.config.translation_enabled = true;
                                should_reload = false;
                                reload_translation = a.server_running;
                            } else if let Some(vad) = files.vad.clone() {
                                a.config.vad_model = Some(vad.to_string_lossy().into_owned());
                                should_reload = a.server_running;
                            } else if let Some(punctuation) = files.punctuation.clone() {
                                a.config.punctuation_model =
                                    Some(punctuation.to_string_lossy().into_owned());
                                should_reload = a.server_running;
                            } else {
                                a.config.active_model =
                                    Some(imported.to_string_lossy().into_owned());
                                should_reload = a.server_running;
                            }
                            let _ = a.config.save(&a.store_dir);
                            (
                                a.runtime.clone(),
                                a.config.clone(),
                                should_reload,
                                reload_translation,
                            )
                        };

                        if should_reload_translation {
                            if let Err(e) = runtime.reload_translation(&config) {
                                tracing::error!("Translation reload failed after import: {e}");
                            }
                        }

                        // 3) If server is running and ASR model became active, reload runtime.
                        if should_reload_asr {
                            let reload_model = if files.vad.is_some() || files.punctuation.is_some()
                            {
                                config.active_model.as_deref().map(PathBuf::from)
                            } else {
                                Some(imported.clone())
                            };
                            match reload_model
                                .ok_or_else(|| anyhow::anyhow!("No active model configured"))
                                .and_then(|model_dir| {
                                    crate::model::detect::detect_model_with_metadata(
                                        &model_dir,
                                        Some(&config),
                                    )
                                }) {
                                Ok(info) => {
                                    if let Err(e) = runtime.load_model(&info, &config) {
                                        tracing::error!("Runtime reload failed after import: {e}");
                                    } else {
                                        info!("Runtime reloaded after import: {}", info.name);
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Model detection failed after import: {e}");
                                }
                            }
                        }

                        info!("Imported from {import_label}");
                    });
                }
                TuiCommand::SetDenoiser {
                    model,
                    voice_sample,
                } => {
                    let app = app.clone();
                    std::thread::spawn(move || {
                        let (runtime, config, active_model, store_dir, running) = {
                            let mut a = app.lock().unwrap();
                            a.config.denoiser_model =
                                (!model.trim().is_empty()).then(|| model.trim().to_string());
                            a.config.voxlock_voice_sample = (!voice_sample.trim().is_empty())
                                .then(|| voice_sample.trim().to_string());
                            let _ = a.config.save(&a.store_dir);
                            (
                                a.runtime.clone(),
                                a.config.clone(),
                                a.config.active_model.clone(),
                                a.store_dir.clone(),
                                a.server_running,
                            )
                        };
                        if running {
                            if let Some(active) = active_model {
                                let path = PathBuf::from(active);
                                match crate::model::detect::detect_model_with_metadata(
                                    &path,
                                    Some(&config),
                                )
                                .and_then(|info| runtime.load_model(&info, &config))
                                {
                                    Ok(()) => info!("Denoiser configuration reloaded"),
                                    Err(e) => tracing::error!("Denoiser reload failed: {e}"),
                                }
                            }
                        }
                        let _ = store_dir;
                    });
                }
                TuiCommand::SetGithubToken(token) => {
                    let mut app = app.lock().unwrap();
                    app.config.github_token = if token.is_empty() { None } else { Some(token) };
                    let _ = app.config.save(&app.store_dir);
                    info!("GitHub token updated");
                }
                TuiCommand::SetSilenceMs(ms) => {
                    let mut app = app.lock().unwrap();
                    app.config.silence_ms = ms;
                    if let Ok(mut cfg) = app.runtime.config.write() {
                        cfg.silence_ms = ms;
                        // Rebuild VAD with the new silence duration (VAD is inside
                        // EngineConfig, updated live via the runtime's config lock).
                        info!("Silence threshold updated to {} ms", ms);
                    }
                    let _ = app.config.save(&app.store_dir);
                    info!("Silence threshold saved to config: {} ms", ms);
                }
                TuiCommand::SetHostsSource(url) => {
                    let mut app = app.lock().unwrap();
                    app.config.hosts_source_url = url;
                    let _ = app.config.save(&app.store_dir);
                    info!("Hosts source URL updated");
                }
                TuiCommand::ToggleHosts => {
                    let mut app = app.lock().unwrap();
                    app.config.hosts_enabled = !app.config.hosts_enabled;
                    let enabled = app.config.hosts_enabled;
                    let _ = app.config.save(&app.store_dir);
                    info!(
                        "Hosts mapping {}",
                        if enabled { "enabled" } else { "disabled" }
                    );
                }
                TuiCommand::UpdateHosts => {
                    let app = app.clone();
                    tokio::spawn(async move {
                        let (mut config, _store_dir, http) = {
                            let a = app.lock().unwrap();
                            (a.config.clone(), a.store_dir.clone(), a.http.clone())
                        };
                        match download::client::update_hosts_from_source(&mut config, &http).await {
                            Ok(count) => {
                                let mut a = app.lock().unwrap();
                                a.config = config;
                                let _ = a.config.save(&a.store_dir);
                                info!("Updated {count} host entries from source");
                            }
                            Err(e) => {
                                tracing::error!("Failed to update hosts: {e}");
                            }
                        }
                    });
                }
                TuiCommand::AddMirror {
                    pattern,
                    replacement,
                } => {
                    let mut app = app.lock().unwrap();
                    app.config.mirrors.push(MirrorRule {
                        pattern,
                        replacement,
                        enabled: true,
                    });
                    let _ = app.config.save(&app.store_dir);
                    info!("Mirror rule added");
                }
                TuiCommand::ToggleMirror(idx) => {
                    let mut app = app.lock().unwrap();
                    let enabled = if let Some(m) = app.config.mirrors.get_mut(idx) {
                        m.enabled = !m.enabled;
                        m.enabled
                    } else {
                        continue;
                    };
                    let _ = app.config.save(&app.store_dir);
                    info!(
                        "Mirror rule {} {}",
                        idx,
                        if enabled { "enabled" } else { "disabled" }
                    );
                }
                TuiCommand::ClearMirrors => {
                    let mut app = app.lock().unwrap();
                    app.config.mirrors.clear();
                    let _ = app.config.save(&app.store_dir);
                    info!("All mirror rules cleared");
                }
                TuiCommand::ReloadTranslation => {
                    let (runtime, config) = {
                        let a = app.lock().unwrap();
                        (a.runtime.clone(), a.config.clone())
                    };
                    if let Err(e) = runtime.reload_translation(&config) {
                        tracing::error!("Translation reload failed: {e}");
                    }
                }
            },
        }
    }

    // Wait for the cursive thread to fully restore terminal state
    // (mouse tracking off, cursor visible, alternate screen exited).
    let _ = tui_join.join();

    // Belt-and-suspenders: even if cursive's Drop didn't get to run
    // (panic, signal, etc.), force the terminal back to a sane state.
    restore_terminal();

    Ok(())
}
