//! Local ping history: one CSV file per day under the settings folder, pruned
//! to a configurable number of days.
//!
//! Row format: `time,host,rtt_ms,result,detail`
//! - `time`   RFC 3339 local time with offset, e.g. `2026-10-01T18:04:05+01:00`
//! - `rtt_ms` empty when there was no reply
//! - `result` `reply`, `timeout` or `error`

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Local, NaiveDate};

use crate::config::APP_NAME;
use crate::stats::PingOutcome;

pub const CSV_HEADER: &str = "time,host,rtt_ms,result,detail";

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub at: DateTime<Local>,
    pub host: String,
    pub outcome: PingOutcome,
}

pub struct History {
    dir: PathBuf,
}

impl History {
    /// History folder next to the config file. `None` if the platform has no config dir.
    pub fn default_dir() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join(APP_NAME).join("history"))
    }

    pub fn new(dir: PathBuf) -> History {
        History { dir }
    }

    #[cfg(test)]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file_for(&self, date: NaiveDate) -> PathBuf {
        self.dir.join(format!("{}.csv", date.format("%Y-%m-%d")))
    }

    /// Append one sample to today's file, creating it (with a header) if needed.
    pub fn append(&self, row: &Row) -> Result<(), String> {
        fs::create_dir_all(&self.dir)
            .map_err(|e| format!("cannot create {}: {e}", self.dir.display()))?;
        let path = self.file_for(row.at.date_naive());
        let is_new = !path.exists();
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        if is_new {
            writeln!(f, "{CSV_HEADER}").map_err(|e| e.to_string())?;
        }
        writeln!(f, "{}", format_row(row)).map_err(|e| e.to_string())
    }

    /// Delete day files older than `keep_days` days (today counts as day one).
    /// Returns how many files were removed.
    pub fn prune(&self, keep_days: u64, today: NaiveDate) -> usize {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return 0;
        };
        let cutoff = today - Duration::days(keep_days.saturating_sub(1) as i64);
        let mut removed = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(date) = date_from_path(&path) {
                if date < cutoff && fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }
        removed
    }

    /// All rows at or after `since`, oldest first. Unreadable lines are skipped.
    pub fn load_since(&self, since: DateTime<Local>) -> Vec<Row> {
        let mut files: Vec<(NaiveDate, PathBuf)> = match fs::read_dir(&self.dir) {
            Ok(entries) => entries
                .flatten()
                .map(|e| e.path())
                .filter_map(|p| date_from_path(&p).map(|d| (d, p)))
                .filter(|(d, _)| *d >= since.date_naive())
                .collect(),
            Err(_) => return Vec::new(),
        };
        files.sort();
        let mut rows = Vec::new();
        for (_, path) in files {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for line in text.lines().skip(1) {
                if let Some(row) = parse_row(line) {
                    if row.at >= since {
                        rows.push(row);
                    }
                }
            }
        }
        rows
    }
}

