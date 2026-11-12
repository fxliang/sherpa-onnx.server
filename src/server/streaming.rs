use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::sync::mpsc;
use tracing::{info, warn};

use super::session::{emit_final_with_translation, event_error, event_json, ControlCommand};
use crate::engine::ServerRuntime;

pub(crate) async fn run_streaming_session(
    runtime: Arc<ServerRuntime>,
    sample_rate: i32,
    mut audio_rx: mpsc::Receiver<Vec<f32>>,
    mut control_rx: mpsc::Receiver<ControlCommand>,
    out_tx: mpsc::Sender<String>,
) {
    let stream = {
        let config = match runtime.config.read() {
            Ok(c) => c,
            Err(_) => {
                let _ = out_tx.send(event_error(1, "Runtime lock error")).await;
                let _ = out_tx.send(event_json("ended", None)).await;
                return;
            }
        };
        match config.online_recognizer.as_ref() {
            Some(r) => r.create_stream(),
            None => {
                let _ = out_tx
                    .send(event_error(1, "No online recognizer loaded"))
                    .await;
                let _ = out_tx.send(event_json("ended", None)).await;
                return;
            }
        }
    };

    info!("streaming session start sampleRate={sample_rate}");
    let _ = out_tx.send(event_json("ready", None)).await;

    let mut canceled = false;

    loop {
        // Check whether the server is shutting down BEFORE acquiring the
        // read-lock on runtime.config.  Without this bail-out,
        // load_model (which needs the write-lock) can be starved
        // indefinitely by continuous reader acquires.
        if runtime.shutting_down.load(Ordering::Acquire) {
            break;
        }

        tokio::select! {
            chunk = audio_rx.recv() => {
                match chunk {
                    Some(samples) => {
                        if samples.is_empty() { continue; }
                        let samples = crate::engine::denoiser::process_audio(
                            &runtime,
                            &samples,
                            sample_rate,
                        );

                        // Bail out before acquiring the read-lock if the
                        // server is shutting down, to prevent writer starvation.
                        if runtime.shutting_down.load(Ordering::Acquire) {
                            break;
                        }

                        let (partial_text, final_raw, is_endpoint) = {
                            let config = match runtime.config.read() {
                                Ok(c) => c,
                                Err(_) => break,
                            };
                            let recognizer: &sherpa_onnx::OnlineRecognizer = match config.online_recognizer.as_ref() {
                                Some(r) => r,
                                None => break,
                            };

                            stream.accept_waveform(sample_rate, &samples);
                            while recognizer.is_ready(&stream) {
                                recognizer.decode(&stream);
                            }

                            let partial_text = if config.interim_partial {
                                get_text(recognizer, &stream)
                            } else {
                                String::new()
                            };

                            let is_endpoint = recognizer.is_endpoint(&stream);
                            let final_raw = if is_endpoint {
                                get_text(recognizer, &stream)
                            } else {
                                String::new()
                            };

                            (partial_text, final_raw, is_endpoint)
                        }; // config read guard dropped here

                        if !partial_text.is_empty() {
                            let _ = out_tx.send(event_json("partial", Some(&partial_text))).await;
                        }

                        if is_endpoint && !final_raw.is_empty() {
                            let text = add_punctuation(&runtime, &final_raw);
                            tracing::debug!("streaming final={text:?}");
                            let _ = emit_final_with_translation(&runtime, &out_tx, &text).await;
                            // reset requires read lock; OK as a separate acquisition
                            if let Ok(config) = runtime.config.read() {
                                if let Some(ref recognizer) = config.online_recognizer {
                                    recognizer.reset(&stream);
                                }
                            }
                        }
                    }
                    None => break,
                }
            }
            cmd = control_rx.recv() => {
                match cmd {
                    Some(ControlCommand::Finish) => break,
                    Some(ControlCommand::Cancel) => {
                        canceled = true;
                        break;
                    }
                    None => break,
                }
            }
        }
    }

    // Drain remaining audio — use a short timeout so we catch any audio
    // that is still in-flight from the WebSocket handler.
    loop {
        match tokio::time::timeout(Duration::from_millis(100), audio_rx.recv()).await {
            Ok(Some(samples)) => {
                if samples.is_empty() {
                    continue;
                }
                let samples = crate::engine::denoiser::process_audio(
                    &runtime,
                    &samples,
                    sample_rate,
                );
                if let Ok(config) = runtime.config.read() {
                    if let Some(ref recognizer) = config.online_recognizer {
                        stream.accept_waveform(sample_rate, &samples);
                        while recognizer.is_ready(&stream) {
                            recognizer.decode(&stream);
                        }
                    }
                }
            }
            Ok(None) | Err(_) => break,
        }
    }

    if !canceled {
        // Pad tail with silence to ensure the last partial decode chunk
        // (less than decode_chunk_len frames) is not silently discarded.
        // zipformer2 models need at least 16 frames per decode step;
        // at 10ms/frame → ~160ms → ~2560 samples at 16kHz.
        let pad_samples = (sample_rate as usize).max(16000); // ~1s silence, enough for 2-3 decode steps
        let silence = vec![0.0f32; pad_samples];
        if let Ok(config) = runtime.config.read() {
            if let Some(ref recognizer) = config.online_recognizer {
                stream.accept_waveform(sample_rate, &silence);
                while recognizer.is_ready(&stream) {
                    recognizer.decode(&stream);
                }
            }
        }

        let final_raw = {
            if let Ok(config) = runtime.config.read() {
                if let Some(ref recognizer) = config.online_recognizer {
                    stream.input_finished();
                    while recognizer.is_ready(&stream) {
                        recognizer.decode(&stream);
                    }
                    get_text(recognizer, &stream)
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        }; // config read guard dropped here

        if !final_raw.is_empty() {
            let text = add_punctuation(&runtime, &final_raw);
            tracing::debug!("streaming final={text:?}");
            let _ = emit_final_with_translation(&runtime, &out_tx, &text).await;
        }
    }

    let _ = out_tx.send(event_json("ended", None)).await;
}

fn get_text(
    recognizer: &sherpa_onnx::OnlineRecognizer,
    stream: &sherpa_onnx::OnlineStream,
) -> String {
    recognizer
        .get_result(stream)
        .map(|r| crate::engine::text::clean_text(&r.text))
        .unwrap_or_default()
}

fn clean_text(text: &str) -> String {
    crate::engine::text::clean_text(text)
}

fn add_punctuation(runtime: &ServerRuntime, text: &str) -> String {
    if let Ok(config) = runtime.config.read() {
        if let Some(ref punct) = config.punctuation {
            return punct
                .add_punctuation(text)
                .map(|punctuated| {
                    let punctuated = clean_text(&punctuated);
                    tracing::debug!("streaming punctuation raw={text:?} punctuated={punctuated:?}");
                    punctuated
                })
                .unwrap_or_else(|| {
                    warn!("streaming punctuation returned no text");
                    text.to_string()
                });
        }
        warn!("streaming final has no punctuation model loaded");
    }
    text.to_string()
}
