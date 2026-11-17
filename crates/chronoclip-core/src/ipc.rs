//! Line-based protocol between `chronoclip <command>` and the running instance.
//!
//! The client sends one request line (`save\n`) and receives one response line
//! (`ok <message>\n` or `error <message>\n`).

use std::fmt;
use std::str::FromStr;

/// File name of the Unix socket inside the runtime directory.
pub const SOCKET_NAME: &str = "chronoclip.sock";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// Check that the instance is alive.
    Ping,
    /// Show the settings window.
    Show,
    /// Save the replay buffer to a clip.
    Save,
    /// Quit the running instance.
    Quit,
}

impl Request {
    pub const fn as_str(self) -> &'static str {
        match self {
            Request::Ping => "ping",
            Request::Show => "show",
            Request::Save => "save",
            Request::Quit => "quit",
        }
    }
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    #[error("unknown request `{0}`")]
    UnknownRequest(String),
    #[error("malformed response `{0}`")]
    MalformedResponse(String),
}

impl FromStr for Request {
    type Err = ProtocolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "ping" => Ok(Request::Ping),
            "show" => Ok(Request::Show),
            "save" => Ok(Request::Save),
            "quit" => Ok(Request::Quit),
            other => Err(ProtocolError::UnknownRequest(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Ok(String),
    Error(String),
}

impl fmt::Display for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (tag, msg) = match self {
            Response::Ok(msg) => ("ok", msg),
            Response::Error(msg) => ("error", msg),
        };
        // Keep the response on a single line.
        let msg = msg.replace(['\n', '\r'], " ");
        if msg.is_empty() {
            f.write_str(tag)
        } else {
            write!(f, "{tag} {msg}")
        }
    }
}

impl FromStr for Response {
    type Err = ProtocolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let line = s.trim_end_matches(['\n', '\r']);
        let (tag, msg) = line.split_once(' ').unwrap_or((line, ""));
        match tag {
            "ok" => Ok(Response::Ok(msg.to_owned())),
            "error" => Ok(Response::Error(msg.to_owned())),
            _ => Err(ProtocolError::MalformedResponse(line.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        for req in [Request::Ping, Request::Show, Request::Save, Request::Quit] {
            assert_eq!(req.to_string().parse::<Request>(), Ok(req));
        }
        assert_eq!(" save\n".parse::<Request>(), Ok(Request::Save));
        assert!("delete-everything".parse::<Request>().is_err());
    }

    #[test]
    fn response_roundtrip() {
        for resp in [
            Response::Ok("saved /home/me/Videos/clip.mp4".into()),
            Response::Ok(String::new()),
            Response::Error("not recording".into()),
        ] {
            assert_eq!(format!("{resp}\n").parse::<Response>(), Ok(resp));
        }
    }

    #[test]
    fn response_is_single_line() {
        let text = Response::Error("line one\nline two".into()).to_string();
        assert_eq!(text, "error line one line two");
    }

    #[test]
    fn malformed_response() {
        assert!("hello world".parse::<Response>().is_err());
    }
}
