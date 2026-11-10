# Design: desk lock state and self-update

## Context

- PingAgent is a Rust tray app (native-windows-gui, plain Win32), no runtime,
  installed per user by an Inno Setup installer into
  `%LOCALAPPDATA%\Programs\PingAgent`, no admin rights. There's also a bare
  portable exe.
- CI (`.github/workflows/build.yml`) builds on Windows and publishes a
  GitHub release on every merge to `main`, tagged `v<Cargo version>-build.<run
  number>`, with `PingAgent-Setup.exe` and `PingAgent.exe`. The Cargo version
  rarely changes (0.4.0 across many builds), so the run number is what moves.
- The installer already stops a running copy (`taskkill`) and relaunches
  after an interactive install; a silent install skips the relaunch
  (`skipifsilent`).
- The app has a message window with raw event handlers (the tray callback
  uses one) and a worker thread that talks back through an `nwg::Notice`.
- The desk box (local-docker) takes `POST /api/desk/presence` with header
  `X-Desk-Token` and a JSON body with `busy`, `locked` or both; see its
  `docs/desk-presence.md`.
- The repo has no HTTP client yet.

## Goals and non-goals

Goals: the lamp hears about an unlock within a second or two; PingAgent
stays current with no clicks; nothing here can leave the PC with a broken or
foreign exe.

Non-goals: microphone busy, code signing, updating from the box rather than
GitHub, any admin-rights install.

## Decisions

### Lock state from session notifications

`WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION)` on the
existing message window, and a raw handler for `WM_WTSSESSION_CHANGE`:
`WTS_SESSION_LOCK` is locked, `WTS_SESSION_UNLOCK` unlocked. The app starts
from unlocked (it starts at sign-in, in an unlocked session). Sleep and
resume are covered by the heartbeat: no post for 2 minutes and the box drops
the state.

Options weighed: polling whether the input desktop is the secure desktop
(`OpenInputDesktop`; works but polls, and misreads UAC prompts); idle time
from `GetLastInputInfo` (says "idle", not "locked", and reading still counts
as away).

### Posting from the worker thread

The post runs off the UI thread with a 3-second timeout: at once on a
change, then every 30 seconds with the current state. Failures are kept as
the last error for the tray menu ("Desk: token wrong", "Desk: box not
answering"); no queue of old states, no fast retries (the next heartbeat is
the retry). Body: `{"locked": bool, "source": "pc"}`. `busy` is left out so
the box keeps whatever else reports it.

### Self-update from GitHub's latest release

At start (after a minute, so sign-in isn't slowed) and every 6 hours:

1. `GET https://api.github.com/repos/BeardyWalrus/ping-agent/releases/latest`
   with a plain `User-Agent: PingAgent`. The repo is public, so no token;
   the unauthenticated limit (60 an hour) is far above 4 a day.
2. Read the build number from `tag_name` (`v0.4.0-build.57` -> 57). Newer
   than the exe's own number means an update. Equal or older: nothing.
3. Find the `PingAgent-Setup.exe` asset. Download it over HTTPS to
   `%TEMP%\PingAgent-Setup-57.exe`, following redirects only to
   `github.com` and `*.githubusercontent.com`.
4. Check its SHA-256 against the asset's `digest` (`sha256:...`) from the
   same API reply. No digest, or a mismatch: delete the file, don't run it,
   show "Update check failed" in the menu.
5. Run it with `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /RELAUNCH`. The
   setup kills this copy, installs, and starts the new one, which shows a
   balloon "Updated to build 57".

The exe's own build number comes from `GITHUB_RUN_NUMBER`, stamped by
`build.rs` (`cargo:rustc-env`). A build without it (a local `cargo build`)
reports build 0 and never updates itself, so a developer's copy isn't
overwritten.

Only an installed copy (running from `%LOCALAPPDATA%\Programs\PingAgent`)
installs updates. A portable exe elsewhere only shows "Build 57 is out" in
the menu, which opens the releases page.

Options weighed:

- **Update from the box**, like the boards' OTA: local-docker would carry
  PingAgent's builds and serve them. More moving parts (a second copy of
  every release, a new endpoint, and the box must be reachable), for no gain:
  the PC has the internet and GitHub already hosts every build.
- **Replace the exe in place** without the installer (rename the running
  exe, write the new one, restart): works for the portable copy too, but
  skips the installer's Start menu and uninstall bookkeeping, and is more
  code to get wrong.
- **A library such as `self_update`**: handles GitHub releases, but pulls in
  a larger tree and assumes archives or a bare binary, not an installer.
- **Winget or the Microsoft Store**: needs publishing and signing first.

### Installer: relaunch after a silent install

A second `[Run]` entry for the app with `Check: IsRelaunch`, where
`IsRelaunch` looks for `/RELAUNCH` on the command line. The interactive entry
stays as it is. Inno's `UsePreviousTasks` (on by default) keeps the earlier
"start when I sign in" choice on a silent reinstall.

### HTTP client

`ureq` (blocking, small, rustls for TLS) for both the box and GitHub, and
`sha2` for the digest. The exe grows by roughly 1 to 2 MB. Both are tested on
Linux in CI. Options weighed: WinHTTP through `windows-sys` (no new crate,
Windows TLS, but untestable off Windows and much more code); `reqwest`
(pulls in tokio).

### Settings

`config.json` gains:

| Key | Default | Meaning |
|---|---|---|
| `desk_enabled` | `false` | Post the lock state |
| `desk_url` | `""` | The box's base address, for example `http://192.168.86.20:3006` |
| `desk_token` | `""` | The desk token from the box's Settings |
| `auto_update` | `true` | Install new builds by itself |

The Settings window gets a "Desk box" group (tick box, address, token shown
as dots) and an "Update automatically" tick box. The token sits in
`config.json` under `%APPDATA%`, like the rest of the settings: readable by
anything running as him, which is the same exposure as the box's own
settings page on his LAN.

## Risks and trade-offs

- [A bad build reaches main and updates itself everywhere.] → Only builds
  that passed CI are released; the release notes link the PR; running the
  previous setup by hand rolls back. One PC, one user.
- [GitHub or the release is compromised.] → The digest comes from the same
  place as the file, so it guards against a broken download, not a
  compromised repo. Signing (later) would close that.
- [SmartScreen.] → A file the app downloads itself carries no "from the
  internet" mark, so a silent run isn't stopped; the first manual install
  still is.
- [Update while he's mid-use.] → The swap takes a few seconds and the tray
  icon comes back; a check that finds an update while the Settings window is
  open waits for it to close.
- [Locked but at the desk.] → The box doesn't turn the lamp off on a lock;
  it takes a last photo first (local-docker ADR 0006).
