# Design: pop-ups from the desk box

## Context

- PingAgent posts `{"locked": bool, "source": "pc"}` to the box's
  `POST <desk_url>/api/desk/presence` on every lock and unlock and every 30
  seconds, from a background thread (`desk::run_reporter`), which pokes the UI
  thread through an `nwg::Notice` when the status changes. Until now only the
  reply's HTTP status was read.
- The box (local-docker) starts adding an optional `notice` to its 200 reply
  and repeats the same notice, same id, in its replies for about 2 minutes.
- The tray icon is driven through `Shell_NotifyIconW` directly with
  `NOTIFYICON_VERSION_4` (`src/tray.rs`), and already shows one balloon,
  "Updated to build N". On Windows 10 and 11 a balloon shows as a toast and
  lands in the notification centre.
- The owner's PC isn't on Tailscale; it reaches the box at the LAN address in
  `desk_url`.

## Goals and non-goals

Goals: a notice from the box shows on the PC within one heartbeat, once,
without a sound; a click opens the right page on the box; nothing the box (or
anything pretending to be it) sends can make PingAgent open anything but a
page on the configured box.

Non-goals: buttons on the notification, reporting back to the box that a
notice was seen, notices while desk reporting is off.

## Decisions

### Notices ride on the presence reply

The box puts the notice in the reply PingAgent already gets every 30 seconds.
See ADR 0002.

Options weighed:

- **The box calls the PC**: needs PingAgent to listen on a port, a firewall
  rule on the PC, and the box to know the PC's address, which changes.
- **A second polling endpoint** (`GET /api/desk/notices`): one more request
  every 30 seconds for what the existing reply can carry, and a second token
  check and error path.
- **Windows push (WNS) or a phone push service**: a cloud service in the path
  of a message between two machines on the same desk, and app identity
  registration.

### The reply format and its limits

`notice` is an object of strings: `id`, `title`, `text`, optional `path`.
Anything else (missing, `null`, not an object, a missing, non-string or empty
`id`, `title` or `text`) is no notice; a non-string or blank `path` is no
link. Fields are trimmed, control characters become spaces (the text keeps
line breaks), and they're capped: id 64 characters, title 63 and text 255
UTF-16 units, which is what the shell's `szInfoTitle` and `szInfo` hold less
their NUL. A cut title or text ends with "…", and a character outside the
Basic Multilingual Plane is never split. `copy_tip` in `tray.rs` still
truncates too.

Only a 2xx reply is read, and only its first 16 KB; a longer or non-JSON body
has no notice and the post still counts as `Sent`. A notice on a 401, 503 or
other refusal is ignored.

`post` now returns `Reply { status, notice }`, so it stays testable against
the local fake server, and `run_reporter`'s `send` closure returns a `Reply`
so tests can still fake it.

### Once per id, decided on the reporter thread

`run_reporter` keeps a `SeenIds` of the last 16 ids it handed over. A new id
goes into a shared `Arc<Mutex<Option<DeskNotice>>>` and the existing desk
`nwg::Notice` is poked; the UI thread takes it from there and shows it. The
dedupe sits on the reporter side so it's plain Rust tested on Linux; the UI
side only takes and shows.

Options weighed: deduping on the UI thread (untestable off Windows); a
channel of notices (more than one waiting makes no sense: they're 30 s apart
and the UI takes each at once); remembering ids on disk (a restart within the
box's 2-minute repeat would then not show the notice again, but it's a rare
case and a second pop-up is harmless).

With `desk_notices` off the reporter ignores notices entirely and doesn't
record their ids, so turning it on while the box is still repeating one shows
it. The UI thread checks the setting again before showing, in case it was
turned off in between.

### A silent tray balloon, not a WinRT toast

The notice is the existing tray balloon with `NIIF_INFO | NIIF_NOSOUND`. A
stuffy-air nudge shouldn't chime. The "Updated to build N" balloon keeps its
sound, as before: it comes once after an update and is worth noticing.

Options weighed: a WinRT toast with Snooze and I've opened it buttons was
mocked up; it needs an app user model id registered with a Start menu
shortcut, COM activation for the buttons, and a WinRT binding crate, for
buttons the owner didn't want. He chose the plain notification.

### A click opens the page on the box

`NIN_BALLOONUSERCLICK` arrives through the tray's `CALLBACK_MESSAGE` (with
`NOTIFYICON_VERSION_4`, the event is in the low word of `lParam`) and decodes
to `TrayEvent::BalloonClick`. The app remembers the link of the notification
it last put up (`balloon_link`); the update balloon sets none. A click opens
it with the existing `open_url` (`ShellExecuteW` "open").

The link is `desk_url` (trimmed, trailing `/` dropped) plus `path`, made only
when the path starts with a single `/` and has no `//`, `\`, `:`,
whitespace or control characters (and is at most 512 bytes), and the address
passes `desk::check_url` (`http://` or `https://` and a host). So the box can
pick a page on itself but can't name another host, a `file:` path or another
scheme. The configured address is the right base: it's how this PC reaches
the box.

The shell's click message doesn't say which notification was clicked, so a
click on an older notification still in the notification centre opens the
newest one's page. Every notice's page is on the same box, and notices are
rare, so this is accepted.

### Setting

`desk_notices: bool`, default `true`, through the struct-wide
`#[serde(default)]`, so a config file from before this change loads with it
on. A "Show pop-ups from the desk box" tick box under the desk token, greyed
out while desk reporting is off, like the address and token.

### Version

The Cargo version stays at 0.4.0: releases are numbered by CI build
(`v0.4.0-build.N`), and the previous feature didn't bump it either.

## Risks and trade-offs

- [Something on the LAN answers as the box and sends notices.] → It already
  needs to be at `desk_url`; it can show a 255-character pop-up at most and
  open only a page at `desk_url`.
- [A burst of notices.] → One per 30-second post at most, each id once.
- [Focus assist or Do not disturb hides it.] → Windows' choice; the notice is
  in the notification centre afterwards.
- [App restarts while a notice is repeating.] → It shows once more; harmless.
