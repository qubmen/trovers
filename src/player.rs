use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::sync::watch;
use tokio::time::{sleep, Duration};

static SOCKET_SEQ: AtomicU32 = AtomicU32::new(0);

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
        let socket_path = PathBuf::from(format!("/tmp/trovers-{pid}-{seq}.sock"));

        // Remove stale socket if present
        let _ = std::fs::remove_file(&socket_path);

        let mut cmd = Command::new("mpv");
        cmd.args([
            "--no-video",
            "--really-quiet",
            &format!("--input-ipc-server={}", socket_path.display()),
        ]);
        if let Some(pos) = start_pos.filter(|&p| p > 0.1) {
            cmd.arg(format!("--start={pos:.3}"));
        }
        cmd.arg(source);

        let process = cmd
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
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
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .context("failed to connect to mpv IPC socket")?;

        let mut payload = cmd.to_string();
        payload.push('\n');
        stream.write_all(payload.as_bytes()).await?;

        // Read response (terminated by newline)
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            stream.read_exact(&mut byte).await?;
            if byte[0] == b'\n' {
                break;
            }
            buf.push(byte[0]);
        }

        let response: serde_json::Value =
            serde_json::from_slice(&buf).context("failed to parse mpv IPC response")?;
        Ok(response)
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
/// Stops when mpv exits (socket disappears) or the receiver is dropped.
pub async fn poll_position_loop(socket_path: PathBuf, pos_tx: watch::Sender<f64>) {
    loop {
        sleep(Duration::from_secs(1)).await;
        match time_pos_from_socket(&socket_path).await {
            Ok(pos) => {
                if pos_tx.send(pos).is_err() {
                    break; // TUI has quit
                }
            }
            Err(_) => {
                // Only stop if the socket is gone (mpv exited).
                // During initial buffering mpv returns null for time-pos — keep polling.
                if !socket_path.exists() {
                    break;
                }
            }
        }
    }
}

async fn time_pos_from_socket(socket_path: &PathBuf) -> Result<f64> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .context("connect to mpv socket")?;

    let mut payload =
        serde_json::json!({"command": ["get_property", "time-pos"]}).to_string();
    payload.push('\n');
    stream.write_all(payload.as_bytes()).await?;

    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await?;
        if byte[0] == b'\n' {
            break;
        }
        buf.push(byte[0]);
    }

    let resp: serde_json::Value = serde_json::from_slice(&buf)?;
    resp["data"].as_f64().context("non-numeric time-pos")
}

impl Drop for Player {
    fn drop(&mut self) {
        // Kill the mpv process and clean up the socket file
        let _ = self.process.start_kill();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}
