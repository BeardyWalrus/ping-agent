# 0001. PingAgent updates itself from GitHub releases through its own installer

- Status: Accepted
- Date: 2026-11-10

## Context

Every merge to `main` already publishes a release with `PingAgent-Setup.exe`.
The owner updates the copy on his desktop by hand and asked whether it could
update itself, as his boards and robot do from his home server. PingAgent
runs per user with no admin rights and isn't code-signed.

## Decision

PingAgent checks GitHub's latest release at start and every 6 hours, compares
the release's build number (`v<version>-build.<run>`) with the one CI
stamped into the exe, downloads a newer `PingAgent-Setup.exe`, verifies its
SHA-256 against the digest GitHub lists for the asset, and runs it silently
with `/RELAUNCH`. Only installed copies do this; local builds (no build
number) never do; it can be turned off in Settings.

## Consequences

- No extra hosting: GitHub already holds every build, so the home server
  plays no part and the PC needs no route to it for updates.
- The installer stays the one way files land, so Start menu, uninstall and
  "start when I sign in" stay right.
- Every green merge reaches the desktop within about 6 hours. Rolling back is
  running an older setup by hand, and turning off auto-update stops it.
- The digest guards against a broken download, not a compromised repo;
  signing the exe is the later fix for that.
- The app gains an HTTP client (`ureq`) and `sha2`.

## Alternatives considered

Serving builds from the home server like the boards' OTA; replacing the exe
in place without the installer; the `self_update` crate; winget or the
Microsoft Store. See `openspec/changes/add-desk-lock-and-self-update/design.md`.
