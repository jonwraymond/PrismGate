use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use nix::unistd::getuid;
use serde::{Deserialize, Serialize};

/// Resolve the default Unix socket path for this platform and user.
///
/// The path MUST be deterministic regardless of environment variables like `$TMPDIR`,
/// which can vary between shell sessions and spawned subprocesses (e.g. Claude Code's
/// MCP subprocess may not inherit `$TMPDIR`). Using an inconsistent path causes
/// multiple daemons to spawn — each at a different socket.
///
/// - Linux: `$XDG_RUNTIME_DIR/gatemini.sock` (guaranteed consistent per-user by spec)
/// - macOS/fallback: `/tmp/gatemini-$UID.sock` (deterministic, user-isolated)
#[cfg(unix)]
pub fn default_socket_path() -> PathBuf {
    // XDG_RUNTIME_DIR is set by systemd on Linux — guaranteed consistent per-user.
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("gatemini.sock");
    }

    // macOS + fallback: use /tmp with UID for user isolation.
    // Do NOT use $TMPDIR — it varies between shell sessions and spawned processes.
    let uid = getuid();
    PathBuf::from(format!("/tmp/gatemini-{}.sock", uid))
}

/// Path to the flock lockfile (sibling of the socket).
pub fn lock_path(socket: &Path) -> PathBuf {
    socket.with_extension("lock")
}

pub fn daemon_log_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("gatemini")
        .join("daemon.log")
}

pub fn open_daemon_log() -> io::Result<fs::File> {
    let path = daemon_log_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::OpenOptions::new().create(true).append(true).open(path)
}

/// Path to the PID file (sibling of the socket).
#[cfg(unix)]
pub fn pid_path(socket: &Path) -> PathBuf {
    socket.with_extension("pid")
}

fn socket_stem(socket: &Path) -> String {
    socket
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("gatemini")
        .to_string()
}

