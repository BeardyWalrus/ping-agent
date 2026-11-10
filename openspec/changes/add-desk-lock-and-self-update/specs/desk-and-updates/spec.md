# Spec Delta

## Purpose

Defines PingAgent's report of the PC's lock state to the desk box, and how it
keeps itself up to date from GitHub releases.

## ADDED Requirements

### Requirement: Lock state reported at once
When desk reporting is on, the app SHALL post `{"locked": true}` on a
session lock and `{"locked": false}` on an unlock to `<desk_url>/api/desk/presence`
with the `X-Desk-Token` header, within 2 seconds of the event.

#### Scenario: Unlock
- **WHEN** he unlocks the PC
- **THEN** the box receives `locked: false` within 2 seconds

### Requirement: Heartbeat
While desk reporting is on, the app SHALL post the current lock state every
30 seconds, and SHALL NOT queue or burst-retry failed posts.

#### Scenario: Box down
- **WHEN** the box doesn't answer
- **THEN** the menu shows the box isn't answering and the next try is the next heartbeat

### Requirement: Desk errors shown in the menu
A 401 SHALL show "Desk: token wrong", a 503 "Desk: no token set on the box",
and no answer "Desk: box not answering", with no popup.

### Requirement: Update check
With auto-update on, an installed copy SHALL check GitHub's latest release at
start (after a minute) and every 6 hours, and SHALL install a build whose
number is greater than its own.

#### Scenario: Newer build
- **WHEN** the latest release is build 57 and this copy is build 55
- **THEN** it downloads the setup, checks its digest, runs it silently, and the new copy shows "Updated to build 57"

#### Scenario: Local build
- **WHEN** the exe has no build number
- **THEN** it never updates itself

### Requirement: Only a verified setup runs
The app SHALL run a downloaded setup only if its SHA-256 matches the digest
GitHub lists for that asset, and only if every download URL's host is
`github.com` or ends in `.githubusercontent.com`.

#### Scenario: Digest mismatch
- **WHEN** the file's digest differs
- **THEN** it is deleted, not run, and the menu says the update check failed

### Requirement: Auto-update off
With auto-update off, the app SHALL still check, and SHALL offer
"Install build N" in the menu instead of installing.

### Requirement: Portable copy
A copy running outside the install folder SHALL NOT install updates, and
SHALL offer "Build N is out", which opens the releases page.

### Requirement: Relaunch after a silent install
The setup SHALL start the app after installing when given `/RELAUNCH`, and
SHALL keep the earlier "start when I sign in" choice.
