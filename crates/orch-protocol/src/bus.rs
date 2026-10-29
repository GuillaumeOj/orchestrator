//! The local bus: a Unix socket with 0600 permissions and no network port
//! (NFR-3, ADR-009 decision 5). One JSON line per request and per reply.

use std::io::{BufRead, Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// macOS `sun_path` is 104 bytes including the terminating NUL.
pub const MAX_SOCKET_PATH: usize = 104;

/// Longest accepted request or reply line (a hook payload with a large tool input).
pub const MAX_LINE: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum BusError {
    /// The socket path does not fit in `sun_path`.
    PathTooLong {
        len: usize,
    },
    /// Another core answers on this socket.
    AlreadyRunning,
    /// No socket file (`ENOENT`).
    NotFound,
    /// A stale socket file with nobody listening (`ECONNREFUSED`).
    Refused,
    /// Connect or read deadline.
    Timeout,
    /// The peer closed the connection before answering.
    Eof,
    Protocol(String),
    Io(std::io::Error),
}

impl BusError {
    /// Short tag recorded in the spool's `why`.
    pub fn tag(&self) -> String {
        match self {
            BusError::PathTooLong { .. } => "socket path too long".into(),
            BusError::AlreadyRunning => "already running".into(),
            BusError::NotFound => "ENOENT".into(),
            BusError::Refused => "ECONNREFUSED".into(),
            BusError::Timeout => "deadline".into(),
            BusError::Eof => "core closed the connection".into(),
            BusError::Protocol(m) => format!("protocol: {m}"),
            BusError::Io(e) => format!("io: {e}"),
        }
    }

    fn from_io(e: std::io::Error) -> BusError {
        use std::io::ErrorKind::*;
        match e.kind() {
            NotFound => BusError::NotFound,
            ConnectionRefused => BusError::Refused,
            TimedOut | WouldBlock => BusError::Timeout,
            UnexpectedEof | ConnectionReset | BrokenPipe => BusError::Eof,
            _ => BusError::Io(e),
        }
    }
}

impl std::fmt::Display for BusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BusError::PathTooLong { len } => {
                write!(f, "socket path is {len} bytes, macOS allows at most {}", MAX_SOCKET_PATH - 1)
            }
            other => f.write_str(&other.tag()),
        }
    }
}

impl std::error::Error for BusError {}

impl From<std::io::Error> for BusError {
    fn from(e: std::io::Error) -> Self {
        BusError::from_io(e)
    }
}

/// The path must leave room for the NUL in `sun_path` (checked at start, NFR-3).
pub fn check_socket_path(path: &Path) -> Result<(), BusError> {
    let len = path.as_os_str().len();
    if len >= MAX_SOCKET_PATH { Err(BusError::PathTooLong { len }) } else { Ok(()) }
}

