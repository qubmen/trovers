use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::sync::watch;
use tokio::time::{sleep, Duration};
use tracing::info;

static SOCKET_SEQ: AtomicU32 = AtomicU32::new(0);

/// Directory holding the mpv IPC sockets, and the filename prefix that marks a
/// socket as ours. Full form: `<SOCKET_DIR>/<SOCKET_PREFIX><pid>-<seq>.sock`.
const SOCKET_DIR: &str = "/tmp";
const SOCKET_PREFIX: &str = "trovers-";

/// How long any single mpv IPC exchange may take before it is abandoned.
/// Generous for a local socket, and short enough that a wedged mpv costs one
/// noticeable pause rather than a permanently frozen UI.
const IPC_TIMEOUT: Duration = Duration::from_secs(2);

/// How many unsolicited event lines to skip while looking for a command reply
/// before giving up. A bound, not an expectation — mpv normally sends none.
const MAX_IPC_EVENT_LINES: usize = 32;

pub struct Player {
    pub process: Child,
    pub socket_path: PathBuf,
}

impl Player {
    /// Spawn mpv with --no-video and an IPC socket at /tmp/trovers-<pid>-<seq>.sock.
    /// Pass `start_pos` to resume playback at a specific position (e.g. when
    /// switching from a stream to the freshly downloaded local file).
    /// Retries connecting to the socket up to 20 times with 50ms delay.
    pub async fn spawn(source: &str, start_pos: Option<f64>) -> Result<Self> {
        let pid = std::process::id();
        let seq = SOCKET_SEQ.fetch_add(1, Ordering::SeqCst);
        let socket_path =
            PathBuf::from(SOCKET_DIR).join(format!("{SOCKET_PREFIX}{pid}-{seq}.sock"));

        // Remove stale socket if present
        let _ = std::fs::remove_file(&socket_path);

        let mut cmd = Command::new("mpv");
        cmd.args([
            "--no-video",
            // Without this mpv keeps its own terminal keybindings active on the
            // tty we share with the TUI and competes with us for keystrokes.
            "--no-terminal",
            "--really-quiet",
            &format!("--input-ipc-server={}", socket_path.display()),
        ]);
        if let Some(pos) = start_pos.filter(|&p| p > 0.1) {
            cmd.arg(format!("--start={pos:.3}"));
        }
        cmd.arg(source);

        let process = cmd
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            // Without this, dropping the `Child` *detaches* mpv instead of
            // killing it. That is what left mpv playing forever whenever the
            // future below was cancelled mid-spawn (user quits or switches
            // track during the socket wait) — the `Child` is still a bare
            // local at that point, so `Player::drop` never gets to run.
            .kill_on_drop(true)
            .spawn()
            .context("failed to spawn mpv")?;

        // Wait for the socket to appear (up to 20 × 50ms = 1s)
        let mut attempts = 0u8;
        loop {
            if socket_path.exists() {
                break;
            }
            attempts += 1;
            if attempts >= 20 {
                bail!("mpv IPC socket did not appear at {}", socket_path.display());
            }
            sleep(Duration::from_millis(50)).await;
        }

        Ok(Self { process, socket_path })
    }

    /// Send a raw JSON command over the IPC socket and return the response value.
    pub async fn send_command(&self, cmd: serde_json::Value) -> Result<serde_json::Value> {
        self.send_command_with_timeout(cmd, IPC_TIMEOUT).await
    }

