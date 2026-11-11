# 0002. Desk notices ride on the presence reply and show as a tray notification

- Status: Accepted
- Date: 2026-11-11

## Context

The desk box (local-docker) wants to tell the owner things while he's at the
PC, starting with "Desk air is stuffy" when CO2 has been high for a while.
PingAgent already posts the PC's lock state to the box's
`/api/desk/presence` every 30 seconds and on every lock and unlock, but read
only the HTTP status of the reply. The PC isn't on Tailscale and its LAN
address changes; the box's address is fixed in PingAgent's settings.

## Decision

The box adds an optional `notice` (`id`, `title`, `text`, optional `path`) to
its 200 reply to the presence post and repeats it, same id, for about 2
minutes. PingAgent shows each id once as its tray icon's notification, with
no sound, and a click opens the box's address plus `path` (for a stuffy desk,
the Air history page), only when the path is a plain path on that box. A
setting, on by default, turns it off.

## Consequences

- No new request, port, firewall rule or cloud service: a notice reaches the
  PC within one heartbeat (30 s) of the box raising it, while desk reporting
  is on.
- The box can't reach the PC when reporting is off or the PC is asleep; a
  notice raised then is missed once its 2 minutes are up. That's the point:
  it's for when he's at the desk.
- The notification is the shell's plain one: no buttons, and the click
  message doesn't say which notification was clicked, so the newest one's
  page opens.
- What the box sends is capped and checked, so a stray reply can show at most
  a short pop-up and open only a page at `desk_url`.

## Alternatives considered

- The box calling the PC: needs a listening port, a firewall rule, and the
  PC's address, which changes.
- A second polling endpoint: one more request every 30 s for what the
  existing reply can carry.
- Windows push (WNS) or a phone push service: a cloud service in the path,
  and app identity registration.
- A full WinRT toast with Snooze and I've opened it buttons: mocked up; the
  owner chose the plain tray notification, with a click opening the history
  page.

local-docker's ADR 0009 records the same decision from the box's side. See
`openspec/changes/add-desk-notices/design.md`.
