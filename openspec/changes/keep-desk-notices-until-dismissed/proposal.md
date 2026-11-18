# Proposal: keep desk notices on screen until dismissed

## Why

The owner got "Desk air is stuffy" on his PC, but the tray notification
faded by itself. He asked for it to stay until he acknowledges it.

## What changes

- **A toast that stays.** Desk notices show as a Windows toast with the
  `reminder` scenario, silent, with **Open** (when the notice has a safe
  page) and **Dismiss** buttons. A click on the toast also opens the page.
  A newer desk notice replaces the one on screen.
- **An app id.** The app registers `BeardyWalrus.PingAgent` for the current
  user on every start, and the installer's shortcut carries it.
- **A fallback.** If Windows refuses the toast, the old tray balloon shows.
- The update notification is unchanged.

## Impact

- `src/toast.rs` (new): the toast XML (escaped, tested on Linux), app id
  registration and showing the toast.
- `src/app.rs`: registers the app id at start; desk notices use the toast.
- `installer/PingAgent.iss`: shortcut app id, and the registry key removed on
  uninstall.
- `Cargo.toml`: `windows` crate, version 0.5.0.
- ADR 0003, README.
