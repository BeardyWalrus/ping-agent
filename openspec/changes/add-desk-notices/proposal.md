# Proposal: pop-ups from the desk box, riding on the presence reply

## Why

The desk box (local-docker) now watches the desk's air, and wants to tell
the owner when it matters, for example when CO2 has been over 1200 ppm for 10
minutes. When he's at the PC, the PC is the place to tell him. PingAgent
already talks to the box every 30 seconds (`POST /api/desk/presence`, from
`add-desk-lock-and-self-update`), but only reads the HTTP status of the
reply.

A full Windows toast with buttons (Snooze, I've opened it) was mocked up; the
owner chose the plain tray notification instead, with a click opening the
box's page for it (the Air history page for a stuffy desk). local-docker's ADR
0009 records the same decision from the box's side.

## What changes

- **Read a notice from the presence reply.** The box's 200 reply may carry
  `"notice": {"id", "title", "text", "path"}`. PingAgent parses it (all
  strings, `path` optional), ignores it when it's missing, not an object, or
  `id`, `title` or `text` is missing or empty, and trims and caps the fields
  (id 64 characters, title 63, text 255 UTF-16 units: the shell's limits). A
  reply body that isn't JSON changes nothing: the post still counts as sent.
- **Show each notice once, quietly.** The box repeats a notice, same id, for
  about 2 minutes so a lost reply gets another chance. PingAgent remembers the
  last 16 ids it showed and shows a new one as the tray icon's notification
  (the same balloon as "Updated to build N"), with no sound.
- **A click opens the page.** Clicking the notification opens `desk_url` plus
  the notice's `path` in the default browser, only when the path is a plain
  path on the box (one leading `/`; no `//`, `\`, `:`, whitespace or control
  characters) and the address passes the existing desk address check.
  Otherwise, and for the update notification, a click does nothing.
- **A setting.** `desk_notices`, on by default (old config files load with it
  on), with a "Show pop-ups from the desk box" tick box in the Settings
  window's desk group. Notices only arrive while desk reporting is on.

## Impact

- `src/desk.rs`: `DeskNotice`, `Reply`, `parse_notice`, `notice_link`,
  `SeenIds`; `post` returns a `Reply` and reads up to 16 KB of a 2xx body;
  `run_reporter` hands new notices to the UI thread.
- `src/tray.rs`: `balloon` takes `silent` (adds `NIIF_NOSOUND`);
  `NIN_BALLOONUSERCLICK` decodes to `TrayEvent::BalloonClick`.
- `src/app.rs`: the pending-notice slot, showing it on the desk notice poke,
  remembering the shown notification's link, opening it on a click, the
  Settings tick box.
- `src/config.rs`: `desk_notices`.
- README: the setting, and a "Pop-ups from the box" section.
- `docs/adr/0002-desk-notices-ride-on-the-presence-reply.md` (new).

## Later, not in this change

- Buttons on the notification (Snooze, I've opened it), which need a WinRT
  toast and an app identity.
- Telling the box a notice was seen or clicked.
- Notices while desk reporting is off.
