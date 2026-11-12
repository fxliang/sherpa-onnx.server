use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{bail, Result};
use tokio::sync::mpsc;

use crate::engine::ServerRuntime;

pub struct SessionHandle {
    audio_tx: mpsc::Sender<Vec<f32>>,
    control_tx: mpsc::Sender<ControlCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlCommand {
    Finish,
    Cancel,
}

pub(crate) fn event_json(event_type: &str, text: Option<&str>) -> String {
    let mut map = serde_json::Map::new();
    map.insert(
        "type".to_string(),
        serde_json::Value::String(event_type.to_string()),
    );
    if let Some(text) = text {
        map.insert(
            "text".to_string(),
            serde_json::Value::String(text.to_string()),
        );
    }
    serde_json::Value::Object(map).to_string()
}

pub(crate) fn event_final(text: &str, translated: bool) -> String {
    serde_json::json!({
        "type": "final",
        "text": text,
        "translated": translated,
    })
    .to_string()
}

pub(crate) fn event_error(code: i32, message: &str) -> String {
    serde_json::json!({
        "type": "error",
        "code": code,
        "message": message,
    })
    .to_string()
}

pub(crate) async fn emit_final_with_translation(
    runtime: &ServerRuntime,
    out_tx: &mpsc::Sender<String>,
    text: &str,
) -> Result<(), ()> {
    let engine = runtime
        .config
        .read()
        .ok()
        .and_then(|cfg| cfg.translation.clone());

    let mut translated_by_server = false;
    let final_text = match engine {
        Some(ref engine) if engine.is_enabled() => {
            // Emit the ASR text as a final partial before translating, so the
            // client can show the recognized text to the user.
            let _ = out_tx
                .send(event_json("partial", Some(text)))
                .await;

            match engine.translate(text).await {
                Ok(Some(translated)) => {
                    tracing::debug!("translation result: {translated:?}");
                    translated_by_server = true;
                    translated
                }
                Ok(None) => {
                    tracing::info!("translation skipped (no trigger match or empty output), using original text");
                    text.to_string()
                }
                Err(e) => {
                    tracing::error!("translation failed: {e}");
                    let _ = out_tx
                        .send(event_error(4, &format!("Translation failed: {e}")))
                        .await;
                    text.to_string()
                }
            }
        }
        _ => {
            if engine.is_some() {
                tracing::debug!("translation engine present but not enabled, using original text");
            } else {
                tracing::debug!("no translation engine configured, using original text");
            }
            text.to_string()
        }
    };

    out_tx
        .send(event_final(&final_text, translated_by_server))
        .await
        .map_err(|_| ())?;

    Ok(())
}

pub(crate) fn start_session(
    runtime: &Arc<ServerRuntime>,
    sample_rate: i32,
    silence_ms: i32,
    out_tx: mpsc::Sender<String>,
    speaker_embeddings: Option<Vec<Vec<f32>>>,
    speaker_threshold: f32,
    speaker_model_name: Option<String>,
) -> Result<SessionHandle> {
    // Bail out if the server is shutting down / restarting to prevent
    // new read-locks from starving the write-lock in load_model.
    if runtime.shutting_down.load(Ordering::Acquire) {
        bail!("server is shutting down, refusing new session");
    }

    let streaming = runtime.snapshot_streaming();
    let rt_sample_rate = runtime.snapshot_sample_rate();
    let actual_sample_rate = if sample_rate > 0 {
        sample_rate
    } else {
        rt_sample_rate
    };
    let actual_silence_ms = if silence_ms > 0 { silence_ms } else { 3000 };

    // Warn about sample rate mismatch, but don't reject — the client may
    // expect results even at non-optimal rates.
    if sample_rate > 0 && sample_rate != rt_sample_rate {
        tracing::warn!(
            "client requested sampleRate={sample_rate}, model expects {rt_sample_rate}; \
             recognition quality may be degraded"
        );
    }

    // Snapshot the shared speaker extractor (if any) plus validate that the
    // server's speaker model name matches what the client sent (if provided).
    // The extractor is loaded once at model-load time and shared across
    // sessions, so nothing is read from disk here.
    let speaker_extractor = speaker_embeddings
        .as_ref()
        .filter(|e| !e.is_empty() && speaker_threshold > 0.0)
        .and_then(|_| {
            let cfg = runtime.config.read().ok()?;
            let extractor = cfg.speaker_extractor.clone()?;
            // If the client sent a model name, verify the server's model
            // path contains it (loose match — filename substring).
            if let Some(ref name) = speaker_model_name {
                let path = cfg.speaker_model.clone().unwrap_or_default();
                if !path.contains(name.as_str()) {
                    tracing::warn!(
                        "speaker model mismatch: client='{name}', server='{}' — \
                         speaker verification disabled for this session",
                        crate::path_display::for_log_path(std::path::Path::new(&path))
                    );
                    return None;
                }
            }
            Some(extractor)
        });

    let (audio_tx, audio_rx) = mpsc::channel::<Vec<f32>>(512);
    let (control_tx, control_rx) = mpsc::channel::<ControlCommand>(16);

    let runtime = runtime.clone();
    let embs = speaker_embeddings;

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        // Build the per-session verifier around the shared extractor. No model
        // is loaded here; only this session's reference embeddings + threshold
        // are stored.
        let speaker_verifier = embs
            .zip(speaker_extractor)
            .map(|(embeddings, extractor)| {
                crate::engine::speaker::SpeakerVerifier::from_shared(
                    extractor,
                    embeddings,
                    speaker_threshold,
                )
            });

        // Redirect native stderr → tracing so sherpa-onnx library
        // warnings (e.g. sample rate mismatch) don't corrupt the TUI.
        if streaming {
            crate::log::capture_native_stderr(|| {
                rt.block_on(super::streaming::run_streaming_session(
                    runtime,
                    actual_sample_rate,
                    audio_rx,
                    control_rx,
                    out_tx,
                ));
            });
        } else {
            crate::log::capture_native_stderr(|| {
                rt.block_on(super::offline::run_offline_session(
                    runtime,
                    actual_sample_rate,
                    actual_silence_ms,
                    audio_rx,
                    control_rx,
                    out_tx,
                    speaker_verifier,
                ));
            });
        }
    });

    Ok(SessionHandle {
        audio_tx,
        control_tx,
    })
}

impl SessionHandle {
    pub async fn feed(&self, samples: Vec<f32>) {
        let _ = self.audio_tx.send(samples).await;
    }

    pub async fn finish(&self) {
        let _ = self.control_tx.send(ControlCommand::Finish).await;
    }

    pub async fn cancel(&self) {
        let _ = self.control_tx.send(ControlCommand::Cancel).await;
    }
}