fn sibling_with_name(socket: &Path, name: String) -> PathBuf {
    socket
        .parent()
        .map(|p| p.join(&name))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Private socket path used while a new daemon generation initializes.
#[cfg(unix)]
pub fn staged_socket_path(socket: &Path, pid: i32) -> PathBuf {
    sibling_with_name(
        socket,
        format!("{}.upgrade-{pid}.sock", socket_stem(socket)),
    )
}

/// Socket path retained for an old draining daemon generation.
#[cfg(unix)]
pub fn drain_socket_path(socket: &Path, pid: i32) -> PathBuf {
    sibling_with_name(socket, format!("{}.drain-{pid}.sock", socket_stem(socket)))
}

/// PID marker path for an old draining daemon generation.
#[cfg(unix)]
pub fn drain_pid_path(socket: &Path, pid: i32) -> PathBuf {
    sibling_with_name(socket, format!("{}.drain-{pid}.pid", socket_stem(socket)))
}

/// Metadata file for the daemon generation serving a socket.
#[cfg(unix)]
pub fn generation_info_path(socket: &Path) -> PathBuf {
    sibling_with_name(socket, format!("{}.info.json", socket_stem(socket)))
}

/// Metadata file for an old draining daemon generation.
#[cfg(unix)]
pub fn drain_generation_info_path(socket: &Path, pid: i32) -> PathBuf {
    sibling_with_name(
        socket,
        format!("{}.drain-{pid}.info.json", socket_stem(socket)),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GenerationRole {
    Active,
    Staged,
    Draining,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GenerationInfo {
    pub pid: i32,
    pub version: String,
    pub role: GenerationRole,
    pub socket_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainGeneration {
    pub pid: i32,
    pub socket_path: PathBuf,
    pub pid_path: PathBuf,
    pub info_path: PathBuf,
    pub alive: bool,
}

#[cfg(unix)]
pub fn write_generation_info(socket: &Path, role: GenerationRole, pid: i32) -> io::Result<()> {
    let info = GenerationInfo {
        pid,
        version: env!("CARGO_PKG_VERSION").to_string(),
        role,
        socket_path: socket.to_path_buf(),
    };
    let bytes = serde_json::to_vec_pretty(&info).map_err(io::Error::other)?;
    fs::write(generation_info_path(socket), bytes)
}

#[cfg(unix)]
pub fn write_drain_generation_info(socket: &Path, pid: i32) -> io::Result<()> {
    let drain_socket = drain_socket_path(socket, pid);
    let info = GenerationInfo {
        pid,
        version: "unknown".to_string(),
        role: GenerationRole::Draining,
        socket_path: drain_socket,
    };
    let bytes = serde_json::to_vec_pretty(&info).map_err(io::Error::other)?;
    fs::write(drain_generation_info_path(socket, pid), bytes)
}

#[cfg(unix)]
pub fn read_generation_info(socket: &Path) -> Option<GenerationInfo> {
    let bytes = fs::read(generation_info_path(socket)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Check whether a daemon process is alive by reading the PID file and sending signal 0.
#[cfg(unix)]
pub fn is_daemon_alive(socket: &Path) -> bool {
    let pid_file = pid_path(socket);
    let Ok(contents) = fs::read_to_string(&pid_file) else {
        return false;
    };
    let Ok(pid) = contents.trim().parse::<i32>() else {
        return false;
    };
    // kill(pid, 0) returns Ok if process exists and we have permission to signal it.
    // ESRCH (no such process) → Err → dead.
    // EPERM (permission denied) → Err but process exists — shouldn't happen for same-user.
    is_pid_alive(pid)
}

/// Read the daemon PID from the PID file.
#[cfg(unix)]
pub fn read_pid(socket: &Path) -> Option<i32> {
    let contents = fs::read_to_string(pid_path(socket)).ok()?;
    contents.trim().parse().ok()
}

/// Remove stale socket, PID, and generation metadata files.
/// The flock file is intentionally preserved; stale lock files are harmless.
pub fn cleanup_files(socket: &Path) {
    #[cfg(unix)]
    let _ = fs::remove_file(socket);
    #[cfg(unix)]
    let _ = fs::remove_file(pid_path(socket));
    #[cfg(unix)]
    let _ = fs::remove_file(generation_info_path(socket));
    #[cfg(not(unix))]
    {
        let _ = std::fs::remove_file(socket);
    }
}

/// Remove a daemon generation's public files only when the PID file still
/// points to that generation. The shared flock file is intentionally preserved.
#[cfg(unix)]
pub fn cleanup_owned_generation_files(socket: &Path, owner_pid: i32) {
    if read_pid(socket) == Some(owner_pid) {
        let _ = fs::remove_file(socket);
        let _ = fs::remove_file(pid_path(socket));
        let _ = fs::remove_file(generation_info_path(socket));
    }
}

#[cfg(unix)]
pub fn cleanup_drain_generation_files(socket: &Path, owner_pid: i32) {
    let _ = fs::remove_file(drain_socket_path(socket, owner_pid));
    let _ = fs::remove_file(drain_pid_path(socket, owner_pid));
    let _ = fs::remove_file(drain_generation_info_path(socket, owner_pid));
}

#[cfg(unix)]
pub fn discover_drain_generations(socket: &Path) -> Vec<DrainGeneration> {
    let Some(parent) = socket.parent() else {
        return Vec::new();
    };
    let prefix = format!("{}.drain-", socket_stem(socket));
    let suffix = ".pid";

    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };

    let mut drains = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if !file_name.starts_with(&prefix) || !file_name.ends_with(suffix) {
            continue;
        }
        let pid_text = &file_name[prefix.len()..file_name.len() - suffix.len()];
        let Ok(pid) = pid_text.parse::<i32>() else {
            continue;
        };
        let pid_path = entry.path();
        let alive = is_pid_alive(pid);
        if !alive {
            cleanup_drain_generation_files(socket, pid);
            continue;
        }
        drains.push(DrainGeneration {
            pid,
            socket_path: drain_socket_path(socket, pid),
            pid_path,
            info_path: drain_generation_info_path(socket, pid),
            alive,
        });
    }
    drains.sort_by_key(|g| g.pid);
    drains
}

#[cfg(unix)]
pub fn is_pid_alive(pid: i32) -> bool {
    let pid = nix::unistd::Pid::from_raw(pid);
    nix::sys::signal::kill(pid, None).is_ok()
}

#[cfg(not(unix))]
pub fn write_generation_info(_socket: &Path, _role: GenerationRole, _pid: i32) -> io::Result<()> {
    Ok(())
}

#[cfg(not(unix))]
pub fn read_generation_info(_socket: &Path) -> Option<GenerationInfo> {
    None
}

#[cfg(not(unix))]
pub fn cleanup_owned_generation_files(_socket: &Path, _owner_pid: i32) {}

#[cfg(not(unix))]
pub fn discover_drain_generations(_socket: &Path) -> Vec<DrainGeneration> {
    Vec::new()
}

/// Try to acquire an exclusive, non-blocking flock on the lock file.
/// Returns the file descriptor on success (caller must keep it open to hold the lock).
#[cfg(unix)]
pub fn try_acquire_lock(socket: &Path) -> io::Result<fs::File> {
    use std::os::unix::io::AsRawFd;

    let lock_file = lock_path(socket);
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_file)?;

    let fd = file.as_raw_fd();
    let result = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(file)
}

#[cfg(not(unix))]
pub fn try_acquire_lock(socket: &Path) -> io::Result<std::fs::File> {
    let lock_file = lock_path(socket);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_file)?;

    Ok(file)
}

#[cfg(not(unix))]
pub fn default_socket_path() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push("gatemini.sock");
    path
}

#[cfg(not(unix))]
pub fn is_daemon_alive(_socket: &Path) -> bool {
    false
}

#[cfg(not(unix))]
pub fn read_pid(_socket: &Path) -> Option<i32> {
    None
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_is_deterministic() {
        let path = default_socket_path();
        // On Linux with XDG_RUNTIME_DIR, name is gatemini.sock
        // On macOS/fallback, name is gatemini-$UID.sock
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("gatemini"));
        assert!(name.ends_with(".sock"));
    }

    #[test]
    fn sibling_paths() {
        let sock = PathBuf::from("/tmp/gatemini.sock");
        assert_eq!(lock_path(&sock), PathBuf::from("/tmp/gatemini.lock"));
        assert_eq!(pid_path(&sock), PathBuf::from("/tmp/gatemini.pid"));
    }

    #[test]
    fn daemon_log_path_is_stable_cache_file() {
        let path = daemon_log_path();
        assert_eq!(path.file_name().unwrap(), "daemon.log");
        assert!(
            path.components().any(|c| c.as_os_str() == "gatemini"),
            "daemon log should live under a gatemini cache directory"
        );
    }

    #[test]
    fn upgrade_paths_are_deterministic_and_pid_scoped() {
        let sock = PathBuf::from("/tmp/gatemini-503.sock");

        assert_eq!(
            staged_socket_path(&sock, 42),
            PathBuf::from("/tmp/gatemini-503.upgrade-42.sock")
        );
        assert_eq!(
            drain_socket_path(&sock, 1234),
            PathBuf::from("/tmp/gatemini-503.drain-1234.sock")
        );
        assert_eq!(
            drain_pid_path(&sock, 1234),
            PathBuf::from("/tmp/gatemini-503.drain-1234.pid")
        );
        assert_eq!(
            generation_info_path(&sock),
            PathBuf::from("/tmp/gatemini-503.info.json")
        );
        assert_eq!(
            drain_generation_info_path(&sock, 1234),
            PathBuf::from("/tmp/gatemini-503.drain-1234.info.json")
        );
    }

    #[test]
    fn owner_cleanup_preserves_foreign_generation_files() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("gatemini.sock");
        fs::write(&sock, "").unwrap();
        fs::write(pid_path(&sock), "111").unwrap();
        fs::write(generation_info_path(&sock), "{}").unwrap();
        fs::write(lock_path(&sock), "").unwrap();

        cleanup_owned_generation_files(&sock, 222);

        assert!(sock.exists(), "foreign socket should not be removed");
        assert!(
            pid_path(&sock).exists(),
            "foreign pid should not be removed"
        );
        assert!(
            generation_info_path(&sock).exists(),
            "foreign generation info should not be removed"
        );
        assert!(lock_path(&sock).exists(), "lock file should be preserved");

        cleanup_owned_generation_files(&sock, 111);

        assert!(!sock.exists(), "owned socket should be removed");
        assert!(!pid_path(&sock).exists(), "owned pid should be removed");
        assert!(
            !generation_info_path(&sock).exists(),
            "owned generation info should be removed"
        );
        assert!(
            lock_path(&sock).exists(),
            "lock file should still be preserved"
        );
    }

    #[test]
    fn drain_generation_discovery_filters_dead_pids() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("gatemini.sock");
        let alive_pid = std::process::id() as i32;
        let dead_pid = 999_999;

        fs::write(drain_pid_path(&sock, alive_pid), alive_pid.to_string()).unwrap();
        fs::write(drain_generation_info_path(&sock, alive_pid), "{}").unwrap();
        fs::write(drain_pid_path(&sock, dead_pid), dead_pid.to_string()).unwrap();

        let drains = discover_drain_generations(&sock);

        assert_eq!(drains.len(), 1);
        assert_eq!(drains[0].pid, alive_pid);
        assert!(
            drains[0]
                .pid_path
                .ends_with(format!("gatemini.drain-{alive_pid}.pid"))
        );
    }

    #[test]
    fn nonexistent_pid_reports_dead() {
        let sock = PathBuf::from("/tmp/gatemini-test-nonexistent.sock");
        assert!(!is_daemon_alive(&sock));
    }
}
