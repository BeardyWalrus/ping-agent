# Tasks

## 0. Before any code

- [x] 0.1 Agree the shape with the owner: a plain tray notification (not a WinRT toast with buttons), a click opens the page on the box
- [x] 0.2 Agree the reply format with local-docker (its ADR 0009)

## 1. Reading notices

- [x] 1.1 `DeskNotice`, `parse_notice` with trimming and caps, and verify tests for a reply with a notice, without one, a garbage body, bad and empty fields, and over-long fields
- [x] 1.2 `post` returns `Reply { status, notice }` and reads up to 16 KB of a 2xx body, and verify the fake-server test with a notice, a non-JSON 200 and a notice on a 422
- [x] 1.3 `notice_link` and `path_is_safe`, and verify tests for "/air" and for "//evil.com", "http://x", "/a b", "air", "/a\b"
- [x] 1.4 `SeenIds` and `run_reporter` handing over new ids only while `desk_notices` is on, and verify tests that an id is handed over once across repeated replies, a new id is, and nothing is when off

## 2. Showing them

- [x] 2.1 `TrayIcon::balloon` takes `silent` (`NIIF_NOSOUND`); the update balloon keeps its sound
- [x] 2.2 The UI thread takes the pending notice on the desk poke and shows it
- [x] 2.3 `NIN_BALLOONUSERCLICK` opens the shown notification's link; no link, nothing
- [x] 2.4 `desk_notices` setting, default on, with a config test for old files, and the Settings tick box
- [x] 2.5 README: the setting and the "Pop-ups from the box" section
- [x] 2.6 ADR 0002

## 3. Release

- [ ] 3.1 CI green on Windows (clippy with `-D warnings` and the build cover `tray.rs` and `app.rs`, which don't compile on Linux)
- [ ] 3.2 Verify on the PC that a stuffy desk pops up once and a click opens the Air page
- [ ] 3.3 Verify on the PC that the pop-up makes no sound, and unticking "Show pop-ups from the desk box" stops it