fn date_from_path(path: &Path) -> Option<NaiveDate> {
    if path.extension().and_then(|e| e.to_str()) != Some("csv") {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    NaiveDate::parse_from_str(stem, "%Y-%m-%d").ok()
}

pub fn format_row(row: &Row) -> String {
    let (rtt, result, detail) = match &row.outcome {
        PingOutcome::Reply(d) => (d.as_millis().to_string(), "reply", String::new()),
        PingOutcome::Timeout => (String::new(), "timeout", String::new()),
        PingOutcome::Error(e) => (String::new(), "error", csv_quote(e)),
    };
    format!(
        "{},{},{},{},{}",
        row.at.to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
        csv_quote(&row.host),
        rtt,
        result,
        detail
    )
}

pub fn parse_row(line: &str) -> Option<Row> {
    let fields = split_csv(line);
    if fields.len() < 4 {
        return None;
    }
    let at = DateTime::parse_from_rfc3339(&fields[0])
        .ok()?
        .with_timezone(&Local);
    let host = fields[1].clone();
    let outcome = match fields[3].as_str() {
        "reply" => PingOutcome::Reply(std::time::Duration::from_millis(fields[2].parse().ok()?)),
        "timeout" => PingOutcome::Timeout,
        "error" => PingOutcome::Error(fields.get(4).cloned().unwrap_or_default()),
        _ => return None,
    };
    Some(Row { at, host, outcome })
}

/// Quote a field only when it needs it.
fn csv_quote(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Minimal RFC 4180 field splitter (handles quoted fields with doubled quotes).
fn split_csv(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::time::Duration as StdDuration;

    fn temp_history() -> History {
        let dir = std::env::temp_dir()
            .join(format!("ping-agent-history-test-{}", std::process::id()))
            .join(format!("{:?}", std::thread::current().id()).replace(['(', ')'], ""));
        let _ = fs::remove_dir_all(&dir);
        History::new(dir)
    }

    fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn rows_round_trip() {
        let rows = [
            Row {
                at: at(2026, 10, 1, 9),
                host: "192.168.86.1".into(),
                outcome: PingOutcome::Reply(StdDuration::from_millis(12)),
            },
            Row {
                at: at(2026, 10, 1, 9),
                host: "a,b".into(),
                outcome: PingOutcome::Timeout,
            },
            Row {
                at: at(2026, 10, 1, 9),
                host: "h".into(),
                outcome: PingOutcome::Error("said \"no\", twice".into()),
            },
        ];
        for row in &rows {
            let line = format_row(row);
            assert_eq!(parse_row(&line).as_ref(), Some(row), "line: {line}");
        }
        assert!(parse_row("garbage").is_none());
        assert!(parse_row(CSV_HEADER).is_none());
    }

    #[test]
    fn appends_and_loads_in_order() {
        let h = temp_history();
        let r1 = Row {
            at: at(2026, 9, 30, 23),
            host: "h".into(),
            outcome: PingOutcome::Reply(StdDuration::from_millis(5)),
        };
        let r2 = Row {
            at: at(2026, 10, 1, 1),
            host: "h".into(),
            outcome: PingOutcome::Timeout,
        };
        let r3 = Row {
            at: at(2026, 10, 1, 2),
            host: "h".into(),
            outcome: PingOutcome::Reply(StdDuration::from_millis(7)),
        };
        for r in [&r3, &r1, &r2] {
            h.append(r).unwrap();
        }
        assert!(h
            .file_for(NaiveDate::from_ymd_opt(2026, 9, 30).unwrap())
            .exists());
        let all = h.load_since(at(2026, 9, 1, 0));
        assert_eq!(all.len(), 3);
        assert_eq!(all[0], r1);
        let header =
            fs::read_to_string(h.file_for(NaiveDate::from_ymd_opt(2026, 10, 1).unwrap())).unwrap();
        assert!(header.starts_with(CSV_HEADER));
        assert_eq!(header.matches(CSV_HEADER).count(), 1);
        let recent = h.load_since(
            at(2026, 10, 1, 1)
                .checked_add_signed(Duration::minutes(1))
                .unwrap(),
        );
        assert_eq!(recent, vec![r3]);
    }

    #[test]
    fn prunes_old_files_only() {
        let h = temp_history();
        for day in [24, 25, 30] {
            h.append(&Row {
                at: at(2026, 9, day, 12),
                host: "h".into(),
                outcome: PingOutcome::Timeout,
            })
            .unwrap();
        }
        fs::write(h.dir().join("notes.txt"), "keep me").unwrap();
        let removed = h.prune(7, NaiveDate::from_ymd_opt(2026, 10, 1).unwrap());
        assert_eq!(removed, 1); // only the 24th is older than 7 days inclusive of today
        assert!(!h
            .file_for(NaiveDate::from_ymd_opt(2026, 9, 24).unwrap())
            .exists());
        assert!(h
            .file_for(NaiveDate::from_ymd_opt(2026, 9, 25).unwrap())
            .exists());
        assert!(h.dir().join("notes.txt").exists());
    }
}
