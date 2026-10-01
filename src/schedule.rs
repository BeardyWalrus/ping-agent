//! Optional active-hours schedule: ping quickly inside a daily window on chosen
//! days, slowly outside it.

use chrono::{Datelike, Local, Timelike};
use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Day {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Day {
    pub const ALL: [Day; 7] = [
        Day::Mon,
        Day::Tue,
        Day::Wed,
        Day::Thu,
        Day::Fri,
        Day::Sat,
        Day::Sun,
    ];
    pub const WEEKDAYS: [Day; 5] = [Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri];

    pub fn short_name(self) -> &'static str {
        match self {
            Day::Mon => "Mon",
            Day::Tue => "Tue",
            Day::Wed => "Wed",
            Day::Thu => "Thu",
            Day::Fri => "Fri",
            Day::Sat => "Sat",
            Day::Sun => "Sun",
        }
    }

    fn from_chrono(w: chrono::Weekday) -> Day {
        match w {
            chrono::Weekday::Mon => Day::Mon,
            chrono::Weekday::Tue => Day::Tue,
            chrono::Weekday::Wed => Day::Wed,
            chrono::Weekday::Thu => Day::Thu,
            chrono::Weekday::Fri => Day::Fri,
            chrono::Weekday::Sat => Day::Sat,
            chrono::Weekday::Sun => Day::Sun,
        }
    }
}

/// A moment in local wall-clock time, reduced to what the schedule cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub day: Day,
    /// Seconds since local midnight, 0..86400.
    pub secs: u32,
}

impl LocalTime {
    pub fn now() -> LocalTime {
        let now = Local::now();
        LocalTime {
            day: Day::from_chrono(now.weekday()),
            secs: now.num_seconds_from_midnight(),
        }
    }

    pub fn minutes(&self) -> u32 {
        self.secs / 60
    }
}

const MINUTES_PER_DAY: u32 = 24 * 60;

/// Parse "HH:MM" (24-hour) into minutes since midnight.
pub fn parse_hhmm(s: &str) -> Result<u32, String> {
    let s = s.trim();
    let err = || format!("'{s}' is not a time in 24-hour HH:MM form.");
    let (h, m) = s.split_once(':').ok_or_else(err)?;
    let h: u32 = h.trim().parse().map_err(|_| err())?;
    let m: u32 = m.trim().parse().map_err(|_| err())?;
    if h > 23 || m > 59 {
        return Err(err());
    }
    Ok(h * 60 + m)
}

pub fn format_hhmm(minutes: u32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Is `minute` inside the [start, end) window? Windows that end before they start
/// wrap past midnight. A zero-length window counts as the whole day.
fn in_window(start: u32, end: u32, minute: u32) -> bool {
    if start == end {
        true
    } else if start < end {
        (start..end).contains(&minute)
    } else {
        minute >= start || minute < end
    }
}

/// What the schedule says right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// No schedule configured: always use the main interval.
    Always,
    /// Inside the active window on an active day.
    Active,
    /// Outside the window, or on an inactive day.
    Idle,
}

pub fn mode(cfg: &Config, now: LocalTime) -> Mode {
    if !cfg.schedule_enabled {
        return Mode::Always;
    }
    let (Ok(start), Ok(end)) = (parse_hhmm(&cfg.active_start), parse_hhmm(&cfg.active_end)) else {
        return Mode::Always;
    };
    if cfg.active_days.contains(&now.day) && in_window(start, end, now.minutes()) {
        Mode::Active
    } else {
        Mode::Idle
    }
}

/// Seconds between pings right now.
pub fn interval_secs(cfg: &Config, now: LocalTime) -> u64 {
    match mode(cfg, now) {
        Mode::Always | Mode::Active => cfg.interval_secs,
        Mode::Idle => cfg.idle_interval_secs,
    }
}

/// Seconds until the schedule could next change state (window start, window end,
/// or midnight when the day changes). `None` when no schedule is in use.
pub fn secs_until_change(cfg: &Config, now: LocalTime) -> Option<u64> {
    if !cfg.schedule_enabled {
        return None;
    }
    let mut boundaries = vec![MINUTES_PER_DAY];
    if let Ok(m) = parse_hhmm(&cfg.active_start) {
        boundaries.push(m);
    }
    if let Ok(m) = parse_hhmm(&cfg.active_end) {
        boundaries.push(m);
    }
    boundaries
        .into_iter()
        .map(|m| m * 60)
        .filter(|&b| b > now.secs)
        .min()
        .map(|b| (b - now.secs) as u64)
}

/// One-line description for the menu / tooltip, e.g. "every 5 s (active hours 08:00-18:00)".
pub fn describe(cfg: &Config, now: LocalTime) -> String {
    match mode(cfg, now) {
        Mode::Always => format!("every {}", secs_label(cfg.interval_secs)),
        Mode::Active => format!(
            "every {} (active hours {}-{})",
            secs_label(cfg.interval_secs),
            cfg.active_start,
            cfg.active_end
        ),
        Mode::Idle => format!(
            "every {} (outside active hours {}-{})",
            secs_label(cfg.idle_interval_secs),
            cfg.active_start,
            cfg.active_end
        ),
    }
}

