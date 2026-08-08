//! Async Mixing Station WebSocket engine. A single `tokio::spawn`ed task that connects,
//! auto-reconnects on a fixed delay, and bridges inbound/outbound messages through
//! channels -- no dedicated OS thread needed (unlike `midi_io.rs`) since
//! `tokio-tungstenite` is async-native.
//!
//! This engine owns connection lifecycle only -- NOT the on-open business logic
//! (console-information fetch, subscriptions, handshake) or the "should I even be
//! running" decision (that tracks bridge lifecycle in `bridge-cli`, a Plan 2d concern).
//! Once spawned, it always tries to stay connected until told `WsCommand::Shutdown`.
//!
//! No stale-connection-close-event guard is needed here (unlike JS's `connectMixingStationWebSocket`,
//! which compares `socket !== msWebSocket` to ignore a superseded connection's late close
//! event) -- this engine has exactly one connection attempt in flight at a time by
//! construction (a single sequential loop), which structurally rules out that bug class
//! rather than needing a runtime guard for it.
//!
//! A periodic keep-alive heartbeat (see `WS_HEARTBEAT_INTERVAL`) is scoped here too -- it's a
//! connection-lifecycle concern (preventing an idle timeout), not business logic, so it doesn't
//! cross the boundary this module otherwise holds.
//!
//! Port of the connection-lifecycle half of `index.js`'s `connectMixingStationWebSocket`/
//! `sendToMixingStationWS`.

use crate::value_coercion::coerce_ws_payload_to_text;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};

/// Fixed delay between a disconnect and the next connect attempt, matching JS's
/// `delay = 2000` in `connectMixingStationWebSocket`.
pub const WS_RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_millis(2000);

/// Interval between keep-alive pings to Mixing Station while connected, matching JS's
/// `wsHeartbeatInterval` (`connectMixingStationWebSocket`'s `open` handler). Working theory:
/// without this, Mixing Station's own server idle-times-out the connection and closes it --
/// this port never had a heartbeat at all until this commit. Not yet confirmed against a real
/// Mixing Station instance; if the pre-fix reported disconnect cycle recurs at a roughly fixed
/// period even with this heartbeat running, the actual cause is something else.
pub const WS_HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(4000);

/// Wire payload for the heartbeat, matching `sendToMixingStationWS`'s JSON.stringify output
/// for `{path: "/hi/v", method: "GET"}` byte-for-byte.
const WS_HEARTBEAT_FRAME: &str = r#"{"path":"/hi/v","method":"GET"}"#;

/// A command to the running WebSocket engine task.
pub enum WsCommand {
    /// Send a text frame to Mixing Station. Dropped (not buffered) if the engine isn't
    /// currently connected — see `wait_for_reconnect_or_shutdown` for why that's correct.
    Send(String),
    /// Close the connection cleanly and stop the engine task.
    Shutdown,
}

/// An event emitted by the engine on its inbound channel.
#[derive(Debug, Clone, PartialEq)]
pub enum WsEvent {
    /// A connection was established. Plan 2d's cue to run its own on-open logic
    /// (console-information fetch, layout rebuild, subscriptions, handshake).
    Connected,
    /// An inbound text frame (binary frames are decoded to text first).
    Message(String),
    /// The connection was lost, or a connect attempt failed. The engine will retry after
    /// `reconnect_delay` on its own — this is informational, not a prompt to respawn it.
    Disconnected,
}

/// Handle to the running WebSocket engine — holds the outbound command sender, the inbound
/// event receiver, and the task join handle. Send [`WsCommand::Shutdown`] and then `.await`
/// `join_handle` to stop it cleanly — this works correctly whether the engine is mid-connect,
/// connected, or waiting out a reconnect delay. Simply dropping this handle without sending
/// `Shutdown` first also works (dropping `command_tx` makes the engine's `command_rx.recv()`
/// return `None`, which every receive site treats as a stop signal), but doesn't guarantee the
/// task has fully exited before your own code continues.
pub struct WsEngineHandle {
    /// Sends commands to the engine. Unbounded, but bounded in practice: commands sent while
    /// disconnected are dropped rather than queued, so the only growth vector is a send rate
    /// outpacing the socket's flush rate while connected — not a realistic risk at this
    /// bridge's message volumes.
    pub command_tx: mpsc::UnboundedSender<WsCommand>,
    /// Receives inbound events. Unbounded for the same reason as `command_tx`.
    pub events: mpsc::UnboundedReceiver<WsEvent>,
    /// The engine's `tokio` task. Stop it with `.await` (it's a [`tokio::task::JoinHandle`],
    /// not a `std::thread::JoinHandle` — there is no `.join()` to call).
    pub join_handle: tokio::task::JoinHandle<()>,
}

