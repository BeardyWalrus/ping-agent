//! Self-update from GitHub releases (docs/adr/0001).
//!
//! Every merge to `main` publishes a release tagged `v<version>-build.<N>`,
//! where N is the CI run number, which `build.rs` also stamps into the exe.
//! An installed copy checks the latest release a minute after it starts and
//! every 6 hours; when N is greater than its own build it downloads
//! `PingAgent-Setup.exe`, checks the file's SHA-256 against the digest GitHub
//! lists for it, and runs it silently with `/RELAUNCH`. The setup stops this
//! copy, installs, and starts the new one.
//!
//! A build without a number (a local `cargo build`) is build 0 and never
//! updates. A copy running outside the install folder, or with "Update
//! automatically" off, only offers the new build in the tray menu.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::config::Config;

pub const RELEASES_API: &str =
    "https://api.github.com/repos/BeardyWalrus/ping-agent/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/BeardyWalrus/ping-agent/releases";
pub const SETUP_ASSET: &str = "PingAgent-Setup.exe";
/// What the setup is run with: no windows, no reboot, start the app after.
pub const SETUP_ARGS: [&str; 4] = [
    "/VERYSILENT",
    "/SUPPRESSMSGBOXES",
    "/NORESTART",
    "/RELAUNCH",
];

/// Wait this long after start, so sign-in isn't slowed.
pub const FIRST_CHECK: Duration = Duration::from_secs(60);
/// Then check this often.
pub const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: usize = 5;
const MAX_SETUP_BYTES: u64 = 64 * 1024 * 1024;
const USER_AGENT: &str = "PingAgent";

/// This exe's build number, 0 for a local build.
pub fn own_build() -> u32 {
    option_env!("PINGAGENT_BUILD")
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// `v0.4.0-build.57` -> 57.
pub fn parse_build(tag: &str) -> Option<u32> {
    let (_, n) = tag.trim().rsplit_once("-build.")?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse().ok().filter(|&n| n > 0)
}

/// A local build (0) never updates.
pub fn is_newer(own: u32, latest: u32) -> bool {
    own > 0 && latest > own
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub build: u32,
    pub tag: String,
    pub setup_url: String,
    /// Lower-case hex, from the asset's `digest` (`sha256:...`).
    pub sha256: Option<String>,
}

/// Read GitHub's "latest release" reply.
pub fn parse_release(json: &str) -> Result<Release, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|_| "GitHub's reply wasn't JSON".to_string())?;
    let tag = v["tag_name"]
        .as_str()
        .ok_or("the latest release has no tag")?
        .to_string();
    let build = parse_build(&tag).ok_or_else(|| format!("can't read a build number from {tag}"))?;
    let asset = v["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["name"] == SETUP_ASSET))
        .ok_or_else(|| format!("release {tag} has no {SETUP_ASSET}"))?;
    let setup_url = asset["browser_download_url"]
        .as_str()
        .ok_or_else(|| format!("{SETUP_ASSET} in {tag} has no download address"))?
        .to_string();
    let sha256 = asset["digest"]
        .as_str()
        .and_then(|d| d.strip_prefix("sha256:"))
        .map(str::to_ascii_lowercase)
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()));
    Ok(Release {
        build,
        tag,
        setup_url,
        sha256,
    })
}

/// Scheme and lower-case host of an absolute URL; None for anything odd
/// (no scheme, user info, empty host).
fn scheme_and_host(url: &str) -> Option<(&str, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') || authority.is_empty() {
        return None;
    }
    let host = authority.split(':').next()?.to_ascii_lowercase();
    (!host.is_empty()).then_some((scheme, host))
}

