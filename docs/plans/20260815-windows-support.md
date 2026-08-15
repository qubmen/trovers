# Plan: Windows support (prerequisite for CI target)

## Status — 2026-08-15

Not started. Filed as a placeholder so the Windows exclusion in
`dist-workspace.toml` points somewhere concrete instead of just a comment.

## Context

The release CI pipeline (`dist-workspace.toml`, `.github/workflows/release.yml`)
builds macOS (x86_64 + aarch64) and Linux (x86_64 gnu + musl, aarch64 gnu), but
deliberately excludes `x86_64-pc-windows-msvc`. It's not a CI configuration gap
— the code itself won't compile on Windows yet:

- `src/player.rs` uses `tokio::net::UnixStream` for the mpv IPC control
  channel, hardcoded to a socket path under `SOCKET_DIR = "/tmp"`. Windows has
  no Unix domain sockets in this form.
- The same file uses `libc::kill(pid, 0)` with an `EPERM`-based check to detect
  whether an orphaned mpv process is still alive — a POSIX-only pattern.
- There is no `cfg(windows)`/`cfg(unix)`/`target_os` conditional compilation
  anywhere in `src/`, so nothing currently degrades gracefully on Windows —
  it's a hard build failure, not a runtime limitation.

## Approach (sketch, not yet detailed)

1. **mpv IPC channel**: mpv's `--input-ipc-server` accepts a TCP address as
   well as a Unix socket path on all platforms. Switching to a loopback TCP
   socket (e.g. `127.0.0.1:<ephemeral-port>`) removes the platform split
   entirely — one code path instead of a `cfg`-gated pair. Alternative if TCP
   is undesirable: `cfg(windows)` named pipe + `cfg(unix)` Unix socket, kept
   behind a small trait/enum so the rest of `player.rs` doesn't fork.
2. **Orphan liveness check**: replace `libc::kill(pid, 0)` with a portable
   process-existence check — the `sysinfo` crate covers this cross-platform,
   or a `cfg(unix)` / `cfg(windows)` (`OpenProcess`/`GetExitCodeProcess`) pair
   if pulling in `sysinfo` for one check is too heavy.
3. Add a `windows-latest` job to `.github/workflows/ci.yml` (test-only, not
   release) as soon as the above lands, to keep it from regressing silently.
4. Once compiling and passing tests on Windows, add `x86_64-pc-windows-msvc`
   back to `targets` in `dist-workspace.toml`, decide whether to also add the
   `powershell` installer, and run `dist generate` to update the release
   workflow.

## Open questions

- Is Windows support actually wanted, or was excluding it in the first CI pass
  a permanent decision rather than a deferral? No user demand has been
  reported yet — worth confirming before investing the work above.
