# PingAgent

A tiny Windows 11 system tray app that pings one address on a schedule and
shows the latest round-trip time, in milliseconds, **as its tray icon**.

- Green, amber or red badge with the ping in ms (three digits max; `<1` for
  sub-millisecond replies, `X` for a timeout, `?` when the ping could not be sent).
- Hover for the host, last result, average, min, max and packet loss over the
  last 60 pings.
- Right-click (or left-click) the icon for **Ping now**, **History...**, **Settings...** and **Exit**.
- **Home Assistant**: optionally publishes every result over MQTT. The PC shows
  up as a device with latency, average latency, packet loss and reachable
  entities, created automatically through MQTT discovery.
- **History**: every ping is logged to a daily CSV file and kept for a week by
  default. **History...** opens a chart in your browser: latency over the last
  hour, 6 hours, day or week with the amber and red thresholds, a strip marking
  timeouts, hover for exact values, and an hourly table.
- Settings window: host, interval, timeout, colour thresholds, an optional
  **schedule** (fast pings during chosen hours and days, slow pings otherwise),
  and a **Start PingAgent when I sign in to Windows** checkbox.
- Small installer, or a single portable `PingAgent.exe`. No runtime to install, no admin rights.

## Download and install

Every merge to `main` publishes a GitHub release, so the newest installer is
always at this address:

**https://github.com/BeardyWalrus/ping-agent/releases/latest/download/PingAgent-Setup.exe**

Run it. It installs per user into `%LOCALAPPDATA%\Programs\PingAgent`, adds a
Start menu entry, offers a "start when I sign in" tick box, and launches the
app. To update, run the newer setup: it stops the running copy, replaces it and
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
  "idle_interval_secs": 60
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

With the schedule on, `interval_secs` is used inside the active window and
`idle_interval_secs` outside it. The day check uses the current calendar day,
so a 22:00 to 06:00 window on Friday is active until midnight, and after that
only if Saturday is also ticked. The tray menu shows which rate is in effect.

"Start with Windows" writes a value named `PingAgent` under
`HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run` pointing at
the exe's current location. If you move the exe, untick and re-tick the box.

## Home Assistant

PingAgent can send every result to Home Assistant over MQTT. One-time setup in
Home Assistant, if you have not used MQTT there before:

1. **Settings > Add-ons > Add-on store**, install and start **Mosquitto broker**.
2. **Settings > Devices & services > Add integration > MQTT**, accept the
   defaults (it finds the add-on).
3. Create a Home Assistant user for PingAgent (**Settings > People > Users**),
   or add a login under the Mosquitto add-on's configuration. The broker accepts
   either.

Then in PingAgent: right-click the icon, **Home Assistant...**, tick **Send
results to Home Assistant over MQTT**, enter your Home Assistant host or IP,
leave the port at 1883, enter the username and password, and **Save**. The
status line in that window shows whether it connected and how many results have
been sent.

Within a few seconds a device named **"<your PC> ping"** appears under
**Settings > Devices & services > MQTT** with these entities for the target host:

| Entity | Type | Value |
| --- | --- | --- |
| `<host> latency` | sensor, ms | last round-trip time; unknown while the host is not answering |
| `<host> average latency` | sensor, ms (diagnostic) | average over the last 60 pings |
| `<host> packet loss` | sensor, % | share of the last 60 pings with no reply |
| `<host> reachable` | binary sensor, connectivity | on while pings get replies |

Every entity also carries the raw state as attributes (`host`, `result`, `mode`).
The state message is published on `pingagent/<pc>/<host>/state`, availability on
`pingagent/<pc>/availability`, so you can use the topics directly too.

Notes:

- The password is stored in `config.json` encrypted with Windows DPAPI, so only
  your Windows account can read it. Still, give PingAgent its own user.
- The connection is plain MQTT (no TLS), intended for your home network.
- If you change the ping target, a new set of entities appears for the new host;
  the old ones go unavailable and can be deleted from the device page.

Config file keys live under `"mqtt"`: `enabled`, `host`, `port`, `username`,
`password`, `device_name` (blank means the computer name).

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