/// Spawn the WebSocket engine: connect to `url`, emit [`WsEvent`]s for everything that happens
/// on the connection, and reconnect after `reconnect_delay` whenever it drops — forever, until
/// told [`WsCommand::Shutdown`]. Returns as soon as the task is spawned (the first connect
/// attempt happens inside the task, so this never blocks on the network).
pub fn spawn_ws_engine(url: String, reconnect_delay: std::time::Duration) -> WsEngineHandle {
    spawn_ws_engine_with_heartbeat(url, reconnect_delay, WS_HEARTBEAT_INTERVAL)
}

/// Same as [`spawn_ws_engine`], but with an explicit heartbeat interval -- exists so tests can
/// use a short interval instead of waiting out the real 4-second production value. Production
/// code should always call [`spawn_ws_engine`].
fn spawn_ws_engine_with_heartbeat(
    url: String,
    reconnect_delay: std::time::Duration,
    heartbeat_interval: std::time::Duration,
) -> WsEngineHandle {
    let (command_tx, mut command_rx) = mpsc::unbounded_channel::<WsCommand>();
    let (event_tx, event_rx) = mpsc::unbounded_channel::<WsEvent>();

    let join_handle = tokio::spawn(async move {
        loop {
            let connect_fut = tokio_tungstenite::connect_async(&url);
            tokio::pin!(connect_fut);

            let connect_result = loop {
                tokio::select! {
                    result = &mut connect_fut => break Some(result),
                    cmd = command_rx.recv() => {
                        match cmd {
                            Some(WsCommand::Shutdown) | None => break None,
                            Some(WsCommand::Send(_)) => continue, // dropped: not connected yet
                        }
                    }
                }
            };

            let Some(connect_result) = connect_result else {
                return; // Shutdown (or sender dropped) during initial connect attempt.
            };

            let ws_stream = match connect_result {
                Ok((stream, _response)) => stream,
                Err(e) => {
                    eprintln!("Mixing Station WebSocket connect failed: {e}");
                    let _ = event_tx.send(WsEvent::Disconnected);
                    if !wait_for_reconnect_or_shutdown(&mut command_rx, reconnect_delay).await {
                        return;
                    }
                    continue;
                }
            };

            let _ = event_tx.send(WsEvent::Connected);
            let (mut write, mut read) = ws_stream.split();
            let mut shutdown_requested = false;
            let mut heartbeat = tokio::time::interval_at(
                tokio::time::Instant::now() + heartbeat_interval,
                heartbeat_interval,
            );
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            loop {
                tokio::select! {
                    cmd = command_rx.recv() => {
                        match cmd {
                            Some(WsCommand::Send(text)) => {
                                // Awaiting the write parks this whole select!, so inbound reads
                                // aren't polled until it completes. Harmless here: TCP buffers the
                                // backlog and this bridge's message volumes are tiny.
                                if write.send(Message::Text(text.into())).await.is_err() {
                                    break;
                                }
                            }
                            Some(WsCommand::Shutdown) | None => {
                                let _ = write.close().await;
                                shutdown_requested = true;
                                break;
                            }
                        }
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                let _ = event_tx.send(WsEvent::Message(text.to_string()));
                            }
                            Some(Ok(Message::Binary(data))) => {
                                let text = coerce_ws_payload_to_text(&data);
                                let _ = event_tx.send(WsEvent::Message(text));
                            }
                            Some(Ok(Message::Close(_))) => {
                                // tungstenite only *queues* the reply close frame when it reads a
                                // peer close; it's flushed on the next I/O call. Without this we'd
                                // drop the TCP connection mid-handshake (RFC 6455 s7.1.1), making a
                                // clean server-side close look identical to an abnormal one.
                                let _ = write.close().await;
                                break;
                            }
                            None => break,
                            Some(Ok(_)) => {} // ping/pong/frame -- tungstenite handles these internally
                            Some(Err(e)) => {
                                eprintln!("Mixing Station WebSocket read error: {e}");
                                break;
                            }
                        }
                    }
                    // Port of JS's `wsHeartbeatInterval` (`connectMixingStationWebSocket`'s
                    // `open` handler) -- keeps Mixing Station's own server from idle-timing-out
                    // the connection. The reply (if any) is intentionally ignored, matching
                    // `sendToMixingStationWS` -- it just arrives as an ordinary Message event
                    // above like any other inbound frame.
                    _ = heartbeat.tick() => {
                        if write.send(Message::Text(Utf8Bytes::from_static(WS_HEARTBEAT_FRAME))).await.is_err() {
                            break;
                        }
                    }
                }
            }

            if shutdown_requested {
                return;
            }

            let _ = event_tx.send(WsEvent::Disconnected);
            if !wait_for_reconnect_or_shutdown(&mut command_rx, reconnect_delay).await {
                return;
            }
        }
    });

    WsEngineHandle {
        command_tx,
        events: event_rx,
        join_handle,
    }
}

