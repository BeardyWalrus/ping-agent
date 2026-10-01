//! Rolling ping statistics for the tooltip and menu.

use std::collections::VecDeque;
use std::time::Duration;

/// Outcome of one ping attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PingOutcome {
    /// Reply received with this round-trip time.
    Reply(Duration),
    /// No reply within the timeout.
    Timeout,
    /// Could not even send (name resolution failed, no network, etc.).
    Error(String),
}

/// Keeps the last `capacity` outcomes.
#[derive(Debug, Clone)]
pub struct Stats {
    capacity: usize,
    history: VecDeque<PingOutcome>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub samples: usize,
    pub last: Option<PingOutcome>,
    pub avg_ms: Option<f64>,
    pub min_ms: Option<u64>,
    pub max_ms: Option<u64>,
    /// Percentage of samples that did not get a reply, 0..=100.
    pub loss_pct: f64,
}

impl Stats {
    pub fn new(capacity: usize) -> Self {
        Stats {
            capacity: capacity.max(1),
            history: VecDeque::with_capacity(capacity),
        }
    }

    pub fn push(&mut self, outcome: PingOutcome) {
        if self.history.len() == self.capacity {
            self.history.pop_front();
        }
        self.history.push_back(outcome);
    }

    pub fn clear(&mut self) {
        self.history.clear();
    }

    pub fn summary(&self) -> Summary {
        let samples = self.history.len();
        let rtts: Vec<u64> = self
            .history
            .iter()
            .filter_map(|o| match o {
                PingOutcome::Reply(d) => Some(d.as_millis() as u64),
                _ => None,
            })
            .collect();
        let lost = samples - rtts.len();
        Summary {
            samples,
            last: self.history.back().cloned(),
            avg_ms: if rtts.is_empty() {
                None
            } else {
                Some(rtts.iter().sum::<u64>() as f64 / rtts.len() as f64)
            },
            min_ms: rtts.iter().copied().min(),
            max_ms: rtts.iter().copied().max(),
            loss_pct: if samples == 0 {
                0.0
            } else {
                lost as f64 * 100.0 / samples as f64
            },
        }
    }
}

/// Text shown on the tray icon for an outcome. At most three characters.
pub fn icon_text(outcome: Option<&PingOutcome>) -> String {
    match outcome {
        None => "...".to_string(),
        Some(PingOutcome::Reply(d)) => {
            let ms = d.as_millis();
            if ms == 0 {
                "<1".to_string()
            } else if ms > 999 {
                "999".to_string()
            } else {
                ms.to_string()
            }
        }
        Some(PingOutcome::Timeout) => "X".to_string(),
        Some(PingOutcome::Error(_)) => "?".to_string(),
    }
}

/// Short human-readable form of an outcome, e.g. "12 ms" or "timeout".
pub fn outcome_label(outcome: Option<&PingOutcome>) -> String {
    match outcome {
        None => "waiting".to_string(),
        Some(PingOutcome::Reply(d)) if d.as_millis() == 0 => "<1 ms".to_string(),
        Some(PingOutcome::Reply(d)) => format!("{} ms", d.as_millis()),
        Some(PingOutcome::Timeout) => "timeout".to_string(),
        Some(PingOutcome::Error(e)) => format!("error: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> PingOutcome {
        PingOutcome::Reply(Duration::from_millis(n))
    }

    #[test]
    fn summary_counts_loss_and_average() {
        let mut s = Stats::new(10);
        s.push(ms(10));
        s.push(ms(30));
        s.push(PingOutcome::Timeout);
        s.push(PingOutcome::Error("dns".into()));
        let sum = s.summary();
        assert_eq!(sum.samples, 4);
        assert_eq!(sum.avg_ms, Some(20.0));
        assert_eq!(sum.min_ms, Some(10));
        assert_eq!(sum.max_ms, Some(30));
        assert_eq!(sum.loss_pct, 50.0);
        assert_eq!(sum.last, Some(PingOutcome::Error("dns".into())));
    }

    #[test]
    fn history_is_bounded() {
        let mut s = Stats::new(3);
        for i in 0..10 {
            s.push(ms(i));
        }
        let sum = s.summary();
        assert_eq!(sum.samples, 3);
        assert_eq!(sum.min_ms, Some(7));
    }

    #[test]
    fn empty_summary() {
        let sum = Stats::new(5).summary();
        assert_eq!(sum.samples, 0);
        assert_eq!(sum.avg_ms, None);
        assert_eq!(sum.loss_pct, 0.0);
        assert_eq!(sum.last, None);
    }

    #[test]
    fn icon_text_is_at_most_three_chars() {
        assert_eq!(icon_text(None), "...");
        assert_eq!(icon_text(Some(&ms(0))), "<1");
        assert_eq!(icon_text(Some(&ms(7))), "7");
        assert_eq!(icon_text(Some(&ms(123))), "123");
        assert_eq!(icon_text(Some(&ms(1500))), "999");
        assert_eq!(icon_text(Some(&PingOutcome::Timeout)), "X");
        assert_eq!(icon_text(Some(&PingOutcome::Error("x".into()))), "?");
    }
}