    /// `send_command` with an explicit deadline, so tests do not have to wait out
    /// the real one.
    pub async fn send_command_with_timeout(
        &self,
        cmd: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value> {
        // The whole exchange is bounded, not just the connect. An mpv that
        // accepts the connection and then never answers used to park this future
        // forever — and since key handling awaits it inline on the render loop,
        // that was the entire UI frozen with no way out.
        tokio::time::timeout(timeout, async {
            let mut stream = UnixStream::connect(&self.socket_path)
                .await
                .context("failed to connect to mpv IPC socket")?;

            let mut payload = cmd.to_string();
            payload.push('\n');
            stream.write_all(payload.as_bytes()).await?;

            read_reply(&mut stream).await
        })
        .await
        .with_context(|| format!("mpv IPC timed out after {timeout:?}"))?
    }

    pub async fn pause(&self) -> Result<()> {
        self.send_command(serde_json::json!({"command": ["set_property", "pause", true]}))
            .await?;
        Ok(())
    }

    pub async fn resume(&self) -> Result<()> {
        self.send_command(serde_json::json!({"command": ["set_property", "pause", false]}))
            .await?;
        Ok(())
    }

    /// Seek by `secs` seconds. mode: "relative" or "absolute".
    pub async fn seek(&self, secs: i64, mode: &str) -> Result<()> {
        self.send_command(serde_json::json!({"command": ["seek", secs, mode]}))
            .await?;
        Ok(())
    }

    pub async fn set_speed(&self, speed: f32) -> Result<()> {
        self.send_command(serde_json::json!({"command": ["set_property", "speed", speed]}))
            .await?;
        Ok(())
    }

    pub async fn set_volume(&self, vol: u8) -> Result<()> {
        self.send_command(serde_json::json!({"command": ["set_property", "volume", vol]}))
            .await?;
        Ok(())
    }

    pub async fn get_position(&self) -> Result<f64> {
        let resp = self
            .send_command(serde_json::json!({"command": ["get_property", "time-pos"]}))
            .await?;
        resp["data"]
            .as_f64()
            .context("mpv returned non-numeric time-pos")
    }
}

/// Standalone position poller — holds only the socket path, not the Player.
/// Useful when the Player has already been moved into App state.
///
/// `generation` is the player generation this poller belongs to;
/// `player_generation` is the app-wide counter. As soon as the two diverge the
/// player this poller was watching has been superseded and the loop stops
/// *without publishing another position*. That guard is what stops the outgoing
/// track's timestamp from bleeding into `App::position` and making the next
/// track resume where the previous one left off.
///
/// Returns `true` when the loop stopped because mpv exited on its own, which the
/// caller turns into a `TaskMsg::PlayerGone` so the app can drop its now-dead
/// `Player`. Returns `false` when it stopped for any other reason (superseded
/// generation, or the TUI dropped the receiver).
pub async fn poll_position_loop(
    socket_path: PathBuf,
    pos_tx: watch::Sender<f64>,
    generation: u64,
    player_generation: Arc<AtomicU64>,
) -> bool {
    loop {
        sleep(Duration::from_secs(1)).await;

        if player_generation.load(Ordering::SeqCst) != generation {
            return false;
        }

        match poll_time_pos(&socket_path).await {
            PollOutcome::Position(pos) => {
                // Re-check after the await: reading the socket yields, so the
                // player may have been replaced while the read was in flight.
                if player_generation.load(Ordering::SeqCst) != generation {
                    return false;
                }
                if pos_tx.send(pos).is_err() {
                    return false; // TUI has quit
                }
            }
            // Still buffering, or a local hiccup that says nothing about mpv's
            // health — either way, keep polling.
            PollOutcome::NotReady | PollOutcome::Transient => {}
            PollOutcome::Gone => return true,
        }
    }
}

/// Result of a single position poll.
enum PollOutcome {
    /// mpv reported a playback position.
    Position(f64),
    /// mpv answered but has no position yet — it is still buffering.
    NotReady,
    /// mpv is no longer listening on the socket: it has exited.
    Gone,
    /// The exchange failed for a reason that says nothing about whether mpv is
    /// still alive. Treated as "keep watching" so a passing hiccup never makes
    /// the app abandon a player that is happily still playing.
    Transient,
}

/// Ask mpv for `time-pos` over a fresh connection, classifying failures.
///
/// The distinction matters because **mpv does not unlink its IPC socket when it
/// exits** — the file stays on disk indefinitely, so its presence proves
/// nothing. A refused connection (or a missing file, once `Player::drop` has
/// cleaned up) is the only reliable "mpv is gone" signal.
async fn poll_time_pos(socket_path: &Path) -> PollOutcome {
    let mut stream = match UnixStream::connect(socket_path).await {
        Ok(stream) => stream,
        Err(e) => {
            return match e.kind() {
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound => {
                    PollOutcome::Gone
                }
                _ => PollOutcome::Transient,
            };
        }
    };

    // Bounded: an mpv that accepts the connection and then goes quiet would
    // otherwise park this poller forever, and a poller that never returns never
    // reports `Gone` either — so the app would keep a dead `Player` for good.
    // A timeout says nothing about whether mpv is alive, so it is `Transient`
    // and the next tick tries again.
    match tokio::time::timeout(IPC_TIMEOUT, query_time_pos(&mut stream)).await {
        Ok(Ok(Some(pos))) => PollOutcome::Position(pos),
        Ok(Ok(None)) => PollOutcome::NotReady,
        // Connected, then the exchange broke — most likely mpv shutting down
        // mid-request. The next poll will see the refused connection and report
        // it properly.
        Ok(Err(_)) | Err(_) => PollOutcome::Transient,
    }
}

/// Send a `get_property time-pos` request on an already-connected socket.
/// `Ok(None)` means mpv answered without a numeric position (still buffering).
async fn query_time_pos(stream: &mut UnixStream) -> Result<Option<f64>> {
    let mut payload = serde_json::json!({"command": ["get_property", "time-pos"]}).to_string();
    payload.push('\n');
    stream.write_all(payload.as_bytes()).await?;

    let resp = read_reply(stream).await?;
    Ok(resp["data"].as_f64())
}

/// Read one line from the socket.
async fn read_line(stream: &mut UnixStream) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await?;
        if byte[0] == b'\n' {
            return Ok(buf);
        }
        buf.push(byte[0]);
    }
}

