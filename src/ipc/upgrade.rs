use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Stdio;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

#[cfg(unix)]
use anyhow::Context;
use anyhow::{Result, bail};
#[cfg(unix)]
use nix::sys::signal::{self, Signal};
#[cfg(unix)]
use nix::unistd::Pid;
#[cfg(unix)]
use tokio::io::AsyncWriteExt;
#[cfg(unix)]
use tokio::net::UnixStream;

use crate::ipc::daemon::BoundSocket;
use crate::ipc::socket;

#[cfg(unix)]
pub async fn run(config_path: &Path, timeout: Duration) -> Result<()> {
    let public_socket = socket::default_socket_path();

    match socket::read_pid(&public_socket) {
        Some(old_pid) if socket::is_daemon_alive(&public_socket) => {
            let staged_socket =
                socket::staged_socket_path(&public_socket, std::process::id() as i32);
            let mut child =
                spawn_staged_daemon(config_path, &staged_socket, &public_socket, old_pid)
                    .with_context(|| "failed to spawn staged daemon")?;

            println!(
                "Staging daemon upgrade from PID {old_pid} via {}",
                staged_socket.display()
            );

            let deadline = Instant::now() + timeout;
            loop {
                if socket::read_pid(&public_socket) != Some(old_pid)
                    && socket::is_daemon_alive(&public_socket)
                    && smoke_handshake(&public_socket).await.is_ok()
                {
                    let new_pid = socket::read_pid(&public_socket)
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    println!(
                        "Upgrade promoted daemon PID {new_pid}. Old daemon PID {old_pid} is draining existing clients."
                    );
                    return Ok(());
                }

                if let Some(status) = child.try_wait()? {
                    bail!("staged daemon exited before promotion: {status}");
                }

                if Instant::now() >= deadline {
                    bail!(
                        "timed out after {:?} waiting for staged daemon promotion",
                        timeout
                    );
                }

                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        Some(stale_pid) => {
            println!("Daemon PID {stale_pid} is stale. Cleaning up and starting a new daemon.");
            socket::cleanup_files(&public_socket);
            spawn_normal_daemon(config_path)?;
            wait_for_promoted_daemon(&public_socket, None, timeout).await?;
            println!("Daemon started.");
            Ok(())
        }
        None => {
            if public_socket.exists() {
                println!("Socket exists without a PID file. Cleaning up stale files.");
                socket::cleanup_files(&public_socket);
            }
            spawn_normal_daemon(config_path)?;
            wait_for_promoted_daemon(&public_socket, None, timeout).await?;
            println!("Daemon started.");
            Ok(())
        }
    }
}

#[cfg(not(unix))]
pub async fn run(_config_path: &Path, _timeout: Duration) -> Result<()> {
    println!(
        "`gatemini upgrade` is not supported on Windows because daemon mode uses Unix sockets."
    );
    Ok(())
}

#[cfg(unix)]
pub fn promote_staged(
    mut bound: BoundSocket,
    public_socket: PathBuf,
    old_pid: i32,
) -> Result<BoundSocket> {
    promote_staged_inner(&mut bound, public_socket, old_pid, true)?;
    Ok(bound)
}

#[cfg(not(unix))]
pub fn promote_staged(
    _bound: BoundSocket,
    _public_socket: PathBuf,
    _old_pid: i32,
) -> Result<BoundSocket> {
    bail!("staged daemon promotion is not supported on this platform")
}

#[cfg(unix)]
fn promote_staged_inner(
    bound: &mut BoundSocket,
    public_socket: PathBuf,
    old_pid: i32,
    signal_old: bool,
) -> Result<()> {
    let staged_socket = bound.socket_path.clone();
    let new_pid = std::process::id() as i32;
    let old_info = socket::read_generation_info(&public_socket);
    let signal_drain_supported = should_signal_old_generation(old_info.as_ref());
    let _lock = socket::try_acquire_lock(&public_socket)
        .with_context(|| format!("failed to acquire lock for {}", public_socket.display()))?;

    if socket::read_pid(&public_socket) != Some(old_pid) {
        bail!("public daemon PID changed before promotion");
    }
    if !socket::is_pid_alive(old_pid) {
        bail!("old daemon PID {old_pid} is no longer alive");
    }

    let drain_socket = socket::drain_socket_path(&public_socket, old_pid);
    let drain_pid = socket::drain_pid_path(&public_socket, old_pid);
    socket::cleanup_drain_generation_files(&public_socket, old_pid);

    std::fs::rename(&public_socket, &drain_socket).with_context(|| {
        format!(
            "failed to move public socket {} to drain socket {}",
            public_socket.display(),
            drain_socket.display()
        )
    })?;
    std::fs::write(&drain_pid, old_pid.to_string())
        .with_context(|| format!("failed to write drain PID file {}", drain_pid.display()))?;
    if let Some(mut old_info) = old_info {
        old_info.role = socket::GenerationRole::Draining;
        old_info.socket_path = drain_socket.clone();
        let bytes = serde_json::to_vec_pretty(&old_info).map_err(std::io::Error::other)?;
        std::fs::write(
            socket::drain_generation_info_path(&public_socket, old_pid),
            bytes,
        )?;
    } else {
        socket::write_drain_generation_info(&public_socket, old_pid)?;
    }

    if let Err(e) = std::fs::rename(&staged_socket, &public_socket) {
        let _ = std::fs::rename(&drain_socket, &public_socket);
        let _ = std::fs::write(socket::pid_path(&public_socket), old_pid.to_string());
        bail!(
            "failed to promote staged socket {} to public socket {}: {e}",
            staged_socket.display(),
            public_socket.display()
        );
    }
    std::fs::write(socket::pid_path(&public_socket), new_pid.to_string())?;
    socket::write_generation_info(&public_socket, socket::GenerationRole::Active, new_pid)?;
    let _ = std::fs::remove_file(socket::pid_path(&staged_socket));
    let _ = std::fs::remove_file(socket::generation_info_path(&staged_socket));

    if signal_old
        && signal_drain_supported
        && let Err(e) = signal::kill(Pid::from_raw(old_pid), Signal::SIGUSR2)
    {
        eprintln!(
            "warning: promoted new daemon but failed to signal old daemon PID {old_pid} to drain: {e}"
        );
    }

    bound.socket_path = public_socket;
    Ok(())
}

fn should_signal_old_generation(old_info: Option<&socket::GenerationInfo>) -> bool {
    // Daemons before generation metadata did not install a SIGUSR2 drain handler.
    // The first upgrade from such a version must keep that daemon alive and let
    // it exit by its normal idle timeout rather than terminating it by signal.
    old_info.is_some()
}

#[cfg(all(test, unix))]
pub(crate) fn promote_staged_without_signal_for_test(
    bound: &mut BoundSocket,
    public_socket: PathBuf,
    old_pid: i32,
) -> Result<()> {
    promote_staged_inner(bound, public_socket, old_pid, false)
}

#[cfg(unix)]
fn spawn_normal_daemon(config_path: &Path) -> Result<std::process::Child> {
    spawn_daemon(config_path, None, None, None)
}

#[cfg(unix)]
fn spawn_staged_daemon(
    config_path: &Path,
    staged_socket: &Path,
    public_socket: &Path,
    old_pid: i32,
) -> Result<std::process::Child> {
    spawn_daemon(
        config_path,
        Some(staged_socket),
        Some(public_socket),
        Some(old_pid),
    )
}

#[cfg(unix)]
fn spawn_daemon(
    config_path: &Path,
    staged_socket: Option<&Path>,
    promote_to: Option<&Path>,
    old_pid: Option<i32>,
) -> Result<std::process::Child> {
    use std::os::unix::process::CommandExt;

    let exe = std::env::current_exe().context("could not determine current executable")?;
    let daemon_cwd = config_path
        .parent()
        .filter(|p| p.is_dir())
        .map(Path::to_path_buf)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("/"));

    let mut command = std::process::Command::new(exe);
    let stderr = socket::open_daemon_log()
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());
    command
        .arg("-c")
        .arg(config_path)
        .arg("serve")
        .current_dir(daemon_cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr);

    if let Some(socket) = staged_socket {
        command.arg("--socket").arg(socket);
    }
    if let Some(public) = promote_to {
        command.arg("--promote-to").arg(public);
    }
    if let Some(pid) = old_pid {
        command.arg("--old-pid").arg(pid.to_string());
    }
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    command.spawn().context("failed to spawn daemon")
}

