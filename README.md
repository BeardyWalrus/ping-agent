# PingAgent

A tiny Windows 11 system tray app that pings one address on a schedule and
shows the latest round-trip time, in milliseconds, **as its tray icon**.

- Green, amber or red badge with the ping in ms (three digits max; `<1` for
  sub-millisecond replies, `X` for a timeout, `?` when the ping could not be sent).
- Hover for the host, last result, average, min, max and packet loss over the
  last 60 pings.
- Right-click (or left-click) the icon for **Ping now**, **History...**, **Settings...** and **Exit**.
- **History**: every ping is logged to a daily CSV file and kept for a week by
  default. **History...** opens a SmokePing-style chart in your browser for the
  last hour, 6 hours, day or week: grey "smoke" shows the spread of every ping in
  each time step (darker where pings cluster), the median line is coloured by
  that step's packet loss, and steps with no replies at all are tinted. Hover
  for the median, middle half, min to max and loss of any step; an hourly table
  sits underneath.
- Settings window: host, interval, timeout, colour thresholds, an optional
  **schedule** (fast pings during chosen hours and days, slow pings otherwise),
  and a **Start PingAgent when I sign in to Windows** checkbox.
- **Desk box**: optionally tells a [local-docker](https://github.com/BeardyWalrus/local-docker)
  desk box when the PC is locked or unlocked, so its desk lamp knows you're there,
  and shows the box's notices (such as "Desk air is stuffy") as quiet tray pop-ups.
- **Updates itself**: an installed copy installs each new build from GitHub
  releases by itself.
- Small installer, or a single portable `PingAgent.exe`. No runtime to install, no admin rights.

## Download and install

Every merge to `main` publishes a GitHub release, so the newest installer is
always at this address:

**https://github.com/BeardyWalrus/ping-agent/releases/latest/download/PingAgent-Setup.exe**

Run it. It installs per user into `%LOCALAPPDATA%\Programs\PingAgent`, adds a
Start menu entry, offers a "start when I sign in" tick box, and launches the
app. After that it keeps itself up to date (see [Updates](#updates)); running a
newer setup by hand also works: it stops the running copy, replaces it and
relaunches it. Uninstall from Windows Settings > Apps; this also removes the
start-with-Windows entry but keeps your settings file.

The bare exe for running from any folder is next to it:
https://github.com/BeardyWalrus/ping-agent/releases/latest/download/PingAgent.exe

All releases, with notes, are at https://github.com/BeardyWalrus/ping-agent/releases.

Windows SmartScreen may warn about an unsigned program the first time you run
it; choose **More info** then **Run anyway**.

The tray icon registers itself with a fixed identity, so once you drag it from
the hidden-icons overflow onto the taskbar, Windows remembers that across
updates. (Windows ties that identity to the exe's location, which is one reason
to prefer the installer over moving the portable exe around.)

## Settings

Settings live in `%APPDATA%\PingAgent\config.json` and can be edited either in
the Settings window or by hand (restart the app after hand edits):

```json
{
  "host": "192.168.86.1",
  "interval_secs": 5,
  "timeout_ms": 1000,
  "warn_ms": 50,
  "bad_ms": 150,
  "schedule_enabled": false,
  "active_start": "08:00",
  "active_end": "18:00",
  "active_days": ["Mon", "Tue", "Wed", "Thu", "Fri"],
  "idle_interval_secs": 60,
  "history_days": 7,
  "desk_enabled": false,
  "desk_url": "",
  "desk_token": "",
  "desk_notices": true,
  "auto_update": true
}
```

| Key | Meaning | Limits |
| --- | --- | --- |
| `host` | Host name or IPv4 address to ping | not empty |
| `interval_secs` | Seconds between pings | 1 to 3600 |
| `timeout_ms` | How long to wait for a reply | 100 to 10000 |
| `warn_ms` | Replies at or above this are amber | > 0 |
| `bad_ms` | Replies at or above this are red | >= `warn_ms` |
| `schedule_enabled` | Use the schedule below | `true` / `false` |
| `active_start` | Start of the active window, local time, 24-hour `HH:MM` | |
| `active_end` | End of the active window. Earlier than the start means the window runs past midnight; equal to the start means all day | |
| `active_days` | Days the active window applies to (`Mon` to `Sun`) | at least one when the schedule is on |
| `idle_interval_secs` | Seconds between pings outside the active window | 1 to 3600 |
| `history_days` | Days of ping history to keep on disk; 0 turns logging off | 0 to 365 |
| `desk_enabled` | Post the PC's lock state to the desk box | `true` / `false` |
| `desk_url` | The desk box's address, for example `http://192.168.86.20:3006` | `http://` or `https://`; needed when `desk_enabled` is on |
| `desk_token` | The desk token from the box's Settings | needed when `desk_enabled` is on |
| `desk_notices` | Show the box's notices as tray pop-ups (only while `desk_enabled` is on) | `true` / `false`; on if missing |
| `auto_update` | Install new builds by itself | `true` / `false` |

With the schedule on, `interval_secs` is used inside the active window and
`idle_interval_secs` outside it. The day check uses the current calendar day,
so a 22:00 to 06:00 window on Friday is active until midnight, and after that
only if Saturday is also ticked. The tray menu shows which rate is in effect.

"Start with Windows" writes a value named `PingAgent` under
`HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run` pointing at
the exe's current location. If you move the exe, untick and re-tick the box.

## Desk box

With **Tell the desk box when this PC is locked** ticked, PingAgent posts the
session's lock state to the box's `POST <desk_url>/api/desk/presence`, with the
desk token in an `X-Desk-Token` header and the body
`{"locked": true|false, "source": "pc"}`. It posts at once when you lock or
unlock, and again every 30 seconds; the box forgets the state after 2 minutes
without a post, which covers sleep and shutdown. A failed post isn't retried
early: the next one is 30 seconds later. The tray menu shows how the last post
went: **Desk: token wrong** (the box said 401), **Desk: no token set on the
box** (503) or **Desk: box not answering**.

### Pop-ups from the box

The box can put a `notice` in its reply to a post, for example when the desk
air has been stuffy for a while:

```json
{"ok": true, "locked": false,
 "notice": {"id": "co2-1794405600", "title": "Desk air is stuffy",
            "text": "CO2 has been over 1200 ppm for 10 minutes (now 1340). Open a window or door for a few minutes.",
            "path": "/air"}}
```

With **Show pop-ups from the desk box** ticked (the default), PingAgent shows
it as a tray notification, without a sound, once per `id` (the box repeats a
notice for about 2 minutes in case a reply goes missing; PingAgent remembers
the last 16 ids it showed). Clicking the notification opens `desk_url` plus
`path` in the browser, for example `http://192.168.86.20:3006/air`. A `path`
must start with a single `/` and contain no `//`, `\`, `:`, spaces or
control characters, or the click does nothing. Titles over 63 characters and
texts over 255 are cut short, as Windows allows no more. Untick the box to
ignore notices; reporting the lock state carries on.

The box's side, and what the lamp does with it, is in local-docker's
`docs/desk-presence.md`.

## Updates

Each release is tagged `v<version>-build.<N>`, and the exe carries its own N
(the tray menu shows it). An installed copy, one running from
`%LOCALAPPDATA%\Programs\PingAgent`, checks the latest release a minute after
it starts and every 6 hours after that. When N is higher than its own it:

1. downloads `PingAgent-Setup.exe` from GitHub (only from `github.com` and
   `*.githubusercontent.com`, over HTTPS) into the temp folder;
2. checks the file's SHA-256 against the digest GitHub lists for it, and deletes
   it and stops if they differ or there's no digest;
3. runs it with `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /RELAUNCH`, which
   stops this copy, installs, keeps your "start when I sign in" choice and starts
   the new copy, which shows **Updated to build N**.

An update waits while the Settings window is open. Untick **Install new builds
automatically** in Settings to stop that: the menu then offers **Install build N**
when there's one. A portable copy never installs: its menu shows **Build N is
out**, which opens the releases page. A local `cargo build` is build 0 and never
checks. To go back a build, run an older setup from the releases page by hand
and untick automatic updates, or the next check will update it again.

## History files

Each ping is appended to `%APPDATA%\PingAgent\history\YYYY-MM-DD.csv`:

```
time,host,rtt_ms,result,detail
2026-10-01T18:04:05+01:00,192.168.86.1,12,reply,
2026-10-01T18:04:10+01:00,192.168.86.1,,timeout,
```

`rtt_ms` is empty when there was no reply; `result` is `reply`, `timeout` or
`error`. Files older than `history_days` are deleted once a day. A day at
5-second pings is roughly 600 KB. **History...** rebuilds
`history\report.html` from these files each time you open it, so the page is a
snapshot; open it again for fresh data.

## How it works

- Pings are ICMP echo requests sent through the Windows IP Helper API
  (`IcmpSendEcho`), which works without administrator rights. IPv6 targets are
  not supported yet.
- The icon is rendered on the fly at the tray's native size using an embedded
  subset of DejaVu Sans Bold (see `assets/digits-LICENSE.txt`).
- The UI is plain Win32 via [native-windows-gui](https://github.com/gabdube/native-windows-gui);
  the tray icon itself is driven through `Shell_NotifyIcon` directly so it can carry a GUID.
- The installer is built with [Inno Setup](https://jrsoftware.org/isinfo.php) from `installer/PingAgent.iss`.

## Building

Requires a stable Rust toolchain (https://rustup.rs) with the MSVC target on Windows.

```
cargo build --release
```

The exe is written to `target\release\PingAgent.exe`. To build the installer as
well, install Inno Setup 6 and run:

```
ISCC.exe /DMyAppVersion=0.2.0 installer\PingAgent.iss
```

The platform-independent parts (config, statistics, icon rendering) have unit
tests that also run on Linux and macOS: `cargo test`. To look at the icon
rendering without Windows, run the tests with `PING_AGENT_ICON_DUMP=<folder>`
set; a magnified contact sheet is written there as `sheet.png`.

## License

GPL-3.0-or-later. See `LICENSE`.