fn secs_label(secs: u64) -> String {
    if secs >= 60 && secs.is_multiple_of(60) {
        let m = secs / 60;
        format!("{m} min")
    } else {
        format!("{secs} s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: Day, hhmm: &str) -> LocalTime {
        LocalTime {
            day,
            secs: parse_hhmm(hhmm).unwrap() * 60,
        }
    }

    fn scheduled() -> Config {
        Config {
            schedule_enabled: true,
            active_start: "08:00".into(),
            active_end: "18:00".into(),
            active_days: Day::WEEKDAYS.to_vec(),
            interval_secs: 5,
            idle_interval_secs: 300,
            ..Config::default()
        }
    }

    #[test]
    fn parses_times() {
        assert_eq!(parse_hhmm("08:30"), Ok(510));
        assert_eq!(parse_hhmm(" 0:05 "), Ok(5));
        assert_eq!(parse_hhmm("23:59"), Ok(1439));
        assert!(parse_hhmm("24:00").is_err());
        assert!(parse_hhmm("8").is_err());
        assert!(parse_hhmm("08:60").is_err());
        assert_eq!(format_hhmm(510), "08:30");
    }

    #[test]
    fn no_schedule_means_always() {
        let cfg = Config::default();
        assert_eq!(mode(&cfg, at(Day::Sun, "03:00")), Mode::Always);
        assert_eq!(interval_secs(&cfg, at(Day::Sun, "03:00")), 5);
        assert_eq!(secs_until_change(&cfg, at(Day::Sun, "03:00")), None);
    }

    #[test]
    fn active_inside_window_on_active_day() {
        let cfg = scheduled();
        assert_eq!(mode(&cfg, at(Day::Mon, "08:00")), Mode::Active);
        assert_eq!(mode(&cfg, at(Day::Mon, "17:59")), Mode::Active);
        assert_eq!(mode(&cfg, at(Day::Mon, "18:00")), Mode::Idle);
        assert_eq!(mode(&cfg, at(Day::Mon, "07:59")), Mode::Idle);
        assert_eq!(mode(&cfg, at(Day::Sat, "12:00")), Mode::Idle);
        assert_eq!(interval_secs(&cfg, at(Day::Mon, "12:00")), 5);
        assert_eq!(interval_secs(&cfg, at(Day::Sat, "12:00")), 300);
    }

    #[test]
    fn overnight_window_wraps() {
        let cfg = Config {
            active_start: "22:00".into(),
            active_end: "06:00".into(),
            active_days: Day::ALL.to_vec(),
            ..scheduled()
        };
        assert_eq!(mode(&cfg, at(Day::Mon, "23:00")), Mode::Active);
        assert_eq!(mode(&cfg, at(Day::Tue, "01:00")), Mode::Active);
        assert_eq!(mode(&cfg, at(Day::Tue, "06:00")), Mode::Idle);
        assert_eq!(mode(&cfg, at(Day::Tue, "12:00")), Mode::Idle);
    }

    #[test]
    fn equal_start_and_end_is_all_day() {
        let cfg = Config {
            active_start: "09:00".into(),
            active_end: "09:00".into(),
            ..scheduled()
        };
        assert_eq!(mode(&cfg, at(Day::Mon, "03:00")), Mode::Active);
        assert_eq!(mode(&cfg, at(Day::Sun, "03:00")), Mode::Idle);
    }

    #[test]
    fn time_until_next_boundary() {
        let cfg = scheduled();
        assert_eq!(secs_until_change(&cfg, at(Day::Mon, "07:59")), Some(60));
        assert_eq!(
            secs_until_change(&cfg, at(Day::Mon, "08:00")),
            Some(10 * 3600)
        );
        assert_eq!(secs_until_change(&cfg, at(Day::Mon, "23:00")), Some(3600));
        let late = LocalTime {
            day: Day::Mon,
            secs: 86_399,
        };
        assert_eq!(secs_until_change(&cfg, late), Some(1));
    }

    #[test]
    fn describes_mode() {
        let cfg = scheduled();
        assert_eq!(
            describe(&cfg, at(Day::Mon, "12:00")),
            "every 5 s (active hours 08:00-18:00)"
        );
        assert_eq!(
            describe(&cfg, at(Day::Sun, "12:00")),
            "every 5 min (outside active hours 08:00-18:00)"
        );
        assert_eq!(
            describe(&Config::default(), at(Day::Sun, "12:00")),
            "every 5 s"
        );
    }

    #[test]
    fn days_round_trip_through_json() {
        let text = serde_json::to_string(&Day::WEEKDAYS.to_vec()).unwrap();
        assert_eq!(text, r#"["Mon","Tue","Wed","Thu","Fri"]"#);
        let back: Vec<Day> = serde_json::from_str(&text).unwrap();
        assert_eq!(back, Day::WEEKDAYS.to_vec());
    }
}
