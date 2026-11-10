//! The PC's lock state, posted to the desk box (local-docker's
//! `POST /api/desk/presence`), so the desk lamp knows when you're at the PC.
//!
//! A post goes out at once when the session locks or unlocks, and again every
//! 30 seconds with the current state. A failed post isn't retried early: the
//! next heartbeat is the retry. The box drops the state after 2 minutes with
//! no post, which covers sleep and shutdown.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::Config;

/// How often the current state is posted again.
pub const HEARTBEAT: Duration = Duration::from_secs(30);
/// How long a post may take before the box counts as not answering.
pub const TIMEOUT: Duration = Duration::from_secs(3);

/// What the last post came to, for the tray menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeskStatus {
    /// Reporting is off in Settings.
    Off,
    /// On, but nothing posted yet.
    Starting,
    /// The box took the last post.
    Sent { locked: bool },
    /// 401: the token doesn't match the box's.
    TokenWrong,
    /// 503: the box has no desk token set.
    NoTokenOnBox,
    /// Any other HTTP status.
    Refused(u16),
    /// No answer, or not an HTTP answer.
    NotAnswering,
}

impl DeskStatus {
    pub fn menu_line(&self) -> String {
        match self {
            DeskStatus::Off => "Desk: off".into(),
            DeskStatus::Starting => "Desk: starting".into(),
            DeskStatus::Sent { locked: true } => "Desk: locked, sent".into(),
            DeskStatus::Sent { locked: false } => "Desk: unlocked, sent".into(),
            DeskStatus::TokenWrong => "Desk: token wrong".into(),
            DeskStatus::NoTokenOnBox => "Desk: no token set on the box".into(),
            DeskStatus::Refused(code) => format!("Desk: the box said HTTP {code}"),
            DeskStatus::NotAnswering => "Desk: box not answering".into(),
        }
    }
}

/// An address the box can be reached at: `http://` or `https://` and a host.
pub fn check_url(url: &str) -> Result<(), String> {
    let url = url.trim();
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(|| {
            "start it with http://, for example http://192.168.86.20:3006".to_string()
        })?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || host.starts_with(':') {
        return Err("it needs the box's name or IP address after http://".into());
    }
    if url.chars().any(char::is_whitespace) {
        return Err("it must not contain spaces".into());
    }
    Ok(())
}

pub fn presence_url(base: &str) -> String {
    format!("{}/api/desk/presence", base.trim().trim_end_matches('/'))
}

/// Only `locked` is sent, so the box keeps whatever else reports `busy`.
pub fn body(locked: bool) -> String {
    serde_json::json!({ "locked": locked, "source": "pc" }).to_string()
}

pub fn status_for(code: u16, locked: bool) -> DeskStatus {
    match code {
        200..=299 => DeskStatus::Sent { locked },
        401 => DeskStatus::TokenWrong,
        503 => DeskStatus::NoTokenOnBox,
        other => DeskStatus::Refused(other),
    }
}

/// Post the lock state once. Never panics and never retries.
pub fn post(base: &str, token: &str, locked: bool) -> DeskStatus {
    let agent = ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .redirects(0)
        .build();
    let result = agent
        .post(&presence_url(base))
        .set("X-Desk-Token", token)
        .set("Content-Type", "application/json")
        .send_string(&body(locked));
    match result {
        Ok(response) => status_for(response.status(), locked),
        Err(ureq::Error::Status(code, _)) => status_for(code, locked),
        Err(ureq::Error::Transport(_)) => DeskStatus::NotAnswering,
    }
}

/// Messages to the reporter thread.
pub enum DeskMsg {
    /// The session locked (true) or unlocked (false).
    Locked(bool),
    /// Settings changed: post now with the new ones.
    ConfigChanged,
    Quit,
}

