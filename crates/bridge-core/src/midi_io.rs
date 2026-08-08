//! Console 1 Fader Mk III MIDI I/O: port-name matching (pure, tested) and the async engine
//! that opens real ports and bridges MIDI messages to/from the rest of the bridge (integration
//! code, manually verified — real hardware I/O can't be unit-tested, matches this project's
//! established `CLAUDE.md` testing philosophy of verifying live MIDI/WS handling by running it).
//!
//! See `index.js`'s `openSoftubeMidiInput`/`openSoftubeMidiOutput`/`tryOpenConsole1MidiPorts`/
//! `waitForConsole1MidiPorts` for the originals this ports.

use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use std::time::Duration;

pub const MIDI_PORT_RETRY_INTERVAL_MS: u64 = 2000;
pub const DEFAULT_PREFERRED_PORT_NAMES: &[&str] = &["Console 1 Fader Mk III DAW"];

/// Find the index of the first port name containing any of `preferred` — pure substring
/// matching, case-sensitive (matching JS's `.includes()`).
pub fn find_matching_port_index(port_names: &[String], preferred: &[&str]) -> Option<usize> {
    port_names
        .iter()
        .position(|name| preferred.iter().any(|p| name.contains(p)))
}

/// A message received from Console 1's MIDI input: `(timestamp_micros, raw_bytes)`.
pub type InboundMidiMessage = (u64, Vec<u8>);

/// A command to the dedicated MIDI I/O thread.
pub enum MidiCommand {
    /// Send a raw (already-framed) SysEx message to Console 1.
    Send(Vec<u8>),
    /// Cleanly close both ports and stop the thread.
    Shutdown,
}

/// Handle to the running MIDI I/O engine — holds the outbound command sender and the thread
/// join handle. Send `MidiCommand::Shutdown` and then join `join_handle` to stop it cleanly —
/// this works correctly whether the thread is still searching for hardware or already
/// connected. Simply dropping this handle without sending `Shutdown` first also works (the
/// thread's `command_rx.recv()` returns `Err` once the sender drops, so it exits on its own),
/// but doesn't guarantee the thread has fully exited before your own code continues.
pub struct MidiEngineHandle {
    pub command_tx: std::sync::mpsc::Sender<MidiCommand>,
    pub join_handle: std::thread::JoinHandle<()>,
}

/// Open both Console 1 Fader MIDI ports (retrying every `MIDI_PORT_RETRY_INTERVAL_MS` until
/// found — never gives up, matching `waitForConsole1MidiPorts`'s "the GUI may spawn this before
/// the user has connected the Fader" rationale) on a dedicated OS thread, and bridge inbound
/// messages out through `inbound_tx`. Returns once the thread is spawned (the thread itself
/// blocks internally until ports are found).
pub async fn spawn_midi_engine(
    preferred_names: &'static [&'static str],
    inbound_tx: tokio::sync::mpsc::UnboundedSender<InboundMidiMessage>,
) -> MidiEngineHandle {
    let (command_tx, command_rx) = std::sync::mpsc::channel::<MidiCommand>();

    let join_handle = std::thread::spawn(move || {
        run_midi_io_thread(preferred_names, inbound_tx, command_rx);
    });

    MidiEngineHandle {
        command_tx,
        join_handle,
    }
}

/// Outcome of [`run_discovery_loop`]: either a `Shutdown`/disconnect was seen before a
/// connection was established, or a connection was established — bundled with any `Send`
/// payloads that were queued (in arrival order) while discovery was still in progress, so the
/// caller can flush them before treating the connection as ready for normal use.
enum DiscoveryOutcome<C> {
    ShutdownRequested,
    Connected {
        connection: C,
        pending_sends: Vec<Vec<u8>>,
    },
}

/// Drives the "retry until connected, but don't hang on Shutdown and don't lose queued Sends"
/// control flow, independent of what "connect" actually does — `try_connect_fn` is injected so
/// this can be unit-tested deterministically (see the `tests` module below) without real MIDI
/// hardware. `MidiCommand::Send` payloads observed via `try_recv` while still retrying are
/// buffered rather than discarded (the bug this function's extraction fixes: a plain
/// `matches!(command_rx.try_recv(), ...)` peek used to consume-and-drop any `Send` it happened
/// to see), and returned alongside the connection once `try_connect_fn` succeeds so the caller
/// can deliver them in order.
fn run_discovery_loop<C>(
    command_rx: &std::sync::mpsc::Receiver<MidiCommand>,
    mut try_connect_fn: impl FnMut() -> Option<C>,
    retry_interval: Duration,
) -> DiscoveryOutcome<C> {
    let mut pending_sends: Vec<Vec<u8>> = Vec::new();

    loop {
        match command_rx.try_recv() {
            Ok(MidiCommand::Send(bytes)) => pending_sends.push(bytes),
            Ok(MidiCommand::Shutdown) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return DiscoveryOutcome::ShutdownRequested;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }

        match try_connect_fn() {
            Some(connection) => {
                return DiscoveryOutcome::Connected {
                    connection,
                    pending_sends,
                }
            }
            None => {
                println!("Waiting for Console 1 Fader MIDI ports...");
                std::thread::sleep(retry_interval);
            }
        }
    }
}

