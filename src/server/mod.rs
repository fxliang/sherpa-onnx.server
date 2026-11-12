pub mod offline;
pub mod session;
pub mod streaming;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration};
use tokio_tungstenite::tungstenite::protocol::Message;
use tracing::{error, info, warn};

use crate::engine::ServerRuntime;
use crate::mdns::MdnsRegistration;

const DISCOVERY_MAGIC: &[u8] = b"SHERPA_ONNX_ASR_DISCOVER_V1";
const DISCOVERY_ANNOUNCE_INTERVAL_SECS: u64 = 5;

/// Create a TCP listener with explicit SO_REUSEADDR and SO_REUSEPORT set.
/// This ensures the port can be immediately reused after a server restart,
/// even on platforms (e.g. Android) where the default mio settings may not
/// be sufficient to avoid EADDRINUSE after a stop-start cycle.
fn bind_listener(host: &str, port: u16) -> Result<TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};

    let addr: std::net::SocketAddr = format!("{host}:{port}")
        .parse()
        .with_context(|| format!("invalid address: {host}:{port}"))?;

    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))
        .context("failed to create TCP socket")?;

    // Enable address/port reuse before binding — critical for fast stop-start cycles
    // on platforms that are strict about TIME_WAIT (Android, some Linux kernels).
    socket
        .set_reuse_address(true)
        .context("failed to set SO_REUSEADDR")?;
    #[cfg(unix)]
    socket
        .set_reuse_port(true)
        .context("failed to set SO_REUSEPORT")?;

    socket
        .bind(&addr.into())
        .with_context(|| format!("failed to bind to {addr}"))?;
    socket
        .listen(1024)
        .with_context(|| format!("failed to listen on {addr}"))?;
    socket.set_nonblocking(true)?;

    let std_listener: std::net::TcpListener = socket.into();
    TcpListener::from_std(std_listener).context("failed to create async TcpListener")
}

fn is_addr_in_use(err: &anyhow::Error) -> bool {
    use std::io::ErrorKind;

    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .map(|ioe| {
                if ioe.kind() == ErrorKind::AddrInUse {
                    return true;
                }
                // On Windows, WSAEACCES (10013) is often returned instead of
                // WSAEADDRINUSE (10048) when the port is held by another
                // process — especially when that process uses SO_EXCLUSIVEADDRUSE.
                // Treat it the same as AddrInUse for retry purposes.
                #[cfg(windows)]
                if ioe.raw_os_error() == Some(10013) {
                    return true;
                }
                false
            })
            .unwrap_or(false)
    })
}