/// Waits `reconnect_delay` before the next connect attempt, unless `Shutdown` arrives
/// first. Any `Send` command received during the wait is dropped (matches
/// `sendToMixingStationWS`'s own documented "dropped if not connected" behavior -- see
/// this module's doc comment). Returns `false` if the caller should stop entirely
/// (Shutdown received or the command channel closed), `true` to proceed with the retry.
async fn wait_for_reconnect_or_shutdown(
    command_rx: &mut mpsc::UnboundedReceiver<WsCommand>,
    reconnect_delay: std::time::Duration,
) -> bool {
    let sleep = tokio::time::sleep(reconnect_delay);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            _ = &mut sleep => return true,
            cmd = command_rx.recv() => {
                match cmd {
                    Some(WsCommand::Shutdown) | None => return false,
                    Some(WsCommand::Send(_)) => continue, // dropped: still not connected
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::net::TcpListener;

    async fn spawn_echo_server() -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                if let Ok(ws) = tokio_tungstenite::accept_async(stream).await {
                    let (mut write, mut read) = futures_util::StreamExt::split(ws);
                    while let Some(Ok(msg)) = futures_util::StreamExt::next(&mut read).await {
                        if msg.is_close() {
                            break;
                        }
                        if futures_util::SinkExt::send(&mut write, msg).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });
        (format!("ws://{addr}"), handle)
    }

    #[tokio::test]
    async fn connects_and_emits_connected_event() {
        let (url, _server) = spawn_echo_server().await;
        let mut handle = spawn_ws_engine(url, Duration::from_millis(50));
        let event = tokio::time::timeout(Duration::from_secs(2), handle.events.recv())
            .await
            .expect("should not time out")
            .expect("channel should be open");
        assert!(matches!(event, WsEvent::Connected));
        handle.command_tx.send(WsCommand::Shutdown).unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle.join_handle).await;
    }

    #[tokio::test]
    async fn send_command_is_echoed_back_as_a_message_event() {
        let (url, _server) = spawn_echo_server().await;
        let mut handle = spawn_ws_engine(url, Duration::from_millis(50));
        let _connected = handle.events.recv().await.unwrap();

        handle
            .command_tx
            .send(WsCommand::Send("hello".to_string()))
            .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), handle.events.recv())
            .await
            .expect("should not time out")
            .expect("channel should be open");
        assert_eq!(event, WsEvent::Message("hello".to_string()));

        handle.command_tx.send(WsCommand::Shutdown).unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle.join_handle).await;
    }

    #[tokio::test]
    async fn sends_a_heartbeat_on_the_configured_interval() {
        let (url, _server) = spawn_echo_server().await;
        let heartbeat_interval = Duration::from_millis(50);
        let mut handle =
            spawn_ws_engine_with_heartbeat(url, heartbeat_interval, heartbeat_interval);
        let _connected = handle.events.recv().await.unwrap();

        // The echo server bounces back whatever it receives, so heartbeat frames arrive on our
        // own event channel as ordinary Message events -- exactly like Mixing Station's real
        // (ignored) replies would. Observing two of them, with real elapsed time between them,
        // proves the heartbeat is periodic rather than a one-shot fire.
        let first_seen_at = std::time::Instant::now();
        let first = tokio::time::timeout(Duration::from_secs(2), handle.events.recv())
            .await
            .expect("should not time out waiting for the first heartbeat to round-trip")
            .expect("channel should be open");
        assert_eq!(first, WsEvent::Message(WS_HEARTBEAT_FRAME.to_string()));
        assert!(
            first_seen_at.elapsed() >= heartbeat_interval,
            "the first heartbeat must not fire immediately -- tokio's plain `interval()` does, `interval_at` with a delayed start must not"
        );

        let second = tokio::time::timeout(Duration::from_secs(2), handle.events.recv())
            .await
            .expect("should not time out waiting for a second heartbeat")
            .expect("channel should be open");
        assert_eq!(second, WsEvent::Message(WS_HEARTBEAT_FRAME.to_string()));
        assert!(
            first_seen_at.elapsed() >= heartbeat_interval * 2,
            "a second heartbeat only counts as proving periodicity if real time elapsed since the first"
        );

        handle.command_tx.send(WsCommand::Shutdown).unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle.join_handle).await;
    }

    #[tokio::test]
    async fn shutdown_during_initial_connect_returns_promptly() {
        // Nonexistent port -- connect will fail/hang-ish; Shutdown sent immediately after
        // spawn must still cause a prompt return, not wait for a connect timeout.
        let handle = spawn_ws_engine("ws://127.0.0.1:1".to_string(), Duration::from_millis(50));
        handle.command_tx.send(WsCommand::Shutdown).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(3), handle.join_handle).await;
        assert!(
            result.is_ok(),
            "engine should shut down promptly even mid-connect-attempt"
        );
    }

    #[tokio::test]
    async fn shutdown_during_reconnect_wait_returns_promptly_and_does_not_hang() {
        // Use a long reconnect delay; connect to a refused port so it enters the wait
        // state, then Shutdown should win immediately rather than waiting out the delay.
        let mut handle = spawn_ws_engine("ws://127.0.0.1:1".to_string(), Duration::from_secs(30));
        let event = tokio::time::timeout(Duration::from_secs(2), handle.events.recv())
            .await
            .expect("should not time out")
            .expect("channel should be open");
        assert!(matches!(event, WsEvent::Disconnected));
        handle.command_tx.send(WsCommand::Shutdown).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), handle.join_handle).await;
        assert!(result.is_ok(), "must not wait out the full reconnect delay");
    }

    #[tokio::test]
    async fn send_during_reconnect_wait_is_dropped_not_buffered() {
        // Matches sendToMixingStationWS's own documented "dropped if not connected"
        // behavior -- this is intentional, not a repeat of midi_io.rs's discovery-loop bug.
        let mut handle = spawn_ws_engine("ws://127.0.0.1:1".to_string(), Duration::from_secs(30));
        let _disconnected = handle.events.recv().await.unwrap();
        handle
            .command_tx
            .send(WsCommand::Send("lost".to_string()))
            .unwrap();
        handle.command_tx.send(WsCommand::Shutdown).unwrap();
        // No Message event should ever arrive for "lost" -- only Shutdown-triggered exit.
        let result = tokio::time::timeout(Duration::from_secs(2), handle.join_handle).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn acknowledges_a_server_initiated_close_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("ws://{addr}");
        let (got_close_tx, got_close_rx) = tokio::sync::oneshot::channel::<bool>();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            // Server initiates the close; the engine must reply with its own close frame
            // rather than just dropping the TCP connection.
            futures_util::SinkExt::send(&mut ws, Message::Close(None))
                .await
                .unwrap();
            let mut acked = false;
            while let Some(Ok(msg)) = futures_util::StreamExt::next(&mut ws).await {
                if msg.is_close() {
                    acked = true;
                    break;
                }
            }
            let _ = got_close_tx.send(acked);
        });

        let mut handle = spawn_ws_engine(url, Duration::from_secs(30));
        assert!(matches!(
            handle.events.recv().await.unwrap(),
            WsEvent::Connected
        ));

        let acked = tokio::time::timeout(Duration::from_secs(2), got_close_rx)
            .await
            .expect("server should observe the close reply")
            .unwrap();
        assert!(acked, "engine must complete the WS closing handshake");

        assert!(matches!(
            handle.events.recv().await.unwrap(),
            WsEvent::Disconnected
        ));
        handle.command_tx.send(WsCommand::Shutdown).unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle.join_handle).await;
        server.abort();
    }

    #[tokio::test]
    async fn reconnects_after_server_closes_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("ws://{addr}");

        // First connection: accept then immediately close.
        let listener_handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            drop(ws); // close immediately

            // Second connection: accept and hold open.
            let (stream2, _) = listener.accept().await.unwrap();
            let _ws2 = tokio_tungstenite::accept_async(stream2).await.unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        let mut handle = spawn_ws_engine(url, Duration::from_millis(50));
        let first = handle.events.recv().await.unwrap();
        assert!(matches!(first, WsEvent::Connected));
        let second = handle.events.recv().await.unwrap();
        assert!(matches!(second, WsEvent::Disconnected));
        let third = tokio::time::timeout(Duration::from_secs(2), handle.events.recv())
            .await
            .expect("should reconnect within timeout")
            .unwrap();
        assert!(matches!(third, WsEvent::Connected));

        handle.command_tx.send(WsCommand::Shutdown).unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle.join_handle).await;
        listener_handle.abort();
    }
}