/// Downloads, and every redirect on the way, must stay on GitHub over HTTPS.
pub fn is_allowed_download(url: &str) -> bool {
    match scheme_and_host(url) {
        Some(("https", host)) => host == "github.com" || host.ends_with(".githubusercontent.com"),
        _ => false,
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .redirects(0)
        .user_agent(USER_AGENT)
        .build()
}

/// GET with redirects followed by hand, each hop checked with `allowed`.
fn get_checked(url: &str, allowed: &dyn Fn(&str) -> bool) -> Result<ureq::Response, String> {
    let agent = agent();
    let mut url = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        if !allowed(&url) {
            return Err("the download went somewhere other than GitHub".into());
        }
        let response = match agent.get(&url).call() {
            Ok(r) => r,
            Err(ureq::Error::Status(code, _)) => return Err(format!("GitHub said HTTP {code}")),
            Err(ureq::Error::Transport(_)) => return Err("GitHub isn't answering".into()),
        };
        if (300..400).contains(&response.status()) {
            let location = response
                .header("Location")
                .ok_or("a redirect with no address")?;
            url = if location.starts_with('/') {
                let (scheme, host) = scheme_and_host(&url).ok_or("a bad redirect")?;
                let port = url
                    .split_once("://")
                    .and_then(|(_, r)| r.split('/').next())
                    .and_then(|a| a.split_once(':'))
                    .map(|(_, p)| format!(":{p}"))
                    .unwrap_or_default();
                format!("{scheme}://{host}{port}{location}")
            } else {
                location.to_string()
            };
            continue;
        }
        return Ok(response);
    }
    Err("too many redirects".into())
}

pub fn fetch_release(api_url: &str) -> Result<Release, String> {
    let response = match agent()
        .get(api_url)
        .set("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => return Err(format!("GitHub said HTTP {code}")),
        Err(ureq::Error::Transport(_)) => return Err("GitHub isn't answering".into()),
    };
    let text = response
        .into_string()
        .map_err(|_| "GitHub's reply was cut off".to_string())?;
    parse_release(&text)
}

/// Download `release`'s setup into `dir` and check its digest. A file that
/// doesn't match, or a release with no digest, is never left behind.
pub fn fetch_verified(
    release: &Release,
    dir: &Path,
    allowed: &dyn Fn(&str) -> bool,
) -> Result<PathBuf, String> {
    let want = release
        .sha256
        .as_deref()
        .ok_or("GitHub lists no digest for the setup")?;
    let response = get_checked(&release.setup_url, allowed)?;
    let path = dir.join(format!("PingAgent-Setup-{}.exe", release.build));
    let result = (|| -> Result<String, String> {
        let mut file =
            std::fs::File::create(&path).map_err(|e| format!("can't save the setup: {e}"))?;
        let mut reader = response.into_reader().take(MAX_SETUP_BYTES + 1);
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        let mut total: u64 = 0;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|_| "the download was cut off".to_string())?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > MAX_SETUP_BYTES {
                return Err("the download is far bigger than a setup".into());
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])
                .map_err(|e| format!("can't save the setup: {e}"))?;
        }
        file.flush()
            .map_err(|e| format!("can't save the setup: {e}"))?;
        Ok(hex(&hasher.finalize()))
    })();
    match result {
        Ok(got) if got == want => Ok(path),
        Ok(_) => {
            let _ = std::fs::remove_file(&path);
            Err("the setup's digest didn't match GitHub's".into())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            Err(e)
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Setups from earlier updates, left in the temp folder.
pub fn clean_old_setups(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("PingAgent-Setup-") && name.ends_with(".exe") {
            // One still running can't be deleted; that's fine.
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Whether `exe` runs from the installer's folder,
/// `%LOCALAPPDATA%\Programs\PingAgent`.
pub fn is_installed_copy(exe: &Path, local_app_data: &Path) -> bool {
    // Compared as text, so the test runs the same off Windows.
    let norm = |p: &Path| {
        p.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    };
    let exe = norm(exe);
    let Some((dir, _)) = exe.rsplit_once('\\') else {
        return false;
    };
    dir == format!("{}\\programs\\pingagent", norm(local_app_data))
}

pub fn launch_setup(path: &Path) -> Result<(), String> {
    std::process::Command::new(path)
        .args(SETUP_ARGS)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't start the setup: {e}"))
}

/// Remember the build that last ran, and say whether this start follows an
/// update (an earlier build ran last time). The first start remembers only.
pub fn note_build(dir: &Path, own: u32) -> Option<u32> {
    if own == 0 {
        return None;
    }
    let path = dir.join("build.txt");
    let before: Option<u32> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok());
    if before != Some(own) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&path, own.to_string());
    }
    match before {
        Some(b) if b > 0 && b < own => Some(own),
        _ => None,
    }
}

/// Where updating stands, for the tray menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    LocalBuild,
    Waiting,
    UpToDate,
    /// Newer build out, not being installed: auto-update is off, or this
    /// copy isn't the installed one.
    Available {
        build: u32,
        can_install: bool,
    },
    Installing {
        build: u32,
    },
    Failed(String),
}

