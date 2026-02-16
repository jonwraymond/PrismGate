use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use nix::unistd::getuid;

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

/// Path to the PID file (sibling of the socket).
#[cfg(unix)]
pub fn pid_path(socket: &Path) -> PathBuf {
    socket.with_extension("pid")
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
    let pid = nix::unistd::Pid::from_raw(pid);
    nix::sys::signal::kill(pid, None).is_ok()
}

/// Read the daemon PID from the PID file.
#[cfg(unix)]
pub fn read_pid(socket: &Path) -> Option<i32> {
    let contents = fs::read_to_string(pid_path(socket)).ok()?;
    contents.trim().parse().ok()
}

/// Remove stale socket, lock, and PID files.
pub fn cleanup_files(socket: &Path) {
    #[cfg(unix)]
    let _ = fs::remove_file(socket);
    #[cfg(unix)]
    let _ = fs::remove_file(lock_path(socket));
    #[cfg(unix)]
    let _ = fs::remove_file(pid_path(socket));
    #[cfg(not(unix))]
    {
        let _ = std::fs::remove_file(socket);
    }
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
    fn nonexistent_pid_reports_dead() {
        let sock = PathBuf::from("/tmp/gatemini-test-nonexistent.sock");
        assert!(!is_daemon_alive(&sock));
    }
}
