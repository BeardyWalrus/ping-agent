# PingAgent

A tiny Windows 11 system tray app that pings one address on a schedule and
shows the latest round-trip time, in milliseconds, **as its tray icon**.

- Green, amber or red badge with the ping in ms (three digits max; `<1` for
  sub-millisecond replies, `X` for a timeout, `?` when the ping could not be sent).
- Hover for the host, last result, average, min, max and packet loss over the
  last 60 pings.
- Right-click (or left-click) the icon for **Ping now**, **Settings...** and **Exit**.
- Settings window: host, interval, timeout, colour thresholds, and a
  **Start PingAgent when I sign in to Windows** checkbox.
- Single portable `PingAgent.exe`. No installer, no runtime to install, no admin rights.

## Download

Every push builds a Windows binary. Open the latest run under the repository's
**Actions** tab, then download the `PingAgent-windows-x64` artifact and unzip it.
Tagged versions (`v*`) are also attached to a GitHub release.

Windows SmartScreen may warn about an unsigned exe the first time you run it;
choose **More info** then **Run anyway**.

## Settings

Settings live in `%APPDATA%\PingAgent\config.json` and can be edited either in
the Settings window or by hand (restart the app after hand edits):

```json
{
  "host": "192.168.86.1",
  "interval_secs": 5,
  "timeout_ms": 1000,
  "warn_ms": 50,
  "bad_ms": 150
}
```

| Key | Meaning | Limits |
| --- | --- | --- |
| `host` | Host name or IPv4 address to ping | not empty |
| `interval_secs` | Seconds between pings | 1 to 3600 |
| `timeout_ms` | How long to wait for a reply | 100 to 10000 |
| `warn_ms` | Replies at or above this are amber | > 0 |
| `bad_ms` | Replies at or above this are red | >= `warn_ms` |

"Start with Windows" writes a value named `PingAgent` under
`HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run` pointing at
the exe's current location. If you move the exe, untick and re-tick the box.

## How it works

- Pings are ICMP echo requests sent through the Windows IP Helper API
  (`IcmpSendEcho`), which works without administrator rights. IPv6 targets are
  not supported yet.
- The icon is rendered on the fly at the tray's native size using an embedded
  subset of DejaVu Sans Bold (see `assets/digits-LICENSE.txt`).
- The UI is plain Win32 via [native-windows-gui](https://github.com/gabdube/native-windows-gui).

## Building

Requires a stable Rust toolchain (https://rustup.rs) with the MSVC target on Windows.

```
cargo build --release
```

The exe is written to `target\release\PingAgent.exe`.

The platform-independent parts (config, statistics, icon rendering) have unit
tests that also run on Linux and macOS: `cargo test`. To look at the icon
rendering without Windows, run the tests with `PING_AGENT_ICON_DUMP=<folder>`
set; a magnified contact sheet is written there as `sheet.png`.

## License

GPL-3.0-or-later. See `LICENSE`.
