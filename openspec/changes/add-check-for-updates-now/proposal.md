# Proposal: check for updates from the tray menu

## Why

A new build was released minutes after PingAgent's last check, and the menu
said "Build 35, up to date", greyed out, with no way to look again short of
waiting up to 6 hours or restarting it.

## What changes

- The build line is clickable while waiting, up to date or after a failed
  check: **Build N: check for updates**, **Build N, up to date: check
  again**, **Update check failed: ... Try again**. A click asks GitHub at
  once (also skipping the first minute's wait after start).
- While asking, the line reads **Build N: checking for updates...**.
- What happens with a newer build is unchanged: an installed copy with
  automatic updates on installs it; otherwise the menu offers it.

## Impact

- `src/update.rs`: `UpdateStatus::Checking`, `UpdateMsg::CheckNow`, menu
  lines, and tests that a click finds a build released since the last check
  and skips the first wait.
- `src/app.rs`: the click sends `CheckNow`.
- README's Updates section.
