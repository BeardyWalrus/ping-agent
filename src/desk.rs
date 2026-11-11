//! The PC's lock state, posted to the desk box (local-docker's
//! `POST /api/desk/presence`), so the desk lamp knows when you're at the PC.
//!
//! A post goes out at once when the session locks or unlocks, and again every
//! 30 seconds with the current state. A failed post isn't retried early: the
//! next heartbeat is the retry. The box drops the state after 2 minutes with
//! no post, which covers sleep and shutdown.
//!
//! The box's reply can carry a `notice` (for example "Desk air is stuffy").
//! It repeats the same notice, with the same id, for about 2 minutes, so the
//! reporter shows each id once and hands it to the UI thread to put up as a
//! tray notification.

use std::collections::VecDeque;
use std::io::Read;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::Config;

/// How often the current state is posted again.
pub const HEARTBEAT: Duration = Duration::from_secs(30);
/// How long a post may take before the box counts as not answering.
pub const TIMEOUT: Duration = Duration::from_secs(3);
/// The most of a reply body that is read; a longer one carries no notice.
pub const MAX_REPLY_BYTES: u64 = 16 * 1024;
/// Notice ids remembered, so a repeated notice isn't shown again.
pub const SEEN_IDS: usize = 16;
/// Longest notice id kept, in characters.
pub const NOTICE_ID_MAX: usize = 64;
/// Longest notice title, in UTF-16 units: the shell's `szInfoTitle` less its NUL.
pub const NOTICE_TITLE_MAX: usize = 63;
/// Longest notice text, in UTF-16 units: the shell's `szInfo` less its NUL.
pub const NOTICE_TEXT_MAX: usize = 255;
/// Longest path a notice may link to.
pub const NOTICE_PATH_MAX: usize = 512;

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

/// A notice from the box, to show once as a tray notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeskNotice {
    pub id: String,
    pub title: String,
    pub text: String,
    /// A page on the box to open when the notification is clicked, such as
    /// `/air`. Checked by `notice_link` before it's used.
    pub path: Option<String>,
}

/// What a post came to: the status for the menu, and a notice if the box
/// sent one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: DeskStatus,
    pub notice: Option<DeskNotice>,
}

impl From<DeskStatus> for Reply {
    fn from(status: DeskStatus) -> Reply {
        Reply {
            status,
            notice: None,
        }
    }
}

/// The `notice` in a reply body, if there is a usable one. Anything else
/// (not JSON, no notice, not an object, a missing or empty id, title or
/// text) is no notice.
pub fn parse_notice(body: &str) -> Option<DeskNotice> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let notice = value.get("notice")?.as_object()?;
    let field = |name: &str| notice.get(name).and_then(serde_json::Value::as_str);
    let id = cap(clean(field("id")?, false), NOTICE_ID_MAX, false);
    let title = cap(clean(field("title")?, false), NOTICE_TITLE_MAX, true);
    let text = cap(clean(field("text")?, true), NOTICE_TEXT_MAX, true);
    if id.is_empty() || title.is_empty() || text.is_empty() {
        return None;
    }
    let path = field("path")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    Some(DeskNotice {
        id,
        title,
        text,
        path,
    })
}