impl UpdateStatus {
    /// The menu line, and whether it can be clicked.
    pub fn menu_item(&self, own: u32) -> (String, bool) {
        match self {
            UpdateStatus::LocalBuild => ("Local build, no updates".into(), false),
            UpdateStatus::Waiting => (format!("Build {own}"), false),
            UpdateStatus::UpToDate => (format!("Build {own}, up to date"), false),
            UpdateStatus::Available {
                build,
                can_install: true,
            } => (format!("Install build {build}"), true),
            UpdateStatus::Available {
                build,
                can_install: false,
            } => (format!("Build {build} is out"), true),
            UpdateStatus::Installing { build } => (format!("Installing build {build}..."), false),
            UpdateStatus::Failed(e) => (format!("Update check failed: {e}"), false),
        }
    }
}

pub enum UpdateMsg {
    /// The menu's "Install build N".
    InstallNow,
    ConfigChanged,
    Quit,
}

/// What the updater needs to know about this copy.
pub struct UpdateEnv {
    pub own: u32,
    pub installed: bool,
    pub temp_dir: PathBuf,
    /// An update waits while the Settings window is open.
    pub settings_open: Arc<AtomicBool>,
    pub first: Duration,
    pub every: Duration,
}

/// The updater thread's loop. `check`, `fetch` and `launch` are the network
/// and process steps (`fetch_release`, `fetch_verified`, `launch_setup` in
/// the app, fakes in tests).
#[allow(clippy::too_many_arguments)]
pub fn run_updater(
    shared: Arc<Mutex<Config>>,
    rx: Receiver<UpdateMsg>,
    status: Arc<Mutex<UpdateStatus>>,
    env: UpdateEnv,
    check: impl Fn() -> Result<Release, String>,
    fetch: impl Fn(&Release, &Path) -> Result<PathBuf, String>,
    launch: impl Fn(&Path) -> Result<(), String>,
    changed: impl Fn(),
) {
    let set = |new: UpdateStatus| {
        *status.lock().unwrap() = new;
        changed();
    };

    if env.own == 0 {
        set(UpdateStatus::LocalBuild);
        // Nothing to do but wait to be told to stop.
        while !matches!(rx.recv(), Ok(UpdateMsg::Quit) | Err(_)) {}
        return;
    }

    set(UpdateStatus::Waiting);
    match rx.recv_timeout(env.first) {
        Ok(UpdateMsg::Quit) | Err(RecvTimeoutError::Disconnected) => return,
        _ => {}
    }

    let install = |release: &Release| -> bool {
        while env.settings_open.load(Ordering::SeqCst) {
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(UpdateMsg::Quit) | Err(RecvTimeoutError::Disconnected) => return false,
                _ => {}
            }
        }
        set(UpdateStatus::Installing {
            build: release.build,
        });
        match fetch(release, &env.temp_dir).and_then(|path| launch(&path)) {
            // The setup stops this copy in a moment.
            Ok(()) => {}
            Err(e) => set(UpdateStatus::Failed(e)),
        }
        true
    };

    let mut available: Option<Release> = None;
    let mut do_check = true;
    loop {
        if do_check {
            clean_old_setups(&env.temp_dir);
            match check() {
                Err(e) => set(UpdateStatus::Failed(e)),
                Ok(r) if !is_newer(env.own, r.build) => {
                    available = None;
                    set(UpdateStatus::UpToDate);
                }
                Ok(r) => {
                    let auto = shared.lock().unwrap().auto_update;
                    if env.installed && auto {
                        if !install(&r) {
                            return;
                        }
                    } else {
                        set(UpdateStatus::Available {
                            build: r.build,
                            can_install: env.installed,
                        });
                    }
                    available = Some(r);
                }
            }
        }
        match rx.recv_timeout(env.every) {
            Ok(UpdateMsg::Quit) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(UpdateMsg::InstallNow) => {
                do_check = false;
                if let (Some(r), true) = (available.clone(), env.installed) {
                    if !install(&r) {
                        return;
                    }
                }
            }
            Ok(UpdateMsg::ConfigChanged) | Err(RecvTimeoutError::Timeout) => do_check = true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desk::fake;
    use std::sync::mpsc::channel;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pingagent-test-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn release(build: u32) -> Release {
        Release {
            build,
            tag: format!("v0.4.0-build.{build}"),
            setup_url: "https://github.com/x".into(),
            sha256: Some("0".repeat(64)),
        }
    }

    #[test]
    fn build_numbers_from_tags() {
        assert_eq!(parse_build("v0.4.0-build.57"), Some(57));
        assert_eq!(parse_build("v1.0.0-build.3"), Some(3));
        assert_eq!(parse_build("v0.4.0"), None);
        assert_eq!(parse_build("v0.4.0-build."), None);
        assert_eq!(parse_build("v0.4.0-build.5x"), None);
        assert_eq!(parse_build("v0.4.0-build.0"), None);
    }

    #[test]
    fn newer_same_older_and_local() {
        assert!(is_newer(55, 57));
        assert!(!is_newer(57, 57));
        assert!(!is_newer(58, 57));
        assert!(!is_newer(0, 57));
    }

    #[test]
    fn reads_the_release_reply() {
        let json = include_str!("../tests/fixtures/release_latest.json");
        let r = parse_release(json).unwrap();
        assert_eq!(r.build, 26);
        assert_eq!(r.tag, "v0.4.0-build.26");
        assert!(r.setup_url.ends_with("/PingAgent-Setup.exe"));
        assert_eq!(
            r.sha256.as_deref(),
            Some("a9a1a3a25d75dde7515f129c659a00e2ead37fea9ef1448de2674be4f97c9591")
        );
    }

    #[test]
    fn release_without_a_digest_or_setup() {
        let no_digest = r#"{"tag_name":"v0.4.0-build.9","assets":[{"name":"PingAgent-Setup.exe","browser_download_url":"https://github.com/a"}]}"#;
        assert_eq!(parse_release(no_digest).unwrap().sha256, None);
        let no_setup = r#"{"tag_name":"v0.4.0-build.9","assets":[{"name":"PingAgent.exe","browser_download_url":"https://github.com/a"}]}"#;
        assert!(parse_release(no_setup).is_err());
        assert!(parse_release(r#"{"tag_name":"v1"}"#).is_err());
        assert!(parse_release("<html>").is_err());
    }

    #[test]
    fn downloads_stay_on_github() {
        assert!(is_allowed_download(
            "https://github.com/BeardyWalrus/ping-agent/releases/download/v0.4.0-build.26/PingAgent-Setup.exe"
        ));
        assert!(is_allowed_download(
            "https://objects.githubusercontent.com/github-production-release-asset/1?x=y"
        ));
        assert!(is_allowed_download(
            "https://release-assets.githubusercontent.com/a"
        ));
        assert!(!is_allowed_download("http://github.com/a"));
        assert!(!is_allowed_download("https://github.com.evil.example/a"));
        assert!(!is_allowed_download("https://evilgithubusercontent.com/a"));
        assert!(!is_allowed_download("https://github.com@evil.example/a"));
        assert!(!is_allowed_download("https://example.com/github.com"));
        assert!(!is_allowed_download("github.com/a"));
    }

    #[test]
    fn installed_copy_is_the_install_folder() {
        let lad = Path::new(r"C:\Users\Phil\AppData\Local");
        assert!(is_installed_copy(
            Path::new(r"C:\Users\Phil\AppData\Local\Programs\PingAgent\PingAgent.exe"),
            lad
        ));
        assert!(is_installed_copy(
            Path::new(r"c:\users\phil\appdata\local\programs\pingagent\PingAgent.exe"),
            lad
        ));
        assert!(!is_installed_copy(
            Path::new(r"C:\Users\Phil\Downloads\PingAgent.exe"),
            lad
        ));
    }

    fn sha(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    #[test]
    fn a_good_file_is_kept() {
        let setup = b"MZ pretend setup".to_vec();
        let (base, _rx) = fake::serve(vec![fake::response("200 OK", &setup)]);
        let dir = temp_dir("good");
        let r = Release {
            setup_url: format!("{base}/setup.exe"),
            sha256: Some(sha(&setup)),
            ..release(31)
        };
        let path = fetch_verified(&r, &dir, &|u| u.starts_with(&base)).unwrap();
        assert_eq!(path, dir.join("PingAgent-Setup-31.exe"));
        assert_eq!(std::fs::read(&path).unwrap(), setup);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_bad_digest_is_deleted() {
        let (base, _rx) = fake::serve(vec![fake::response("200 OK", b"MZ something else")]);
        let dir = temp_dir("bad");
        let r = Release {
            setup_url: format!("{base}/setup.exe"),
            sha256: Some(sha(b"MZ the real one")),
            ..release(31)
        };
        let err = fetch_verified(&r, &dir, &|u| u.starts_with(&base)).unwrap_err();
        assert!(err.contains("digest"), "{err}");
        assert!(!dir.join("PingAgent-Setup-31.exe").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn no_digest_downloads_nothing() {
        let dir = temp_dir("nodigest");
        let r = Release {
            sha256: None,
            setup_url: "http://127.0.0.1:1/never".into(),
            ..release(31)
        };
        assert!(fetch_verified(&r, &dir, &|_| true).is_err());
        assert!(!dir.join("PingAgent-Setup-31.exe").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn redirects_are_followed_only_where_allowed() {
        let setup = b"MZ moved".to_vec();
        let (base, _rx) = fake::serve(vec![
            b"HTTP/1.1 302 Found\r\nLocation: /assets/setup.exe\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            fake::response("200 OK", &setup),
        ]);
        let dir = temp_dir("redirect");
        let r = Release {
            setup_url: format!("{base}/setup.exe"),
            sha256: Some(sha(&setup)),
            ..release(31)
        };
        assert!(fetch_verified(&r, &dir, &|u| u.starts_with(&base)).is_ok());

        let (base2, _rx2) = fake::serve(vec![
            b"HTTP/1.1 302 Found\r\nLocation: https://evil.example/setup.exe\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        ]);
        let r2 = Release {
            setup_url: format!("{base2}/setup.exe"),
            ..r
        };
        let err = fetch_verified(&r2, &dir, &|u| u.starts_with(&base2)).unwrap_err();
        assert!(err.contains("other than GitHub"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn note_build_spots_an_update() {
        let dir = temp_dir("note");
        assert_eq!(note_build(&dir, 30), None); // first start
        assert_eq!(note_build(&dir, 30), None); // same build again
        assert_eq!(note_build(&dir, 32), Some(32)); // updated
        assert_eq!(note_build(&dir, 0), None); // a local build changes nothing
        assert_eq!(note_build(&dir, 32), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn menu_items() {
        assert_eq!(
            UpdateStatus::UpToDate.menu_item(30),
            ("Build 30, up to date".into(), false)
        );
        assert_eq!(
            UpdateStatus::Available {
                build: 31,
                can_install: true
            }
            .menu_item(30),
            ("Install build 31".into(), true)
        );
        assert_eq!(
            UpdateStatus::Available {
                build: 31,
                can_install: false
            }
            .menu_item(30),
            ("Build 31 is out".into(), true)
        );
        assert!(!UpdateStatus::Failed("x".into()).menu_item(30).1);
    }

    struct Run {
        status: Arc<Mutex<UpdateStatus>>,
        tx: std::sync::mpsc::Sender<UpdateMsg>,
        launched: std::sync::mpsc::Receiver<PathBuf>,
        worker: std::thread::JoinHandle<()>,
    }

    fn start(own: u32, installed: bool, auto: bool, latest: u32, settings_open: bool) -> Run {
        let shared = Arc::new(Mutex::new(Config {
            auto_update: auto,
            ..Config::default()
        }));
        let status = Arc::new(Mutex::new(UpdateStatus::Waiting));
        let (tx, rx) = channel();
        let (launch_tx, launched) = channel();
        let s2 = status.clone();
        let env = UpdateEnv {
            own,
            installed,
            temp_dir: temp_dir("loop"),
            settings_open: Arc::new(AtomicBool::new(settings_open)),
            first: Duration::from_millis(10),
            every: Duration::from_secs(60),
        };
        let worker = std::thread::spawn(move || {
            run_updater(
                shared,
                rx,
                s2,
                env,
                move || Ok(release(latest)),
                |r, dir| Ok(dir.join(format!("PingAgent-Setup-{}.exe", r.build))),
                move |p| {
                    launch_tx.send(p.to_path_buf()).unwrap();
                    Ok(())
                },
                || {},
            )
        });
        Run {
            status,
            tx,
            launched,
            worker,
        }
    }

    fn settle() {
        std::thread::sleep(Duration::from_millis(150));
    }

    #[test]
    fn installed_copy_installs_a_newer_build() {
        let run = start(55, true, true, 57, false);
        let path = run.launched.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(path.ends_with("PingAgent-Setup-57.exe"));
        assert_eq!(
            *run.status.lock().unwrap(),
            UpdateStatus::Installing { build: 57 }
        );
        run.tx.send(UpdateMsg::Quit).unwrap();
        run.worker.join().unwrap();
    }

    #[test]
    fn same_build_is_up_to_date() {
        let run = start(57, true, true, 57, false);
        settle();
        assert_eq!(*run.status.lock().unwrap(), UpdateStatus::UpToDate);
        assert!(run.launched.try_recv().is_err());
        run.tx.send(UpdateMsg::Quit).unwrap();
        run.worker.join().unwrap();
    }

    #[test]
    fn auto_off_offers_and_installs_on_the_menu() {
        let run = start(55, true, false, 57, false);
        settle();
        assert_eq!(
            *run.status.lock().unwrap(),
            UpdateStatus::Available {
                build: 57,
                can_install: true
            }
        );
        assert!(run.launched.try_recv().is_err());
        run.tx.send(UpdateMsg::InstallNow).unwrap();
        assert!(run
            .launched
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .ends_with("PingAgent-Setup-57.exe"));
        run.tx.send(UpdateMsg::Quit).unwrap();
        run.worker.join().unwrap();
    }

    #[test]
    fn portable_copy_only_offers() {
        let run = start(55, false, true, 57, false);
        settle();
        assert_eq!(
            *run.status.lock().unwrap(),
            UpdateStatus::Available {
                build: 57,
                can_install: false
            }
        );
        run.tx.send(UpdateMsg::InstallNow).unwrap();
        settle();
        assert!(run.launched.try_recv().is_err());
        run.tx.send(UpdateMsg::Quit).unwrap();
        run.worker.join().unwrap();
    }

    #[test]
    fn local_build_never_checks() {
        let run = start(0, true, true, 57, false);
        settle();
        assert_eq!(*run.status.lock().unwrap(), UpdateStatus::LocalBuild);
        assert!(run.launched.try_recv().is_err());
        run.tx.send(UpdateMsg::Quit).unwrap();
        run.worker.join().unwrap();
    }

    #[test]
    fn waits_while_settings_are_open() {
        let run = start(55, true, true, 57, true);
        settle();
        assert!(run.launched.try_recv().is_err());
        run.tx.send(UpdateMsg::Quit).unwrap();
        run.worker.join().unwrap();
    }
}
