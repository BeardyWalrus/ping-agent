//! Builds the self-contained HTML history page from `assets/report.html`.

use chrono::{DateTime, Local};

use crate::config::Config;
use crate::history::Row;
use crate::stats::PingOutcome;

static TEMPLATE: &str = include_str!("../assets/report.html");

/// Fill the template with `rows` (oldest first) and the current settings.
pub fn build_html(
    config: &Config,
    rows: &[Row],
    history_dir: &str,
    generated_at: DateTime<Local>,
) -> String {
    let mut data = String::with_capacity(rows.len() * 16 + 2);
    data.push('[');
    for (i, row) in rows.iter().enumerate() {
        if i > 0 {
            data.push(',');
        }
        let v: i64 = match &row.outcome {
            PingOutcome::Reply(d) => d.as_millis() as i64,
            PingOutcome::Timeout => -1,
            PingOutcome::Error(_) => -2,
        };
        data.push_str(&format!("[{},{}]", row.at.timestamp(), v));
    }
    data.push(']');

    let meta = format!(
        "{{\"host\":{},\"warn_ms\":{},\"bad_ms\":{},\"history_days\":{},\"generated\":{},\"version\":{},\"history_dir\":{}}}",
        json_str(&config.host),
        config.warn_ms,
        config.bad_ms,
        config.history_days,
        json_str(&generated_at.format("%Y-%m-%d %H:%M").to_string()),
        json_str(env!("CARGO_PKG_VERSION")),
        json_str(history_dir),
    );

    // The template has exactly one of each placeholder; `replacen` keeps a stray
    // "__DATA__" inside the data itself from being touched.
    TEMPLATE
        .replacen("__META__", &meta, 1)
        .replacen("__DATA__", &data, 1)
}

/// JSON string literal, escaped so it is also safe inside a <script> block.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use std::time::Duration as StdDuration;

    #[test]
    fn fills_both_placeholders_and_escapes_host() {
        let cfg = Config {
            host: "evil</script><b>".into(),
            ..Config::default()
        };
        let at = Local.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap();
        let rows = vec![
            Row {
                at,
                host: cfg.host.clone(),
                outcome: PingOutcome::Reply(StdDuration::from_millis(12)),
            },
            Row {
                at: at + Duration::seconds(5),
                host: cfg.host.clone(),
                outcome: PingOutcome::Timeout,
            },
            Row {
                at: at + Duration::seconds(10),
                host: cfg.host.clone(),
                outcome: PingOutcome::Error("x".into()),
            },
        ];
        let html = build_html(&cfg, &rows, "C:\\h", at);
        assert!(!html.contains("__META__") && !html.contains("__DATA__"));
        assert!(
            !html.contains("</script><b>"),
            "host must be escaped inside the script block"
        );
        assert!(html.contains("\\u003c/script\\u003e"));
        let ts = at.timestamp();
        assert!(html.contains(&format!(
            "const DATA = [[{ts},12],[{},-1],[{},-2]];",
            ts + 5,
            ts + 10
        )));
        assert!(html.contains("\"warn_ms\":50"));
        assert!(html.contains("\"history_dir\":\"C:\\\\h\""));
    }

    /// Writes a realistic sample page so the chart can be opened in a browser.
    /// Set PING_AGENT_REPORT_DUMP to a folder to choose where it goes.
    #[test]
    fn dump_sample_report() {
        let Ok(dir) = std::env::var("PING_AGENT_REPORT_DUMP") else {
            return;
        };
        let cfg = Config::default();
        let end = Local.with_ymd_and_hms(2026, 10, 1, 18, 30, 0).unwrap();
        let start = end - Duration::days(7);
        let mut rows = Vec::new();
        let mut t = start;
        let mut seed: u64 = 42;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1000) as f64 / 1000.0
        };
        while t < end {
            let hour = t.format("%H").to_string().parse::<u32>().unwrap();
            let busy = (18..23).contains(&hour);
            let base = if busy { 18.0 } else { 6.0 };
            let r = rnd();
            let outcome = if r < 0.004 {
                PingOutcome::Timeout
            } else if r < 0.02 {
                PingOutcome::Reply(StdDuration::from_millis(
                    (base * 8.0 + rnd() * 300.0) as u64,
                ))
            } else {
                PingOutcome::Reply(StdDuration::from_millis((base + rnd() * base) as u64))
            };
            // An outage on day 3 for twenty minutes.
            let day3 = start + Duration::days(3) + Duration::hours(9);
            let outcome = if t >= day3 && t < day3 + Duration::minutes(20) {
                PingOutcome::Timeout
            } else {
                outcome
            };
            rows.push(Row {
                at: t,
                host: cfg.host.clone(),
                outcome,
            });
            t += Duration::seconds(if busy { 5 } else { 60 });
        }
        let html = build_html(
            &cfg,
            &rows,
            "C:\\Users\\phil\\AppData\\Roaming\\PingAgent\\history",
            end,
        );
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(std::path::Path::new(&dir).join("report.html"), html).unwrap();
    }
}