/// Binds the core's socket: checks the path length, removes a stale socket
/// left by a crash (nobody answers on it), refuses to steal a live one, and
/// sets 0600. The parent folder is created 0700.
pub fn bind_listener(path: &Path) -> Result<UnixListener, BusError> {
    check_socket_path(path)?;
    if let Some(dir) = path.parent() {
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_socket() => {
            if is_listening(path) {
                return Err(BusError::AlreadyRunning);
            }
            std::fs::remove_file(path)?
        }
        Ok(_) => return Err(BusError::Protocol(format!("{} exists and is not a socket", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let l = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(l)
}

/// Connects with a deadline (FR-806: ≤ 1 s). A missing file and a stale
/// socket both fail at once, as `NotFound` and `Refused`.
pub fn connect(path: &Path, timeout: Duration) -> Result<UnixStream, BusError> {
    check_socket_path(path)?;
    let addr = socket2::SockAddr::unix(path)?;
    let s = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
    s.connect_timeout(&addr, timeout).map_err(BusError::from_io)?;
    Ok(UnixStream::from(OwnedFd::from(s)))
}

/// Whether a core answers on this socket.
pub fn is_listening(path: &Path) -> bool {
    connect(path, Duration::from_millis(300)).is_ok()
}

/// Writes one JSON line.
pub fn write_line<T: Serialize>(w: &mut impl Write, msg: &T) -> Result<(), BusError> {
    let mut buf = serde_json::to_vec(msg).map_err(|e| BusError::Protocol(e.to_string()))?;
    buf.push(b'\n');
    w.write_all(&buf)?;
    w.flush()?;
    Ok(())
}

/// Reads one JSON line. EOF before the newline is [`BusError::Eof`].
pub fn read_line<T: DeserializeOwned>(r: &mut impl BufRead) -> Result<T, BusError> {
    let mut buf = Vec::new();
    let n = r.take(MAX_LINE as u64).read_until(b'\n', &mut buf)?;
    if n == 0 || buf.last() != Some(&b'\n') {
        return Err(BusError::Eof);
    }
    serde_json::from_slice(&buf).map_err(|e| BusError::Protocol(e.to_string()))
}

/// One request, one reply: connects (within `connect_timeout`), writes
/// `msg`, waits for the reply (up to `reply_timeout`, `None` = no limit) and
/// checks its protocol version.
pub fn round_trip(
    path: &Path,
    msg: &crate::Envelope,
    connect_timeout: Duration,
    reply_timeout: Option<Duration>,
) -> Result<crate::Reply, BusError> {
    let s = connect(path, connect_timeout)?;
    write_line(&mut &s, msg)?;
    s.set_read_timeout(reply_timeout)?;
    let reply: crate::ReplyEnvelope = read_line(&mut std::io::BufReader::new(&s))?;
    if reply.v != crate::PROTOCOL_VERSION {
        return Err(BusError::Protocol(format!("protocol version {}", reply.v)));
    }
    Ok(reply.reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;
    use std::os::unix::fs::PermissionsExt;

    fn short_dir() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("ob").tempdir_in(crate::test_tmp()).unwrap()
    }

    #[test]
    fn path_length_limit() {
        let long = format!("/tmp/{}", "x".repeat(100));
        assert!(matches!(check_socket_path(Path::new(&long)), Err(BusError::PathTooLong { .. })));
        assert!(check_socket_path(Path::new("/tmp/a.sock")).is_ok());
        assert!(matches!(bind_listener(Path::new(&long)), Err(BusError::PathTooLong { .. })));
    }

    #[test]
    fn bind_sets_0600_removes_stale_and_refuses_live() {
        let d = short_dir();
        let p = d.path().join("c.sock");
        // A stale socket: bound then dropped without unlink (as after kill -9).
        drop(UnixListener::bind(&p).unwrap());
        assert!(matches!(connect(&p, Duration::from_millis(200)), Err(BusError::Refused)));
        let l = bind_listener(&p).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(matches!(bind_listener(&p), Err(BusError::AlreadyRunning)));
        drop(l);
    }

    #[test]
    fn connect_missing_is_not_found() {
        let d = short_dir();
        assert!(matches!(connect(&d.path().join("none.sock"), Duration::from_secs(1)), Err(BusError::NotFound)));
    }

    #[test]
    fn line_round_trip_and_eof() {
        let d = short_dir();
        let p = d.path().join("l.sock");
        let l = bind_listener(&p).unwrap();
        let t = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let v: serde_json::Value = read_line(&mut r).unwrap();
            write_line(&mut &s, &v).unwrap();
            // Second connection: close without answering.
            let (s2, _) = l.accept().unwrap();
            drop(s2);
        });
        let mut c = connect(&p, Duration::from_secs(1)).unwrap();
        write_line(&mut c, &serde_json::json!({"a": 1})).unwrap();
        let back: serde_json::Value = read_line(&mut BufReader::new(&c)).unwrap();
        assert_eq!(back["a"], 1);
        let c2 = connect(&p, Duration::from_secs(1)).unwrap();
        let r: Result<serde_json::Value, _> = read_line(&mut BufReader::new(&c2));
        assert!(matches!(r, Err(BusError::Eof)));
        t.join().unwrap();
    }
}