#[cfg(unix)]
async fn wait_for_promoted_daemon(
    public_socket: &Path,
    previous_pid: Option<i32>,
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if socket::read_pid(public_socket) != previous_pid
            && socket::is_daemon_alive(public_socket)
            && smoke_handshake(public_socket).await.is_ok()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("timed out after {:?} waiting for daemon", timeout);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(unix)]
async fn smoke_handshake(socket_path: &Path) -> Result<()> {
    let mut stream = tokio::time::timeout(Duration::from_secs(2), UnixStream::connect(socket_path))
        .await
        .context("connect timed out")?
        .context("connect failed")?;

    let init = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "gatemini-upgrade", "version": env!("CARGO_PKG_VERSION")}
        }
    });
    stream
        .write_all(format!("{init}\n").as_bytes())
        .await
        .context("failed to write upgrade smoke initialize")?;
    let response = crate::ipc::mcp_framing::read_line(&mut stream)
        .await
        .context("failed to read upgrade smoke initialize response")?;
    if response.is_empty() {
        bail!("daemon closed during upgrade smoke initialize");
    }
    stream
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .await
        .context("failed to write upgrade smoke initialized notification")?;
    Ok(())
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;

    #[test]
    fn legacy_generation_without_metadata_does_not_support_drain_signal() {
        assert!(!should_signal_old_generation(None));
    }

    #[test]
    fn generation_with_metadata_supports_drain_signal() {
        let info = socket::GenerationInfo {
            pid: 123,
            version: "1.14.0".to_string(),
            role: socket::GenerationRole::Active,
            socket_path: PathBuf::from("/tmp/gatemini.sock"),
        };

        assert!(should_signal_old_generation(Some(&info)));
    }

    #[tokio::test]
    async fn staged_promotion_moves_public_socket_and_writes_drain_markers() {
        let dir = tempfile::tempdir().unwrap();
        let public_socket = dir.path().join("gatemini.sock");
        let staged_socket = socket::staged_socket_path(&public_socket, 77);
        let old_pid = std::process::id() as i32;
        let new_pid = old_pid;

        std::fs::write(&public_socket, "").unwrap();
        std::fs::write(socket::pid_path(&public_socket), old_pid.to_string()).unwrap();
        let old_info = socket::GenerationInfo {
            pid: old_pid,
            version: "old-version".to_string(),
            role: socket::GenerationRole::Active,
            socket_path: public_socket.clone(),
        };
        std::fs::write(
            socket::generation_info_path(&public_socket),
            serde_json::to_vec_pretty(&old_info).unwrap(),
        )
        .unwrap();

        let listener = UnixListener::bind(&staged_socket).unwrap();
        std::fs::write(socket::pid_path(&staged_socket), new_pid.to_string()).unwrap();
        socket::write_generation_info(&staged_socket, socket::GenerationRole::Staged, new_pid)
            .unwrap();

        let mut bound = BoundSocket {
            listener,
            socket_path: staged_socket.clone(),
        };

        promote_staged_inner(&mut bound, public_socket.clone(), old_pid, false).unwrap();

        assert_eq!(bound.socket_path, public_socket);
        assert!(public_socket.exists(), "staged socket should become public");
        assert!(
            socket::drain_socket_path(&public_socket, old_pid).exists(),
            "old public socket should be moved to drain socket"
        );
        assert_eq!(socket::read_pid(&public_socket), Some(new_pid));
        assert_eq!(
            std::fs::read_to_string(socket::drain_pid_path(&public_socket, old_pid))
                .unwrap()
                .trim(),
            old_pid.to_string()
        );
        assert!(!socket::pid_path(&staged_socket).exists());
        assert!(!socket::generation_info_path(&staged_socket).exists());
        assert_eq!(
            socket::read_generation_info(&public_socket).unwrap().role,
            socket::GenerationRole::Active
        );
        let drain_info: socket::GenerationInfo = serde_json::from_slice(
            &std::fs::read(socket::drain_generation_info_path(&public_socket, old_pid)).unwrap(),
        )
        .unwrap();
        assert_eq!(drain_info.version, "old-version");
        assert_eq!(drain_info.role, socket::GenerationRole::Draining);
    }
}