pub async fn run_server(
    host: &str,
    port: u16,
    runtime: Arc<ServerRuntime>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    ready: Option<tokio::sync::oneshot::Sender<anyhow::Result<()>>>,
) -> Result<()> {
    let addr = format!("{host}:{port}");
    let mut ready = ready;
    let listener = {
        const MAX_BIND_RETRIES: usize = 15;
        let mut attempt = 0usize;
        loop {
            match bind_listener(host, port) {
                Ok(listener) => break listener,
                Err(e) if is_addr_in_use(&e) && attempt < MAX_BIND_RETRIES => {
                    attempt += 1;
                    let delay_ms = (100 * attempt as u64).min(800);
                    warn!(
                        "bind {addr} failed with EADDRINUSE/WSAEACCES (attempt {attempt}/{MAX_BIND_RETRIES}), retrying in {delay_ms}ms"
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                }
                Err(e) => {
                    if let Some(tx) = ready.take() {
                        let _ = tx.send(Err(anyhow::anyhow!("{e:#}")));
                    }
                    return Err(e);
                }
            }
        }
    };
    if let Some(tx) = ready.take() {
        let _ = tx.send(Ok(()));
    }

    let streaming = runtime.snapshot_streaming();
    let model = runtime.model_name();
    info!(
        "WebSocket ASR server listening on ws://{addr}/v1/asr using {} {model}",
        if streaming { "streaming" } else { "offline" }
    );

    let (_mdns, discovery_task) = if listener.local_addr()?.ip().is_loopback() {
        info!("Service discovery disabled for loopback bind {addr}");
        (None, None)
    } else {
        let mdns = match MdnsRegistration::new(port, Some(&model), streaming) {
            Ok(reg) => Some(reg),
            Err(e) => {
                warn!("mDNS registration failed (service discovery disabled): {e}");
                None
            }
        };
        let discovery_task =
            start_udp_discovery_responder(host, port, model.clone(), streaming).await;
        (mdns, discovery_task)
    };

    // Track connection handler tasks so we can drain them on shutdown.
    // This prevents writer starvation on runtime.config — old session
    // threads hold read locks during decode and would starve the write
    // lock in load_model on restart.
    let mut connections: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, addr)) => {
                        let runtime = runtime.clone();
                        connections.spawn(async move {
                            if let Err(e) = handle_connection(stream, addr, runtime).await {
                                error!("Connection error from {addr}: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        error!("Accept error: {e}");
                    }
                }
            }
            _ = shutdown.changed() => {
                info!("WebSocket server shutting down");
                break;
            }
        }
    }

    // Explicitly drop the listener BEFORE mDNS cleanup.
    // socket2 set SO_REUSEADDR+SO_REUSEPORT, but we still want the port
    // freed as early as possible — the _mdns Drop (MdnsRegistration) calls
    // daemon.shutdown() which can block, so we release the TCP port first.
    drop(listener);

    // Gracefully drain all active connection handlers. This ensures that
    // the session threads release their read-locks on runtime.config before
    // a subsequent load_model tries to acquire the write lock.
    // Without this, std::sync::RwLock (pthread_rwlock on Linux) can starve
    // the writer because it prefers readers by default.
    //
    // We don't abort tasks — we wait for them to drain naturally.
    // tcp connections are already closed because the listener is gone,
    // but existing WebSocket connections may still be draining from the
    // client side. We use a brief timeout to avoid blocking indefinitely.
    let drain_timeout = tokio::time::Duration::from_secs(5);
    loop {
        match tokio::time::timeout(drain_timeout, connections.join_next()).await {
            Ok(Some(_)) => continue, // one connection done, wait for next
            Ok(None) => break,       // all connections drained
            Err(_) => {
                // Timed out — abort remaining connections. Their session
                // threads will see the channel close and exit.
                warn!(
                    "Timed out waiting for {} active connection(s) to drain; aborting",
                    connections.len()
                );
                connections.abort_all();
                while let Some(_) = connections.join_next().await {}
                break;
            }
        }
    }

    // Now _mdns is implicitly dropped (mdns-sd daemon shutdown), which may
    // block briefly; that's fine since the port is already free for rebind.
    if let Some(task) = discovery_task {
        task.abort();
        let _ = task.await;
    }
    drop(_mdns);

    Ok(())
}

async fn start_udp_discovery_responder(
    host: &str,
    port: u16,
    model: String,
    streaming: bool,
) -> Option<JoinHandle<()>> {
    let socket = match UdpSocket::bind(("0.0.0.0", port)).await {
        Ok(socket) => socket,
        Err(e) => {
            warn!("UDP discovery disabled: failed to bind 0.0.0.0:{port}: {e}");
            return None;
        }
    };
    if let Err(e) = socket.set_broadcast(true) {
        warn!("UDP discovery broadcast disabled: {e}");
    }
    info!("UDP discovery listening on udp://0.0.0.0:{port} for host {host}");
    Some(tokio::spawn(async move {
        let mut buf = [0u8; 256];
        let mut announce = time::interval(Duration::from_secs(DISCOVERY_ANNOUNCE_INTERVAL_SECS));
        let payload = serde_json::json!({
            "name": "sherpa-onnx",
            "port": port,
            "path": "/v1/asr",
            "mode": if streaming { "streaming" } else { "offline" },
            "model": model,
        })
        .to_string();
        loop {
            tokio::select! {
                _ = announce.tick() => {
                    let broadcast = format!("255.255.255.255:{port}");
                    if let Err(e) = socket.send_to(payload.as_bytes(), &broadcast).await {
                        tracing::debug!("UDP discovery broadcast failed: {e}");
                    }
                }
                result = socket.recv_from(&mut buf) => {
                    let (len, peer) = match result {
                        Ok(v) => v,
                        Err(e) => {
                            warn!("UDP discovery recv failed: {e}");
                            break;
                        }
                    };
                    if &buf[..len] != DISCOVERY_MAGIC {
                        continue;
                    }
                    if let Err(e) = socket.send_to(payload.as_bytes(), peer).await {
                        warn!("UDP discovery reply to {peer} failed: {e}");
                    }
                }
            }
        }
    }))
}

