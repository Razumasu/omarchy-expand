//! The daemon's local socket. The picker asks the running daemon to paste a combo, so
//! pasting goes through the daemon's virtual keyboard and terminal detection.
//!
//! One request per connection, one line each way: `paste <keyword>` is answered with
//! `ok` or `error <message>`. Keywords never contain whitespace, so no quoting is needed.
//! The socket lives in XDG_RUNTIME_DIR, which only the user can reach.

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use crate::Event;

/// Longer lines are not requests the picker would send.
const MAX_REQUEST_BYTES: u64 = 4096;

/// The daemon waits up to two seconds for the menu to close and then pastes, so a reply
/// taking longer than this means the daemon is stuck; the picker must not hang with it.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

fn socket_path() -> io::Result<PathBuf> {
    env::var_os("XDG_RUNTIME_DIR")
        .map(|dir| PathBuf::from(dir).join("omarchy-expand.sock"))
        .ok_or_else(|| io::Error::other("XDG_RUNTIME_DIR is not set"))
}

/// Starts answering requests, or fails if another daemon already answers them.
pub fn listen(events: Sender<Event>) -> io::Result<()> {
    let path = socket_path()?;
    if UnixStream::connect(&path).is_ok() {
        return Err(io::Error::other(
            "another omarchy-expand daemon is already running",
        ));
    }
    // Nobody answered, so any socket file left is from a daemon that crashed.
    match fs::remove_file(&path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let listener = UnixListener::bind(&path)?;
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if let Err(error) = answer(stream, &events) {
                eprintln!("omarchy-expand: picker request failed: {error}");
            }
        }
    });
    Ok(())
}

fn answer(stream: UnixStream, events: &Sender<Event>) -> io::Result<()> {
    let mut line = String::new();
    BufReader::new((&stream).take(MAX_REQUEST_BYTES)).read_line(&mut line)?;
    if line.is_empty() {
        return Ok(()); // a reachability check that hung up without asking anything
    }
    let result = match parse_request(&line) {
        Some(keyword) => {
            let (reply, outcome) = mpsc::channel();
            let event = Event::Paste {
                keyword: keyword.to_owned(),
                reply,
            };
            if events.send(event).is_err() {
                return Ok(());
            }
            outcome
                .recv()
                .unwrap_or_else(|_| Err("the daemon is shutting down".into()))
        }
        None => Err(format!("unknown request {:?}", line.trim_end())),
    };
    let response = match result {
        Ok(()) => "ok\n".to_owned(),
        Err(message) => format!("error {message}\n"),
    };
    (&stream).write_all(response.as_bytes())
}

fn parse_request(line: &str) -> Option<&str> {
    let keyword = line
        .strip_suffix('\n')
        .unwrap_or(line)
        .strip_prefix("paste ")?;
    (!keyword.is_empty() && !keyword.contains(char::is_whitespace)).then_some(keyword)
}

pub fn daemon_reachable() -> bool {
    socket_path().is_ok_and(|path| UnixStream::connect(path).is_ok())
}

/// Asks the daemon to paste the combo and waits until it has.
pub fn request_paste(keyword: &str) -> Result<(), String> {
    let path = socket_path().map_err(|error| error.to_string())?;
    let mut stream = UnixStream::connect(&path)
        .map_err(|_| "the omarchy-expand daemon is not running".to_owned())?;
    stream
        .set_read_timeout(Some(REPLY_TIMEOUT))
        .map_err(|error| error.to_string())?;
    stream
        .write_all(format!("paste {keyword}\n").as_bytes())
        .map_err(|error| error.to_string())?;
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .map_err(|error| match error.kind() {
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => format!(
                "the daemon did not answer within {} seconds",
                REPLY_TIMEOUT.as_secs()
            ),
            _ => error.to_string(),
        })?;
    match response.trim_end() {
        "ok" => Ok(()),
        other => Err(other.strip_prefix("error ").unwrap_or(other).to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_name_one_keyword() {
        assert_eq!(parse_request("paste @@mail\n"), Some("@@mail"));
        assert_eq!(parse_request("paste blåbær"), Some("blåbær"));
        assert_eq!(parse_request("paste \n"), None);
        assert_eq!(parse_request("paste a b\n"), None);
        assert_eq!(parse_request("delete everything\n"), None);
    }
}