/// Trim, and turn control characters into spaces (keeping line breaks when
/// asked), so the notification shows plain text.
fn clean(s: &str, keep_newlines: bool) -> String {
    s.trim()
        .chars()
        .map(|c| {
            if c.is_control() && !(keep_newlines && c == '\n') {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Cut `s` to at most `max` UTF-16 units without splitting a character,
/// ending with "…" when it was cut and `ellipsis` is set.
fn cap(s: String, max: usize, ellipsis: bool) -> String {
    if s.encode_utf16().count() <= max {
        return s;
    }
    let room = if ellipsis { max - 1 } else { max };
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        if used + c.len_utf16() > room {
            break;
        }
        out.push(c);
        used += c.len_utf16();
    }
    if ellipsis {
        out = out.trim_end().to_string();
        out.push('…');
    }
    out
}

/// A path on the box that's safe to put after its address: one leading `/`,
/// and no `//`, `\`, `:`, spaces or control characters, so it can't name
/// another host or scheme.
pub fn path_is_safe(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= NOTICE_PATH_MAX
        && !path.contains("//")
        && !path.contains('\\')
        && !path.contains(':')
        && !path.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// The page a click on a notice opens: the box's address plus the notice's
/// path, or nothing when either fails its check.
pub fn notice_link(base: &str, path: Option<&str>) -> Option<String> {
    let path = path?;
    if !path_is_safe(path) || check_url(base).is_err() {
        return None;
    }
    Some(format!("{}{}", base.trim().trim_end_matches('/'), path))
}

/// Remembers the last few notice ids shown.
#[derive(Debug, Default)]
pub struct SeenIds(VecDeque<String>);

impl SeenIds {
    /// True the first time an id comes by, false while it's among the last
    /// `SEEN_IDS` seen.
    pub fn first_time(&mut self, id: &str) -> bool {
        if self.0.iter().any(|seen| seen == id) {
            return false;
        }
        if self.0.len() >= SEEN_IDS {
            self.0.pop_front();
        }
        self.0.push_back(id.to_string());
        true
    }
}

pub fn status_for(code: u16, locked: bool) -> DeskStatus {
    match code {
        200..=299 => DeskStatus::Sent { locked },
        401 => DeskStatus::TokenWrong,
        503 => DeskStatus::NoTokenOnBox,
        other => DeskStatus::Refused(other),
    }
}

/// Post the lock state once, and read a notice from a 2xx reply. Never
/// panics and never retries.
pub fn post(base: &str, token: &str, locked: bool) -> Reply {
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
        Ok(response) => {
            let status = status_for(response.status(), locked);
            let notice = match status {
                DeskStatus::Sent { .. } => read_body(response).as_deref().and_then(parse_notice),
                _ => None,
            };
            Reply { status, notice }
        }
        Err(ureq::Error::Status(code, _)) => status_for(code, locked).into(),
        Err(ureq::Error::Transport(_)) => DeskStatus::NotAnswering.into(),
    }
}

/// The reply body, up to `MAX_REPLY_BYTES`; nothing if it can't be read.
fn read_body(response: ureq::Response) -> Option<String> {
    let mut text = String::new();
    response
        .into_reader()
        .take(MAX_REPLY_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
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
/// fake in tests). A notice id not seen before is put in `notices` for the
/// UI thread to take, when `desk_notices` is on. `changed` is called after
/// every new status or new notice.
pub fn run_reporter(
    shared: Arc<Mutex<Config>>,
    rx: Receiver<DeskMsg>,
    status: Arc<Mutex<DeskStatus>>,
    notices: Arc<Mutex<Option<DeskNotice>>>,
    heartbeat: Duration,
    send: impl Fn(&str, &str, bool) -> Reply,
    changed: impl Fn(),
) {
    // The app starts at sign-in, in an unlocked session.
    let mut locked = false;
    let mut seen = SeenIds::default();
    loop {
        let cfg = shared.lock().unwrap().clone();
        let reply = if cfg.desk_enabled && !cfg.desk_url.is_empty() {
            send(&cfg.desk_url, &cfg.desk_token, locked)
        } else {
            DeskStatus::Off.into()
        };
        let differs = {
            let mut s = status.lock().unwrap();
            let differs = *s != reply.status;
            *s = reply.status;
            differs
        };
        let mut fresh = false;
        if let (true, Some(notice)) = (cfg.desk_notices, reply.notice) {
            if seen.first_time(&notice.id) {
                *notices.lock().unwrap() = Some(notice);
                fresh = true;
            }
        }
        if differs || fresh {
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

    const AIR_REPLY: &str = r#"{"ok": true, "busy": false, "quiet": false, "mode": "normal", "locked": false,
        "notice": {"id": "co2-1794405600", "title": "Desk air is stuffy",
                   "text": "CO2 has been over 1200 ppm for 10 minutes (now 1340). Open a window or door for a few minutes.",
                   "path": "/air"}}"#;

    fn notice(id: &str) -> DeskNotice {
        DeskNotice {
            id: id.into(),
            title: "Desk air is stuffy".into(),
            text: "Open a window.".into(),
            path: Some("/air".into()),
        }
    }

    #[test]
    fn reads_a_notice_from_the_reply() {
        assert_eq!(
            parse_notice(AIR_REPLY),
            Some(DeskNotice {
                id: "co2-1794405600".into(),
                title: "Desk air is stuffy".into(),
                text: "CO2 has been over 1200 ppm for 10 minutes (now 1340). Open a window or door for a few minutes.".into(),
                path: Some("/air".into()),
            })
        );
        // No path is still a notice, with no link.
        let n = parse_notice(r#"{"notice": {"id": "a", "title": "T", "text": "X"}}"#).unwrap();
        assert_eq!(n.path, None);
        // A path that isn't a string, or is blank, is no path.
        let n = parse_notice(r#"{"notice": {"id": "a", "title": "T", "text": "X", "path": 5}}"#)
            .unwrap();
        assert_eq!(n.path, None);
        let n = parse_notice(r#"{"notice": {"id": "a", "title": "T", "text": "X", "path": " "}}"#)
            .unwrap();
        assert_eq!(n.path, None);
    }

    #[test]
    fn a_reply_without_a_notice_has_none() {
        assert_eq!(parse_notice(r#"{"ok": true, "locked": false}"#), None);
        assert_eq!(parse_notice(r#"{"ok": true, "notice": null}"#), None);
        assert_eq!(parse_notice("{}"), None);
    }

    #[test]
    fn a_garbage_body_has_no_notice() {
        assert_eq!(parse_notice(""), None);
        assert_eq!(parse_notice("<html>502 Bad Gateway</html>"), None);
        assert_eq!(parse_notice(r#"{"notice": {"id": "a""#), None);
        assert_eq!(parse_notice("[1, 2]"), None);
    }

    #[test]
    fn bad_or_empty_fields_mean_no_notice() {
        for body in [
            r#"{"notice": "Desk air is stuffy"}"#,
            r#"{"notice": ["a", "T", "X"]}"#,
            r#"{"notice": {"title": "T", "text": "X"}}"#,
            r#"{"notice": {"id": "a", "text": "X"}}"#,
            r#"{"notice": {"id": "a", "title": "T"}}"#,
            r#"{"notice": {"id": "", "title": "T", "text": "X"}}"#,
            r#"{"notice": {"id": "a", "title": "  ", "text": "X"}}"#,
            r#"{"notice": {"id": "a", "title": "T", "text": ""}}"#,
            r#"{"notice": {"id": 7, "title": "T", "text": "X"}}"#,
            r#"{"notice": {"id": "a", "title": true, "text": "X"}}"#,
        ] {
            assert_eq!(parse_notice(body), None, "{body}");
        }
    }

    #[test]
    fn fields_are_trimmed_and_capped() {
        let body = serde_json::json!({"notice": {
            "id": format!(" {} ", "i".repeat(100)),
            "title": format!("  {}  ", "t".repeat(100)),
            "text": "x".repeat(400),
            "path": " /air ",
        }})
        .to_string();
        let n = parse_notice(&body).unwrap();
        assert_eq!(n.id, "i".repeat(NOTICE_ID_MAX));
        assert_eq!(n.title.encode_utf16().count(), NOTICE_TITLE_MAX);
        assert!(n.title.ends_with('…'));
        assert_eq!(n.text.encode_utf16().count(), NOTICE_TEXT_MAX);
        assert!(n.text.ends_with('…'));
        assert_eq!(n.path.as_deref(), Some("/air"));

        // Characters outside the BMP take two UTF-16 units and aren't split.
        let body = serde_json::json!({"notice": {
            "id": "a", "title": "\u{1F32C}".repeat(40), "text": "X",
        }})
        .to_string();
        let n = parse_notice(&body).unwrap();
        assert!(n.title.encode_utf16().count() <= NOTICE_TITLE_MAX);
        assert_eq!(n.title, format!("{}…", "\u{1F32C}".repeat(31)));

        // Fields that fit are left as they are; control characters become
        // spaces, and the text keeps its line breaks.
        let body = serde_json::json!({"notice": {
            "id": "a", "title": "Desk\tair", "text": "Line one\nLine two\u{7}",
        }})
        .to_string();
        let n = parse_notice(&body).unwrap();
        assert_eq!(n.title, "Desk air");
        assert_eq!(n.text, "Line one\nLine two");
    }

    #[test]
    fn only_a_safe_path_makes_a_link() {
        let base = "http://192.168.86.20:3006/";
        assert_eq!(
            notice_link(base, Some("/air")).as_deref(),
            Some("http://192.168.86.20:3006/air")
        );
        assert_eq!(
            notice_link(" http://box:3006 ", Some("/air?range=day")).as_deref(),
            Some("http://box:3006/air?range=day")
        );
        for bad in [
            "//evil.com",
            "/a//b",
            "http://x",
            "/a b",
            "air",
            "/a\\b",
            "/a:b",
            "/a\u{0}b",
            "",
        ] {
            assert_eq!(notice_link(base, Some(bad)), None, "{bad:?}");
        }
        assert_eq!(
            notice_link(base, Some(&format!("/{}", "a".repeat(600)))),
            None
        );
        // No path, or a base that isn't the box's address: no link.
        assert_eq!(notice_link(base, None), None);
        assert_eq!(notice_link("", Some("/air")), None);
        assert_eq!(notice_link("file:///C:/", Some("/air")), None);
    }

    #[test]
    fn seen_ids_remember_the_last_sixteen() {
        let mut seen = SeenIds::default();
        assert!(seen.first_time("a"));
        assert!(!seen.first_time("a"));
        for i in 0..SEEN_IDS {
            assert!(seen.first_time(&i.to_string()));
        }
        // "a" has dropped off the end, so it counts as new again.
        assert!(seen.first_time("a"));
        assert!(!seen.first_time(&(SEEN_IDS - 1).to_string()));
    }

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
            fake::response("200 OK", AIR_REPLY.as_bytes()),
            fake::response("200 OK", b"<html>not json</html>"),
            fake::response("401 Unauthorized", b"{}"),
            fake::response("503 Service Unavailable", b"{}"),
            fake::response("422 Unprocessable Entity", AIR_REPLY.as_bytes()),
        ]);
        assert_eq!(
            post(&base, "tok", true),
            DeskStatus::Sent { locked: true }.into()
        );
        let req = rx.recv().unwrap();
        assert!(req.head.starts_with("POST /api/desk/presence "));
        assert!(req
            .head
            .lines()
            .any(|l| l.eq_ignore_ascii_case("x-desk-token: tok")));
        let sent: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(sent["locked"], true);

        // A 200 with a notice brings it back with the status.
        let reply = post(&base, "tok", false);
        assert_eq!(reply.status, DeskStatus::Sent { locked: false });
        let notice = reply.notice.unwrap();
        assert_eq!(notice.id, "co2-1794405600");
        assert_eq!(notice.title, "Desk air is stuffy");
        assert_eq!(notice.path.as_deref(), Some("/air"));

        // A body that isn't JSON is still a good post, with no notice.
        assert_eq!(
            post(&base, "tok", false),
            DeskStatus::Sent { locked: false }.into()
        );

        assert_eq!(post(&base, "bad", false), DeskStatus::TokenWrong.into());
        assert_eq!(post(&base, "tok", false), DeskStatus::NoTokenOnBox.into());
        // A notice on a refusal is ignored.
        assert_eq!(post(&base, "tok", false), DeskStatus::Refused(422).into());
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
            DeskStatus::NotAnswering.into()
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
                Arc::new(Mutex::new(None)),
                Duration::from_millis(1000),
                move |url, token, locked| {
                    sent_tx
                        .send((url.to_string(), token.to_string(), locked))
                        .unwrap();
                    DeskStatus::Sent { locked }.into()
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
                Arc::new(Mutex::new(None)),
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

    /// Run the reporter against replies carrying these notice ids, one per
    /// post (then none), and return the notices it handed over.
    fn deliveries(ids: &[Option<&str>], notices_on: bool) -> Vec<DeskNotice> {
        let cfg = Config {
            desk_enabled: true,
            desk_url: "http://box".into(),
            desk_token: "tok".into(),
            desk_notices: notices_on,
            ..Config::default()
        };
        let shared = Arc::new(Mutex::new(cfg));
        let status = Arc::new(Mutex::new(DeskStatus::Starting));
        let slot = Arc::new(Mutex::new(None));
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let replies: Vec<Reply> = ids
            .iter()
            .map(|id| Reply {
                status: DeskStatus::Sent { locked: false },
                notice: id.map(notice),
            })
            .collect();
        let (tx, rx) = channel();
        let (posted_tx, posted_rx) = channel();
        let (slot2, delivered2) = (slot.clone(), delivered.clone());
        let worker = std::thread::spawn(move || {
            let count = std::cell::Cell::new(0);
            run_reporter(
                shared,
                rx,
                status,
                slot2.clone(),
                Duration::from_millis(5),
                move |_, _, _| {
                    let n = count.get();
                    count.set(n + 1);
                    let _ = posted_tx.send(n + 1);
                    replies
                        .get(n)
                        .cloned()
                        .unwrap_or_else(|| DeskStatus::Sent { locked: false }.into())
                },
                // What the UI thread does on the poke: take the notice.
                move || {
                    if let Some(n) = slot2.lock().unwrap().take() {
                        delivered2.lock().unwrap().push(n);
                    }
                },
            )
        });
        let wait = Duration::from_secs(3);
        while posted_rx.recv_timeout(wait).unwrap() < ids.len() + 1 {}
        tx.send(DeskMsg::Quit).unwrap();
        worker.join().unwrap();
        assert!(slot.lock().unwrap().is_none());
        Arc::try_unwrap(delivered).unwrap().into_inner().unwrap()
    }

    #[test]
    fn reporter_hands_over_each_notice_once() {
        let got = deliveries(
            &[Some("a"), Some("a"), None, Some("a"), Some("b"), Some("b")],
            true,
        );
        let ids: Vec<&str> = got.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn reporter_ignores_notices_when_they_are_off() {
        assert!(deliveries(&[Some("a"), Some("b")], false).is_empty());
    }
}