async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    runtime: Arc<ServerRuntime>,
) -> Result<()> {
    let mut ws_stream = tokio_tungstenite::accept_async(stream).await?;
    info!("New WebSocket connection from {addr}");

    // Channel for session worker to send text frames.
    // The session runs in its own std::thread, so we use mpsc to bridge
    // into this async task.  We forward them inline via tokio::select!
    // instead of spawning a forward task — this matches the Python server's
    // architecture and avoids split-stream half-close races.
    let (out_tx, mut out_rx) = mpsc::channel::<String>(256);

    let mut active_session: Option<session::SessionHandle> = None;

    loop {
        tokio::select! {
            // ── receive from WebSocket ──────────────────────────────
            msg = ws_stream.next() => {
                let msg = match msg {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        info!("WebSocket read error from {addr}: {e}");
                        break;
                    }
                    None => break,
                };

                match msg {
                    Message::Binary(data) => {
                        if let Some(ref session) = active_session {
                            let samples = bytes_to_f32_le(&data);
                            session.feed(samples).await;
                        }
                    }
                    Message::Text(text) => {
                        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&text);
                        let msg = match parsed {
                            Ok(m) => m,
                            Err(_) => {
                                let _ = out_tx.send(
                                    session::event_error(3, "Invalid JSON control frame")
                                ).await;
                                continue;
                            }
                        };

                        let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        info!("Control message from {addr}: {msg_type}");

                        match msg_type {
                            "start" => {
                                if let Some(ref session) = active_session {
                                    session.cancel().await;
                                }
                                let sample_rate = msg.get("sampleRate")
                                    .and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                                let config_silence_ms = runtime.config.read()
                                    .map(|c| c.silence_ms).unwrap_or(3000);
                                let silence_ms = msg.get("silenceMs")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(config_silence_ms as i64) as i32;

                                // Optional speaker verification data: embeddings + threshold
                                // sent by the Android client when a voice-print is enrolled.
                                let speaker_embeddings: Option<Vec<Vec<f32>>> = msg.get("speakerEmbeddings")
                                    .and_then(|v| v.as_array())
                                    .map(|embs| {
                                        embs.iter()
                                            .filter_map(|e| e.as_array())
                                            .map(|arr| {
                                                arr.iter()
                                                    .filter_map(|v| v.as_f64().map(|f| f as f32))
                                                    .collect()
                                            })
                                            .collect()
                                    });
                                let speaker_threshold: f32 = msg.get("speakerThreshold")
                                    .and_then(|v| v.as_f64())
                                    .map(|v| v as f32)
                                    .unwrap_or(0.0);
                                let speaker_model_name: Option<String> = msg.get("speakerModelName")
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.is_empty())
                                    .map(|s| s.to_string());

                                // Validate that the server has a compatible speaker model.
                                if let (Some(ref embs), Some(ref name)) = (&speaker_embeddings, &speaker_model_name) {
                                    let matched = runtime.config.read().ok()
                                        .and_then(|c| c.speaker_model.clone())
                                        .map(|path| path.contains(name.as_str()))
                                        .unwrap_or(false);
                                    if !matched {
                                        tracing::warn!(
                                            "WS start: speaker model '{name}' not found on server — \
                                             speaker verification disabled for this session"
                                        );
                                    } else {
                                        info!(
                                            "WS start: speaker verification enabled — \
                                             {} embedding(s), threshold={:.3}, model={name}",
                                            embs.len(),
                                            speaker_threshold,
                                        );
                                    }
                                } else if speaker_embeddings.is_some() {
                                    info!(
                                        "WS start: speaker verification enabled — \
                                         {} embedding(s), threshold={:.3}",
                                        speaker_embeddings.as_ref().map_or(0, |e| e.len()),
                                        speaker_threshold,
                                    );
                                }

                                let tx = out_tx.clone();
                                match session::start_session(
                                    &runtime,
                                    sample_rate,
                                    silence_ms,
                                    tx,
                                    speaker_embeddings,
                                    speaker_threshold,
                                    speaker_model_name,
                                ) {
                                    Ok(h) => { active_session = Some(h); }
                                    Err(e) => {
                                        let _ = out_tx.send(
                                            session::event_error(2, &e.to_string())
                                        ).await;
                                        let _ = out_tx.send(
                                            session::event_json("ended", None)
                                        ).await;
                                    }
                                }
                            }
                            "end" => {
                                if let Some(ref session) = active_session {
                                    session.finish().await;
                                }
                            }
                            "cancel" | "stop" => {
                                if let Some(ref session) = active_session {
                                    session.cancel().await;
                                }
                            }
                            _ => {
                                let _ = out_tx.send(
                                    session::event_error(
                                        0,
                                        &format!("Unknown control frame: {msg_type}"),
                                    )
                                ).await;
                            }
                        }
                    }
                    Message::Close(_) => {
                        // Tungstenite queues the Close reply while reading it.
                        // Flush before dropping the connection, with a bounded wait.
                        match time::timeout(Duration::from_secs(5), ws_stream.flush()).await {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => tracing::debug!("WebSocket close reply to {addr} failed: {e}"),
                            Err(_) => tracing::debug!("WebSocket close reply to {addr} timed out"),
                        }
                        break;
                    }
                    _ => {}
                }
            }

            // ── forward session output to WebSocket ─────────────────
            out = out_rx.recv() => {
                match out {
                    Some(text) => {
                        if ws_stream.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    None => break, // channel closed
                }
            }
        }
    }

    // Cancel any active session (may be already finished).
    if let Some(session) = active_session.take() {
        session.cancel().await;
    }

    // Stop session output channel and drop pending messages.
    // Do not send text frames during/after close path.
    drop(out_tx);
    while out_rx.try_recv().is_ok() {}

    info!("Connection closed from {addr}");
    Ok(())
}

