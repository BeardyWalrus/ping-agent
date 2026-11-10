# Proposal: PingAgent tells the desk box when the PC is locked, and updates itself

## Why

On Nov 10 the owner asked for two things in PingAgent, the tray app on his
desktop:

- "Could we also update the ping monitor app on the pc to know if the pc is
  locked or unlocked too?" The desk lamp in local-docker (its PR 97,
  `openspec/changes/add-desk-lamp`, ADR 0006 there) already takes
  `{"locked": true|false}` on `POST /api/desk/presence` and counts an
  unlocked PC as "you're at the desk". Nothing sends it yet.
- "Could we make that do the same OTA updates? Is that even possible?" Today
  he updates by downloading and running the newest setup by hand. The boards
  and the robot update themselves from the box; he'd like PingAgent to keep
  itself current the same way.

## What changes

- **Lock state to the desk box.** PingAgent watches Windows' lock and unlock
  events and posts `{"locked": ..., "source": "pc"}` to the box's
  `/api/desk/presence` at once on every change, and every 30 seconds as a
  heartbeat (the box treats a report older than 2 minutes as neither).
  Settings gain the box's address, the desk token, and a tick box to turn it
  on. Off until set up. A failed post is shown in the tray menu, never a
  popup, and never retried in a burst.
- **Self-update from GitHub releases.** Every merge to `main` already
  publishes `PingAgent-Setup.exe` as the latest release. At start and every 6
  hours PingAgent asks GitHub for the latest release; when its build number
  is newer than its own, it downloads the setup, checks its SHA-256 against
  the digest GitHub lists for it, and runs it silently. The setup already
  stops the running copy, replaces it and (with a small change) starts it
  again. A tray balloon says "Updated to build N". A tick box, on by
  default, turns this off; the menu then offers "Install build N" instead.
- **The build number in the exe.** CI stamps the run number into the exe
  (the release tag is `v<version>-build.<run>`), so the app can compare. A
  local build has no number and never updates itself.
- **Installer:** a `/RELAUNCH` switch so a silent update starts the app
  again, keeping the earlier "start when I sign in" choice.

## Impact

- `src/desk.rs` (new): the lock watcher's state, the post, the heartbeat.
- `src/update.rs` (new): the release check, version compare, download,
  digest check, and launching the setup.
- `src/app.rs`: session-change notifications on the message window, the
  worker thread for both, new menu items and settings fields.
- `src/config.rs`: `desk_enabled`, `desk_url`, `desk_token`, `auto_update`.
- `build.rs`: stamp `GITHUB_RUN_NUMBER` as the build number.
- `installer/PingAgent.iss`: `/RELAUNCH` handling.
- `Cargo.toml`: an HTTP client (`ureq`) and `sha2`.
- README: the new settings, how updates work, and how to turn them off.
- `docs/adr/0001-self-update-from-github-releases.md` (new).

## Later, not in this change

- Busy from the microphone (in a call) on the same endpoint.
- Signing the exe, which would also quiet SmartScreen.
- Storing the desk token with Windows' credential store rather than in
  `config.json`.