fn run_midi_io_thread(
    preferred_names: &'static [&'static str],
    inbound_tx: tokio::sync::mpsc::UnboundedSender<InboundMidiMessage>,
    command_rx: std::sync::mpsc::Receiver<MidiCommand>,
) {
    let (_conn_in, mut conn_out) = match run_discovery_loop(
        &command_rx,
        || try_connect(preferred_names, &inbound_tx),
        Duration::from_millis(MIDI_PORT_RETRY_INTERVAL_MS),
    ) {
        DiscoveryOutcome::ShutdownRequested => return,
        DiscoveryOutcome::Connected {
            mut connection,
            pending_sends,
        } => {
            println!("Console 1 Fader MIDI ports found.");
            // Flush anything queued during discovery, in order, before treating the connection
            // as ready for the normal receive loop below.
            for bytes in pending_sends {
                if let Err(e) = connection.1.send(&bytes) {
                    eprintln!("Failed to send SysEx to Console 1: {e}");
                }
            }
            connection
        }
    };

    while let Ok(command) = command_rx.recv() {
        match command {
            MidiCommand::Send(bytes) => {
                if let Err(e) = conn_out.send(&bytes) {
                    eprintln!("Failed to send SysEx to Console 1: {e}");
                }
            }
            MidiCommand::Shutdown => break,
        }
    }

    conn_out.close();
    // `_conn_in` closes (and the port is released) when it drops at the end of this scope.
}

/// Find and connect both Console 1 ports in one attempt. Returns `None` on ANY failure —
/// ports not found, or found-but-connect-failed (e.g. hardware unplugged mid-handshake) — so
/// the caller's retry loop treats both cases identically instead of panicking on the latter.
fn try_connect(
    preferred_names: &[&str],
    inbound_tx: &tokio::sync::mpsc::UnboundedSender<InboundMidiMessage>,
) -> Option<(MidiInputConnection<()>, MidiOutputConnection)> {
    let (mut midi_in, in_port, midi_out, out_port) = try_open_ports(preferred_names)?;
    midi_in.ignore(Ignore::None);
    let inbound_tx = inbound_tx.clone();
    let conn_in = midi_in
        .connect(
            &in_port,
            "Softube MS Bridge (in)",
            move |stamp, message, _| {
                let _ = inbound_tx.send((stamp, message.to_vec()));
            },
            (),
        )
        .ok()?;
    let conn_out = midi_out
        .connect(&out_port, "Softube MS Bridge (out)")
        .ok()?;
    Some((conn_in, conn_out))
}

type OpenedPorts = (
    MidiInput,
    midir::MidiInputPort,
    MidiOutput,
    midir::MidiOutputPort,
);

