# 0003. Desk notices show as a toast that stays until dismissed

- Status: Accepted
- Date: 2026-11-18

## Context

ADR 0002 showed desk notices as the tray icon's own notification. Windows
fades that out after a few seconds and files it away, so "Desk air is stuffy"
was easy to miss. The owner asked for it to stay until he acknowledges it.
The tray balloon has no way to stay.

## Decision

Desk notices show as a WinRT toast with the `reminder` scenario, which stays
on screen until a button is pressed, under the app id
`BeardyWalrus.PingAgent`. It has two buttons, both handled by Windows itself:
**Open** (protocol activation of the notice's page; a click on the toast does
the same) and **Dismiss** (the system's dismiss). It makes no sound. Every
desk toast shares one tag and group, so a newer notice replaces the one on
screen. The app registers its app id for the current user under
`HKCU\Software\Classes\AppUserModelId\` on every start, and the installer's
Start menu shortcut carries it too. If Windows refuses the toast, the notice
falls back to the tray balloon. The update notification stays a balloon.

## Consequences

- The notice is hard to miss, and stays in view until it's dealt with.
- No click handling in the app: Windows opens the page or dismisses it, so
  nothing breaks if PingAgent restarts while a toast is up.
- Adds the `windows` crate (only the XML and notification parts).
- Reminder toasts show even with notifications set to quiet in some Focus
  modes; that's the point for a stuffy desk.

## Alternatives considered

- Our own always-on-top window: works without an app id, but looks foreign
  next to Windows' own notifications.
- Showing the balloon again until acknowledged: noisy, and the balloon still
  can't be acknowledged.
- An `alarm` toast: loops a sound, too much for stuffy air.