fn bytes_to_f32_le(data: &[u8]) -> Vec<f32> {
    data.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn client_close_receives_matching_close_reply() {
        use tokio_tungstenite::tungstenite::protocol::{frame::coding::CloseCode, CloseFrame};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, peer) = listener.accept().await.unwrap();
            handle_connection(stream, peer, Arc::new(ServerRuntime::new())).await.unwrap();
        });
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/v1/asr"))
            .await.unwrap();
        let close = CloseFrame { code: CloseCode::Normal, reason: "session ended".into() };
        ws.close(Some(close.clone())).await.unwrap();
        let reply = time::timeout(Duration::from_secs(5), ws.next()).await.unwrap();
        assert_eq!(reply.unwrap().unwrap(), Message::Close(Some(close)));
        time::timeout(Duration::from_secs(5), server).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn loopback_server_accepts_websockets_without_binding_udp_discovery() {
        for host in ["127.0.0.1", "127.0.0.2", "[::1]"] {
            // TCP and UDP have independent port allocations.
            let listener = (0..100).find_map(|_| {
                let listener = bind_listener(host, 0).unwrap();
                std::net::UdpSocket::bind(("0.0.0.0", listener.local_addr().unwrap().port()))
                    .ok()
                    .map(|_| listener)
            }).expect("no port available for both TCP and UDP");
            let port = listener.local_addr().unwrap().port();
            drop(listener);

            let runtime = Arc::new(ServerRuntime::new());
            let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(run_server(
                host,
                port,
                runtime,
                shutdown_rx,
                Some(ready_tx),
            ));
            time::timeout(Duration::from_secs(5), ready_rx)
                .await
                .unwrap()
                .unwrap()
                .unwrap();

            // A completed handshake ensures discovery startup has been reached.
            let (mut ws, _) = time::timeout(
                Duration::from_secs(5),
                tokio_tungstenite::connect_async(format!("ws://{host}:{port}/v1/asr")),
            )
            .await
            .unwrap()
            .unwrap();
            let udp = UdpSocket::bind(("0.0.0.0", port)).await;
            ws.close(None).await.unwrap();
            shutdown_tx.send(true).unwrap();
            time::timeout(Duration::from_secs(10), server)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(udp.is_ok(), "loopback server bound UDP discovery for {host}");
        }
    }
}
