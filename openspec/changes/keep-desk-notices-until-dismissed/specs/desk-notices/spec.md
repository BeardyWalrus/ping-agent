# Spec Delta

## MODIFIED Requirements

### Requirement: Each notice shown once
With desk reporting and `desk_notices` on, the app SHALL show a notice, the
first time its id arrives, as a Windows toast that stays on screen until
dismissed, without sound, and SHALL NOT show it again while the id is among
the last 16 shown. A newer desk notice SHALL replace one still on screen.

#### Scenario: Stuffy desk while away from the screen
- **WHEN** a stuffy-desk notice arrives
- **THEN** it stays on screen until Dismiss or Open is pressed

#### Scenario: Toast refused
- **WHEN** Windows refuses the toast
- **THEN** the notice shows as the tray notification instead

### Requirement: Open the notice's page
The toast SHALL offer an Open button, and a click on the toast SHALL open the
page, only when the notice's path makes a safe link on the box; otherwise
only Dismiss SHALL be offered.
