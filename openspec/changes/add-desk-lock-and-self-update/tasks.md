# Tasks

## 0. Before any code

- [ ] 0.1 Agree this change with the owner
- [ ] 0.2 Confirm how PingAgent is installed on his desktop now (setup or portable), and its current build

## 1. Desk lock state

- [ ] 1.1 Config fields and validation (`desk_enabled`, `desk_url`, `desk_token`, `auto_update`), and verify config tests for defaults, old files without them, and bad addresses
- [ ] 1.2 `src/desk.rs`: state, request building and reply handling, and verify tests against a local fake server for 200, 401, 503, 422 and no answer
- [ ] 1.3 Session notifications on the message window, the post on change and the 30 s heartbeat, the menu line, and the Settings group
- [ ] 1.4 README section and a pointer to local-docker's `docs/desk-presence.md`

## 2. Self-update

- [ ] 2.1 `build.rs` stamps the build number; `update.rs` parses tags and compares, and verify tests for newer, same, older, malformed and build 0
- [ ] 2.2 Release check, asset and digest lookup, download with the host allow-list, digest check, and verify tests against a fake server for a good file, a bad digest, no digest and a redirect elsewhere
- [ ] 2.3 Installer `/RELAUNCH`; run the setup silently from an installed copy; portable copies only show the menu item
- [ ] 2.4 Tray: "Updated to build N" balloon, "Install build N" when auto-update is off, last check result in the menu
- [ ] 2.5 README: how updates work and how to turn them off

## 3. Release

- [ ] 3.1 Merge, let CI publish, install that build by hand once on the desktop
- [ ] 3.2 Verify on the desktop: locking and unlocking shows on the box's dashboard within seconds, and the lamp follows
- [ ] 3.3 Verify the next merge updates the desktop copy by itself