/// Read mpv's reply to a command, skipping event lines.
///
/// mpv pushes events (`{"event":"playback-restart"}` and friends) to every
/// connected client, unprompted and interleaved with command replies. Taking the
/// first line as the reply meant an event arriving in the window between writing
/// a command and reading its answer was parsed as that answer — a `time-pos`
/// poll silently lost, or a command reported as failed when it had succeeded.
/// Replies are distinguishable: mpv puts an `error` field on every one.
async fn read_reply(stream: &mut UnixStream) -> Result<serde_json::Value> {
    for _ in 0..MAX_IPC_EVENT_LINES {
        let line = read_line(stream).await?;
        let value: serde_json::Value =
            serde_json::from_slice(&line).context("failed to parse mpv IPC response")?;
        if value.get("error").is_some() {
            return Ok(value);
        }
    }
    bail!("mpv sent only events, no reply, in {MAX_IPC_EVENT_LINES} lines")
}

/// Quit and unlink mpv IPC sockets left behind by trovers instances that died
/// without running `Player::drop` — a `SIGKILL`, or the machine losing power.
/// Such an mpv keeps playing forever with no UI attached that could stop it, so
/// this runs once at startup as a self-healing net.
///
/// Only sockets whose encoded pid no longer belongs to a live process are
/// touched, so a second trovers running concurrently is never disturbed. If a
/// dead instance's pid has since been recycled by an unrelated process we skip
/// its socket this time round; the next launch that sees the pid free will
/// clean it up.
pub async fn reap_orphaned_players() {
    let own_pid = std::process::id();
    let Ok(entries) = std::fs::read_dir(SOCKET_DIR) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(pid) = socket_owner_pid(&path) else {
            continue;
        };
        if pid == own_pid || process_is_alive(pid) {
            continue;
        }

        // The socket file outlives mpv, so a refused connection just means the
        // process is already gone and only the stale file needs removing.
        if let Ok(mut stream) = UnixStream::connect(&path).await {
            let _ = stream.write_all(b"{\"command\":[\"quit\"]}\n").await;
            info!(path = %path.display(), orphan_pid = pid, "quit orphaned mpv");
        }
        let _ = std::fs::remove_file(&path);
    }
}

/// Extract the pid from a `/tmp/trovers-<pid>-<seq>.sock` path, or `None` if the
/// path is not one of ours.
pub(crate) fn socket_owner_pid(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let rest = name.strip_prefix(SOCKET_PREFIX)?.strip_suffix(".sock")?;
    rest.split('-').next()?.parse::<u32>().ok()
}

/// True if `pid` is a live process. `EPERM` counts as alive: the process exists,
/// we just may not signal it.
fn process_is_alive(pid: u32) -> bool {
    // SAFETY: `kill` with signal 0 has no side effects — it performs only the
    // existence and permission checks and delivers nothing.
    if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

impl Drop for Player {
    fn drop(&mut self) {
        // Kill the mpv process and clean up the socket file
        let _ = self.process.start_kill();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}
