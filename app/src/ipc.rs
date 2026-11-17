//! Unix socket server/client for `chronoclip save` and single-instance detection.

use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use chronoclip_core::ipc::{Request, Response};

use crate::controller::Controller;

const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LINE: u64 = 64 * 1024;

pub enum Bound {
    Listener(UnixListener),
    /// Another instance answers on the socket.
    AlreadyRunning,
}

/// Bind the socket, or detect that another instance already owns it.
/// A stale socket file left by a crashed instance is replaced.
pub fn bind(path: &Path) -> io::Result<Bound> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let listener = match UnixListener::bind(path) {
        Ok(listener) => listener,
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            if UnixStream::connect(path).is_ok() {
                return Ok(Bound::AlreadyRunning);
            }
            tracing::debug!("removing stale socket {}", path.display());
            fs::remove_file(path)?;
            UnixListener::bind(path)?
        }
        Err(e) => return Err(e),
    };
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(Bound::Listener(listener))
}

/// Accept connections on a background thread and pass requests to the controller.
pub fn serve(listener: UnixListener, controller: Arc<Controller>) -> io::Result<()> {
    thread::Builder::new()
        .name("ipc".into())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        if let Err(e) = handle_connection(&stream, &controller) {
                            tracing::debug!("IPC connection failed: {e}");
                        }
                    }
                    Err(e) => tracing::warn!("IPC accept failed: {e}"),
                }
            }
        })
        .map(drop)
}

fn handle_connection(stream: &UnixStream, controller: &Controller) -> io::Result<()> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let line = read_line(stream)?;
    let response = match line.parse::<Request>() {
        Ok(request) => {
            tracing::debug!(%request, "IPC request");
            controller.handle(request)
        }
        Err(e) => Response::Error(e.to_string()),
    };
    let mut writer = stream;
    writeln!(writer, "{response}")
}

/// Send one request to the running instance and wait for its response.
pub fn send(path: &Path, request: Request) -> io::Result<Response> {
    let stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut writer = &stream;
    writeln!(writer, "{request}")?;
    let line = read_line(&stream)?;
    line.parse()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn read_line(stream: &UnixStream) -> io::Result<String> {
    let mut line = String::new();
    BufReader::new(stream.take(MAX_LINE)).read_line(&mut line)?;
    Ok(line)
}

/// True if the error means no instance is listening.
pub fn is_not_running(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

pub fn cleanup(path: &Path) {
    if let Err(e) = fs::remove_file(path)
        && e.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!("could not remove {}: {e}", path.display());
    }
}