fn try_open_ports(preferred_names: &[&str]) -> Option<OpenedPorts> {
    let midi_in = MidiInput::new("Softube MS Bridge").ok()?;
    let in_ports = midi_in.ports();
    let in_names: Vec<String> = in_ports
        .iter()
        .filter_map(|p| midi_in.port_name(p).ok())
        .collect();
    let in_idx = find_matching_port_index(&in_names, preferred_names)?;

    let midi_out = MidiOutput::new("Softube MS Bridge").ok()?;
    let out_ports = midi_out.ports();
    let out_names: Vec<String> = out_ports
        .iter()
        .filter_map(|p| midi_out.port_name(p).ok())
        .collect();
    let out_idx = find_matching_port_index(&out_names, preferred_names)?;

    Some((
        midi_in,
        in_ports[in_idx].clone(),
        midi_out,
        out_ports[out_idx].clone(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for the bug where `Send` commands received while the discovery loop was
    /// still retrying got silently consumed and dropped by a `try_recv()` used as a non-
    /// destructive peek. `try_connect_fn` simulates "port not found yet" for the first two
    /// attempts (matching the real retry loop's shape) before succeeding, with a zero-length
    /// retry interval so the test runs instantly — no real MIDI hardware involved.
    #[test]
    fn queued_sends_during_discovery_are_delivered_in_order_once_connected() {
        let (command_tx, command_rx) = std::sync::mpsc::channel::<MidiCommand>();
        command_tx.send(MidiCommand::Send(vec![1, 2, 3])).unwrap();
        command_tx.send(MidiCommand::Send(vec![4, 5])).unwrap();

        let mut attempts = 0;
        let outcome = run_discovery_loop(
            &command_rx,
            || {
                attempts += 1;
                if attempts < 3 {
                    None
                } else {
                    Some(42)
                }
            },
            Duration::from_millis(0),
        );

        match outcome {
            DiscoveryOutcome::Connected {
                connection,
                pending_sends,
            } => {
                assert_eq!(connection, 42);
                assert_eq!(pending_sends, vec![vec![1, 2, 3], vec![4, 5]]);
            }
            DiscoveryOutcome::ShutdownRequested => {
                panic!("expected Connected once try_connect_fn succeeds")
            }
        }
        assert_eq!(attempts, 3);
    }

    /// `Shutdown` received during discovery must still return immediately (the case the
    /// original consuming-`try_recv()` peek was written to handle) — the fix for lost `Send`s
    /// must not reintroduce a hang here.
    #[test]
    fn shutdown_during_discovery_returns_immediately() {
        let (command_tx, command_rx) = std::sync::mpsc::channel::<MidiCommand>();
        command_tx.send(MidiCommand::Shutdown).unwrap();

        let mut attempts = 0;
        let outcome = run_discovery_loop(
            &command_rx,
            || {
                attempts += 1;
                None::<()>
            },
            Duration::from_millis(0),
        );

        assert!(matches!(outcome, DiscoveryOutcome::ShutdownRequested));
        assert_eq!(
            attempts, 0,
            "should return before ever attempting to connect"
        );
    }

    /// A `Send` queued before `Shutdown` arrives has no destination (the port was never found),
    /// but the important guarantee is still upheld: the loop doesn't panic or hang, it just
    /// reports `ShutdownRequested` — there's nowhere to flush a pending Send to in this case.
    #[test]
    fn send_followed_by_shutdown_during_discovery_does_not_hang() {
        let (command_tx, command_rx) = std::sync::mpsc::channel::<MidiCommand>();
        command_tx.send(MidiCommand::Send(vec![9, 9])).unwrap();
        command_tx.send(MidiCommand::Shutdown).unwrap();

        let outcome = run_discovery_loop(&command_rx, || None::<()>, Duration::from_millis(0));

        assert!(matches!(outcome, DiscoveryOutcome::ShutdownRequested));
    }

    /// Sender dropping (disconnect) during discovery must also return immediately, matching the
    /// original `Err(TryRecvError::Disconnected)` handling.
    #[test]
    fn disconnected_sender_during_discovery_returns_immediately() {
        let (command_tx, command_rx) = std::sync::mpsc::channel::<MidiCommand>();
        drop(command_tx);

        let outcome = run_discovery_loop(&command_rx, || None::<()>, Duration::from_millis(0));

        assert!(matches!(outcome, DiscoveryOutcome::ShutdownRequested));
    }

    #[test]
    fn matches_exact_substring() {
        let names = vec!["Console 1 Fader Mk III DAW".to_string()];
        assert_eq!(
            find_matching_port_index(&names, DEFAULT_PREFERRED_PORT_NAMES),
            Some(0)
        );
    }

    #[test]
    fn matches_when_preferred_name_is_a_substring_of_a_longer_port_name() {
        let names = vec![
            "IAC Driver Bus 1".to_string(),
            "Console 1 Fader Mk III DAW Port 1".to_string(),
        ];
        assert_eq!(
            find_matching_port_index(&names, DEFAULT_PREFERRED_PORT_NAMES),
            Some(1)
        );
    }

    #[test]
    fn returns_first_match_when_multiple_ports_match() {
        let names = vec![
            "Console 1 Fader Mk III DAW A".to_string(),
            "Console 1 Fader Mk III DAW B".to_string(),
        ];
        assert_eq!(
            find_matching_port_index(&names, DEFAULT_PREFERRED_PORT_NAMES),
            Some(0)
        );
    }

    #[test]
    fn no_match_returns_none() {
        let names = vec!["Some Other Device".to_string()];
        assert_eq!(
            find_matching_port_index(&names, DEFAULT_PREFERRED_PORT_NAMES),
            None
        );
    }

    #[test]
    fn matching_is_case_sensitive() {
        let names = vec!["console 1 fader mk iii daw".to_string()];
        assert_eq!(
            find_matching_port_index(&names, DEFAULT_PREFERRED_PORT_NAMES),
            None
        );
    }

    #[test]
    fn empty_port_list_returns_none() {
        let names: Vec<String> = vec![];
        assert_eq!(
            find_matching_port_index(&names, DEFAULT_PREFERRED_PORT_NAMES),
            None
        );
    }
}
