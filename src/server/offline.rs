use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tracing::{info, warn};

use super::session::{emit_final_with_translation, event_error, event_json, ControlCommand};
use crate::engine::ServerRuntime;
use crate::model::ModelFamily;

const MAX_INTERIM_DECODE_SAMPLES: usize = 128_000; // 8s @ 16kHz — enough context for stable partials without excessive re-encoding

/// Adaptive partial interval: scales from 150ms (fast feedback for short audio)
/// up to 500ms (lower CPU for long utterances where re-encoding is expensive).
fn adaptive_partial_interval(buffer_len: usize, sample_rate: i32) -> Duration {
    let secs = buffer_len as f64 / sample_rate as f64;
    let ms = if secs < 2.0 {
        150
    } else if secs < 5.0 {
        250
    } else if secs < 10.0 {
        350
    } else {
        500
    };
    Duration::from_millis(ms)
}

/// Minimum new audio (samples) before we bother with an interim partial decode.
/// ~100ms at 16kHz — below this threshold the result rarely changes.
const MIN_INTERIM_GROWTH_SAMPLES: usize = 1600;

pub(crate) async fn run_offline_session(
    runtime: Arc<ServerRuntime>,
    sample_rate: i32,
    silence_ms: i32,
    mut audio_rx: mpsc::Receiver<Vec<f32>>,
    mut control_rx: mpsc::Receiver<ControlCommand>,
    out_tx: mpsc::Sender<String>,
    speaker_verifier: Option<crate::engine::speaker::SpeakerVerifier>,
) {
    let has_recognizer = runtime
        .config
        .read()
        .map(|c| c.recognizer.is_some())
        .unwrap_or(false);
    if !has_recognizer {
        let _ = out_tx
            .send(event_error(1, "No offline recognizer loaded"))
            .await;
        let _ = out_tx.send(event_json("ended", None)).await;
        return;
    }

    info!("offline session start sampleRate={sample_rate} silenceMs={silence_ms}");
    let _ = out_tx.send(event_json("ready", None)).await;

    reset_vad(&runtime);

    let has_vad = runtime
        .config
        .read()
        .map(|c| c.vad.is_some())
        .unwrap_or(false);
    let interim_partial = runtime
        .config
        .read()
        .map(|c| c.interim_partial)
        .unwrap_or(true);

    let mut buffer: Vec<f32> = Vec::new();
    let mut session_buffer: Vec<f32> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let mut canceled = false;
    let mut last_voice = Instant::now();
    let mut last_partial = Instant::now() - Duration::from_secs(1);
    let mut last_partial_text = String::new();
    let mut last_partial_buffer_len: usize = 0;
    let mut decode_buf: Vec<f32> = Vec::new(); // reusable padded buffer for decode_offline

    loop {
        // Check whether the server is shutting down BEFORE acquiring the
        // read-lock on runtime.config.  Without this bail-out,
        // load_model (which needs the write-lock) can be starved
        // indefinitely by continuous reader acquires on pthread_rwlock.
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

                        buffer.extend_from_slice(&samples);
                        session_buffer.extend_from_slice(&samples);

                        if has_vad {
                            feed_vad(&runtime, &samples);
                            let mut had_segment = false;
                            for seg_samples in pop_vad_segments(&runtime) {
                                had_segment = true;
                                // Speaker verification: skip segments that
                                // don't match the enrolled voice(s).
                                if let Some(ref sv) = speaker_verifier {
                                    if !sv.verify(&seg_samples, sample_rate) {
                                        info!("speaker rejected, dropping segment");
                                        continue;
                                    }
                                }
                                if let Some(text) = decode_offline(&runtime, &seg_samples, sample_rate) {
                                    if !text.is_empty() {
                                        pending.push(text);
                                        let joined = clean_text(&crate::engine::text::join_segments(&pending));
                                        if !joined.is_empty() {
                                            last_partial_text = joined.clone();
                                            let _ = out_tx.send(event_json("partial", Some(&joined))).await;
                                        }
                                    }
                                }
                            }
                            if had_segment {
                                buffer.clear();
                                last_partial_buffer_len = 0;
                                last_voice = Instant::now();
                            } else if interim_partial
                                && vad_detected(&runtime)
                                && !buffer.is_empty()
                                && buffer.len().saturating_sub(last_partial_buffer_len) >= MIN_INTERIM_GROWTH_SAMPLES
                                && last_partial.elapsed() >= adaptive_partial_interval(buffer.len(), sample_rate)
                            {
                                last_partial = Instant::now();
                                last_partial_buffer_len = buffer.len();
                                if let Some(text) = decode_offline_with_buf(&runtime, &buffer, sample_rate, &mut decode_buf) {
                                    if !text.is_empty() {
                                        let joined = clean_text(&crate::engine::text::join_text(&crate::engine::text::join_segments(&pending), &text));
                                        if !joined.is_empty() && joined != last_partial_text {
                                            last_partial_text = joined.clone();
                                            let _ = out_tx.send(event_json("partial", Some(&joined))).await;
                                        }
                                    }
                                }
                            }
                        } else if interim_partial
                            && !session_buffer.is_empty()
                            && session_buffer.len().saturating_sub(last_partial_buffer_len) >= MIN_INTERIM_GROWTH_SAMPLES
                            && last_partial.elapsed() >= adaptive_partial_interval(session_buffer.len(), sample_rate)
                        {
                            last_partial = Instant::now();
                            last_partial_buffer_len = session_buffer.len();
                            let decode_window = if session_buffer.len() > MAX_INTERIM_DECODE_SAMPLES {
                                &session_buffer[session_buffer.len() - MAX_INTERIM_DECODE_SAMPLES..]
                            } else {
                                &session_buffer
                            };
                            if let Some(text) = decode_offline_with_buf(&runtime, decode_window, sample_rate, &mut decode_buf) {
                                if !text.is_empty() && text != last_partial_text {
                                    last_partial_text = text.clone();
                                    let _ = out_tx.send(event_json("partial", Some(&text))).await;
                                }
                            }
                        }

                        if has_vad
                            && !pending.is_empty()
                            && !vad_detected(&runtime)
                            && last_voice.elapsed() >= Duration::from_millis(silence_ms.max(0) as u64)
                        {
                            let _ = emit_final(&out_tx, &mut pending, &mut last_partial_text, &runtime).await;
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

    // Drain remaining audio
    while let Ok(samples) = audio_rx.try_recv() {
        let samples = crate::engine::denoiser::process_audio(
            &runtime,
            &samples,
            sample_rate,
        );
        buffer.extend_from_slice(&samples);
        session_buffer.extend_from_slice(&samples);
        if has_vad {
            feed_vad(&runtime, &samples);
        }
    }

    if !canceled {
        if has_vad {
            flush_vad(&runtime);
            for seg_samples in pop_vad_segments(&runtime) {
                if let Some(ref sv) = speaker_verifier {
                    if !sv.verify(&seg_samples, sample_rate) {
                        info!("speaker rejected tail segment, dropping");
                        continue;
                    }
                }
                if let Some(text) = decode_offline(&runtime, &seg_samples, sample_rate) {
                    if !text.is_empty() {
                        pending.push(text);
                    }
                }
            }
        }

        if !pending.is_empty() {
            let _ = emit_final(&out_tx, &mut pending, &mut last_partial_text, &runtime).await;
        } else if !last_partial_text.is_empty() {
            // Timeout-safe path: if we already emitted partial text during the
            // session, promote it to final immediately so mobile clients don't
            // hit their close timeout waiting for a heavy end decode.
            let text = add_punctuation(&runtime, &last_partial_text);
            tracing::debug!("offline final(from partial)={text:?}");
            let _ = emit_final_with_translation(&runtime, &out_tx, &text).await;
        } else if !session_buffer.is_empty() {
            if let Some(text) = decode_offline(&runtime, &session_buffer, sample_rate) {
                if !text.is_empty() {
                    let text = add_punctuation(&runtime, &text);
                    tracing::debug!("offline final={text:?}");
                    let _ = emit_final_with_translation(&runtime, &out_tx, &text).await;
                }
            }
        }
    }

    let _ = out_tx.send(event_json("ended", None)).await;
}

fn decode_offline_with_buf(
    runtime: &ServerRuntime,
    samples: &[f32],
    sample_rate: i32,
    buf: &mut Vec<f32>,
) -> Option<String> {
    let config = runtime.config.read().ok()?;
    let recognizer = config.recognizer.as_ref()?;
    if samples.is_empty() {
        return None;
    }
    let is_sense_voice = config
        .model_info
        .as_ref()
        .map(|m| m.family == ModelFamily::OfflineSenseVoice)
        .unwrap_or(false);
    let min_samples = config.min_decode_samples;

    let head_pad = if is_sense_voice {
        (sample_rate as usize) / 3
    } else {
        0
    };
    let total_len = (head_pad + samples.len()).max(min_samples);
    buf.resize(total_len, 0.0f32);
    buf[head_pad..head_pad + samples.len()].copy_from_slice(samples);

    let stream = recognizer.create_stream();
    stream.accept_waveform(sample_rate, buf);
    recognizer.decode(&stream);
    let result = stream.get_result()?;
    Some(clean_text(&result.text))
}

// Thin wrapper for places that don't have a reusable buffer (e.g. draining VAD segments).
fn decode_offline(runtime: &ServerRuntime, samples: &[f32], sample_rate: i32) -> Option<String> {
    let mut buf = Vec::new();
    decode_offline_with_buf(runtime, samples, sample_rate, &mut buf)
}

fn clean_text(text: &str) -> String {
    crate::engine::text::clean_text(text)
}

fn reset_vad(runtime: &ServerRuntime) {
    if let Ok(cfg) = runtime.config.read() {
        if let Some(vad) = cfg.vad.as_ref() {
            vad.reset();
        }
    }
}

fn feed_vad(runtime: &ServerRuntime, samples: &[f32]) {
    if let Ok(cfg) = runtime.config.read() {
        if let Some(vad) = cfg.vad.as_ref() {
            vad.accept_waveform(samples);
        }
    }
}

fn flush_vad(runtime: &ServerRuntime) {
    if let Ok(cfg) = runtime.config.read() {
        if let Some(vad) = cfg.vad.as_ref() {
            vad.flush();
        }
    }
}

fn pop_vad_segments(runtime: &ServerRuntime) -> Vec<Vec<f32>> {
    let mut out = Vec::new();
    if let Ok(cfg) = runtime.config.read() {
        if let Some(vad) = cfg.vad.as_ref() {
            while let Some(seg) = vad.front() {
                out.push(seg.samples().to_vec());
                vad.pop();
            }
        }
    }
    out
}

fn vad_detected(runtime: &ServerRuntime) -> bool {
    runtime
        .config
        .read()
        .ok()
        .and_then(|cfg| cfg.vad.as_ref().map(|v| v.detected()))
        .unwrap_or(false)
}

fn add_punctuation(runtime: &ServerRuntime, text: &str) -> String {
    if let Ok(config) = runtime.config.read() {
        if let Some(ref punct) = config.punctuation {
            return punct
                .add_punctuation(text)
                .map(|text| clean_text(&text))
                .unwrap_or_else(|| {
                    warn!("offline punctuation returned no text");
                    text.to_string()
                });
        }
    }
    text.to_string()
}

async fn emit_final(
    out_tx: &mpsc::Sender<String>,
    pending: &mut Vec<String>,
    last_partial_text: &mut String,
    runtime: &ServerRuntime,
) -> Result<(), ()> {
    let text = clean_text(&crate::engine::text::join_segments(pending));
    pending.clear();
    if text.is_empty() {
        return Ok(());
    }
    let text = add_punctuation(runtime, &text);
    *last_partial_text = text.clone();
    tracing::debug!("offline final={text:?}");
    emit_final_with_translation(runtime, out_tx, &text).await
}
