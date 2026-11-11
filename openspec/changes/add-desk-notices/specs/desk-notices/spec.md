# Spec Delta

## Purpose

Defines how PingAgent shows notices the desk box sends back in its reply to
the presence post.

## ADDED Requirements

### Requirement: Notice read from the presence reply
When a presence post gets a 2xx reply whose JSON body has a `notice` object
with non-empty string `id`, `title` and `text`, the app SHALL take it as a
notice, with `path` when that is a non-empty string. Any other body SHALL
leave the post counted as sent, with no notice.

#### Scenario: Stuffy desk
- **WHEN** the box replies 200 with `notice: {"id": "co2-1794405600", "title": "Desk air is stuffy", "text": "...", "path": "/air"}`
- **THEN** the menu shows the post as sent and the notice is taken

#### Scenario: Not JSON
- **WHEN** the box replies 200 with a body that isn't JSON
- **THEN** the menu shows the post as sent and there is no notice

#### Scenario: Refused
- **WHEN** the reply is 401, 503 or another refusal, with or without a notice
- **THEN** there is no notice

### Requirement: Fields capped
The app SHALL trim the fields and cap the id at 64 characters, the title at
63 and the text at 255 UTF-16 units, without splitting a character.

### Requirement: Each notice shown once
With desk reporting and `desk_notices` on, the app SHALL show a notice as a
tray notification, without sound, the first time its id arrives, and SHALL
NOT show it again while the id is among the last 16 shown.

#### Scenario: Repeated notice
- **WHEN** the box repeats the same notice id in replies for 2 minutes
- **THEN** one notification shows

#### Scenario: New notice
- **WHEN** a notice with a different id arrives
- **THEN** it shows too

#### Scenario: Notices off
- **WHEN** "Show pop-ups from the desk box" is unticked
- **THEN** no notice shows, and the lock state is still posted

### Requirement: Click opens the page on the box
A click on a desk notification SHALL open `desk_url` followed by the notice's
`path` in the default browser, only when the path starts with a single `/`
and contains no `//`, `\`, `:`, whitespace or control characters, and
`desk_url` is an `http://` or `https://` address with a host. Otherwise, and
for notifications with no path such as "Updated to build N", a click SHALL
open nothing.

#### Scenario: Air page
- **WHEN** he clicks the "Desk air is stuffy" notification with path `/air` and `desk_url` `http://192.168.86.20:3006`
- **THEN** the browser opens `http://192.168.86.20:3006/air`

#### Scenario: Another host
- **WHEN** a notice's path is `//evil.com` or `http://x`
- **THEN** a click opens nothing

### Requirement: Setting defaults on
The `desk_notices` setting SHALL default to on, including for config files
written before it existed.