/// The reporter thread's loop. `send` does the post (`post` in the app, a
/// fake in tests); `changed` is called after every new status.
pub fn run_reporter(
    shared: Arc<Mutex<Config>>,
    rx: Receiver<DeskMsg>,
    status: Arc<Mutex<DeskStatus>>,
    heartbeat: Duration,
    send: impl Fn(&str, &str, bool) -> DeskStatus,
    changed: impl Fn(),
) {
    // The app starts at sign-in, in an unlocked session.
    let mut locked = false;
    loop {
        let cfg = shared.lock().unwrap().clone();
        let new = if cfg.desk_enabled && !cfg.desk_url.is_empty() {
            send(&cfg.desk_url, &cfg.desk_token, locked)
        } else {
            DeskStatus::Off
        };
        let differs = {
            let mut s = status.lock().unwrap();
            let differs = *s != new;
            *s = new;
            differs
        };
        if differs {
            changed();
        }
        match rx.recv_timeout(heartbeat) {
            Ok(DeskMsg::Locked(now)) => locked = now,
            Ok(DeskMsg::ConfigChanged) | Err(RecvTimeoutError::Timeout) => {}
            Ok(DeskMsg::Quit) | Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A one-shot HTTP server on localhost, for tests of the real client.

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc::{channel, Receiver};

    pub struct Request {
        pub head: String,
        pub body: Vec<u8>,
    }

    /// Serve the given raw responses, one per connection, in order. Returns
    /// the base address and the requests as they arrive.
    pub fn serve(responses: Vec<Vec<u8>>) -> (String, Receiver<Request>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                // Read the head, then as much body as Content-Length says.
                let (head, mut body) = loop {
                    let n = stream.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break (String::from_utf8_lossy(&buf).to_string(), Vec::new());
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                        break (head, buf[pos + 4..].to_vec());
                    }
                };
                let want = head
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                while body.len() < want {
                    let n = stream.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    body.extend_from_slice(&chunk[..n]);
                }
                let _ = tx.send(Request { head, body });
                let _ = stream.write_all(&response);
            }
        });
        (base, rx)
    }

    pub fn response(status: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn urls_are_checked() {
        assert!(check_url("http://192.168.86.20:3006").is_ok());
        assert!(check_url("https://box.local/").is_ok());
        assert!(check_url("192.168.86.20:3006").is_err());
        assert!(check_url("http://").is_err());
        assert!(check_url("http://:3006").is_err());
        assert!(check_url("ftp://box").is_err());
        assert!(check_url("http://my box").is_err());
    }

    #[test]
    fn presence_url_drops_a_trailing_slash() {
        assert_eq!(
            presence_url("http://box:3006/"),
            "http://box:3006/api/desk/presence"
        );
    }

    #[test]
    fn body_has_only_locked_and_source() {
        let v: serde_json::Value = serde_json::from_str(&body(true)).unwrap();
        assert_eq!(v, serde_json::json!({"locked": true, "source": "pc"}));
    }

    #[test]
    fn menu_lines_name_the_problem() {
        assert_eq!(DeskStatus::TokenWrong.menu_line(), "Desk: token wrong");
        assert_eq!(
            DeskStatus::NoTokenOnBox.menu_line(),
            "Desk: no token set on the box"
        );
        assert_eq!(
            DeskStatus::NotAnswering.menu_line(),
            "Desk: box not answering"
        );
    }

    #[test]
    fn posts_with_the_token_and_reads_each_answer() {
        let (base, rx) = fake::serve(vec![
            fake::response("200 OK", br#"{"ok":true,"locked":true}"#),
            fake::response("401 Unauthorized", b"{}"),
            fake::response("503 Service Unavailable", b"{}"),
            fake::response("422 Unprocessable Entity", b"{}"),
        ]);
        assert_eq!(post(&base, "tok", true), DeskStatus::Sent { locked: true });
        let req = rx.recv().unwrap();
        assert!(req.head.starts_with("POST /api/desk/presence "));
        assert!(req
            .head
            .lines()
            .any(|l| l.eq_ignore_ascii_case("x-desk-token: tok")));
        let sent: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(sent["locked"], true);

        assert_eq!(post(&base, "bad", false), DeskStatus::TokenWrong);
        assert_eq!(post(&base, "tok", false), DeskStatus::NoTokenOnBox);
        assert_eq!(post(&base, "tok", false), DeskStatus::Refused(422));
    }

    #[test]
    fn no_answer_is_not_answering() {
        // Bind and drop, so nothing listens on the port.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert_eq!(
            post(&format!("http://127.0.0.1:{port}"), "tok", false),
            DeskStatus::NotAnswering
        );
    }

    #[test]
    fn reporter_posts_at_once_on_a_change_and_on_the_heartbeat() {
        let cfg = Config {
            desk_enabled: true,
            desk_url: "http://box".into(),
            desk_token: "tok".into(),
            ..Config::default()
        };
        let shared = Arc::new(Mutex::new(cfg));
        let status = Arc::new(Mutex::new(DeskStatus::Starting));
        let (tx, rx) = channel();
        let (sent_tx, sent_rx) = channel();
        let s2 = status.clone();
        let worker = std::thread::spawn(move || {
            run_reporter(
                shared,
                rx,
                s2,
                Duration::from_millis(1000),
                move |url, token, locked| {
                    sent_tx
                        .send((url.to_string(), token.to_string(), locked))
                        .unwrap();
                    DeskStatus::Sent { locked }
                },
                || {},
            )
        });
        let wait = Duration::from_secs(3);
        // At start: unlocked.
        assert_eq!(
            sent_rx.recv_timeout(wait).unwrap(),
            ("http://box".into(), "tok".into(), false)
        );
        // A lock goes out straight away, well inside a heartbeat.
        let t = std::time::Instant::now();
        tx.send(DeskMsg::Locked(true)).unwrap();
        assert!(sent_rx.recv_timeout(wait).unwrap().2);
        assert!(t.elapsed() < Duration::from_millis(500));
        // Then the heartbeat repeats it.
        assert!(sent_rx.recv_timeout(wait).unwrap().2);
        assert_eq!(*status.lock().unwrap(), DeskStatus::Sent { locked: true });
        tx.send(DeskMsg::Quit).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn reporter_posts_nothing_when_off() {
        let shared = Arc::new(Mutex::new(Config::default()));
        let status = Arc::new(Mutex::new(DeskStatus::Starting));
        let (tx, rx) = channel();
        let s2 = status.clone();
        let worker = std::thread::spawn(move || {
            run_reporter(
                shared,
                rx,
                s2,
                Duration::from_millis(50),
                |_, _, _| panic!("posted while off"),
                || {},
            )
        });
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(*status.lock().unwrap(), DeskStatus::Off);
        tx.send(DeskMsg::Quit).unwrap();
        worker.join().unwrap();
    }
}
