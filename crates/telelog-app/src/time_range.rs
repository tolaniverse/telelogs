//! The stream's time range: relative presets that move with the clock, or a fixed window.

use std::time::{Duration, SystemTime};

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeRange {
    All,
    /// The last `Duration`, measured from now every time the filter is applied.
    Last(Duration),
    /// A fixed window; either end may be open.
    Between {
        from: Option<SystemTime>,
        to: Option<SystemTime>,
    },
}

pub const PRESETS: [(Duration, &str); 5] = [
    (Duration::from_secs(5 * 60), "Last 5 minutes"),
    (Duration::from_secs(15 * 60), "Last 15 minutes"),
    (Duration::from_secs(60 * 60), "Last 1 hour"),
    (Duration::from_secs(6 * 60 * 60), "Last 6 hours"),
    (Duration::from_secs(24 * 60 * 60), "Last 24 hours"),
];

impl TimeRange {
    pub fn bounds(self, now: SystemTime) -> (Option<SystemTime>, Option<SystemTime>) {
        match self {
            TimeRange::All => (None, None),
            TimeRange::Last(window) => (now.checked_sub(window), None),
            TimeRange::Between { from, to } => (from, to),
        }
    }

    /// Whether the range keeps moving, so records age out of it over time.
    pub fn is_relative(self) -> bool {
        matches!(self, TimeRange::Last(_))
    }

    /// Whether new lines can still fall inside the range.
    pub fn includes_now(self) -> bool {
        match self {
            TimeRange::All | TimeRange::Last(_) => true,
            TimeRange::Between { to, .. } => to.is_none_or(|to| to >= SystemTime::now()),
        }
    }

    pub fn label(self) -> String {
        match self {
            TimeRange::All => "All time".into(),
            TimeRange::Last(window) => PRESETS
                .iter()
                .find(|(d, _)| *d == window)
                .map(|(_, label)| label.to_string())
                .unwrap_or_else(|| format!("Last {}m", window.as_secs() / 60)),
            TimeRange::Between { from, to } => match (from, to) {
                (Some(from), Some(to)) => format!("{} → {}", short(from), short_end(from, to)),
                (Some(from), None) => format!("Since {}", short(from)),
                (None, Some(to)) => format!("Until {}", short(to)),
                (None, None) => "All time".into(),
            },
        }
    }
}

fn short(t: SystemTime) -> String {
    let local: DateTime<Local> = t.into();
    local.format("%b %-d, %H:%M").to_string()
}

/// The end of a range, dropping the date when it matches the start's.
fn short_end(from: SystemTime, to: SystemTime) -> String {
    let (from, to): (DateTime<Local>, DateTime<Local>) = (from.into(), to.into());
    if from.date_naive() == to.date_naive() {
        to.format("%H:%M").to_string()
    } else {
        to.format("%b %-d, %H:%M").to_string()
    }
}

/// Formats a time the way `parse_local` reads it back.
pub fn format_local(t: SystemTime) -> String {
    let local: DateTime<Local> = t.into();
    local.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Parses a local date/time. Empty input means an open end. Accepts `YYYY-MM-DD HH:MM[:SS]`,
/// `YYYY-MM-DD` (midnight), or `HH:MM[:SS]` (today).
pub fn parse_local(input: &str, today: NaiveDate) -> Result<Option<SystemTime>, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    let naive = [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
    ]
    .iter()
    .find_map(|f| NaiveDateTime::parse_from_str(input, f).ok())
    .or_else(|| {
        NaiveDate::parse_from_str(input, "%Y-%m-%d")
            .ok()
            .map(|d| d.and_time(NaiveTime::MIN))
    })
    .or_else(|| {
        ["%H:%M:%S", "%H:%M"]
            .iter()
            .find_map(|f| NaiveTime::parse_from_str(input, f).ok())
            .map(|t| today.and_time(t))
    })
    .ok_or_else(|| format!("Can't read \"{input}\". Use YYYY-MM-DD HH:MM or HH:MM."))?;
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|t| Some(t.into()))
        .ok_or_else(|| format!("\"{input}\" doesn't exist in your time zone."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
    }

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> SystemTime {
        Local.with_ymd_and_hms(y, m, d, h, min, s).earliest().unwrap().into()
    }

    #[test]
    fn parses_supported_formats() {
        assert_eq!(parse_local("", today()), Ok(None));
        assert_eq!(
            parse_local("2026-09-30 08:15", today()),
            Ok(Some(local(2026, 9, 30, 8, 15, 0)))
        );
        assert_eq!(
            parse_local("2026-09-30 08:15:42", today()),
            Ok(Some(local(2026, 9, 30, 8, 15, 42)))
        );
        assert_eq!(
            parse_local("2026-09-30", today()),
            Ok(Some(local(2026, 9, 30, 0, 0, 0)))
        );
        assert_eq!(parse_local(" 21:05 ", today()), Ok(Some(local(2026, 10, 1, 21, 5, 0))));
        assert!(parse_local("yesterday", today()).is_err());
    }

    #[test]
    fn format_round_trips() {
        let t = local(2026, 10, 1, 21, 5, 7);
        assert_eq!(parse_local(&format_local(t), today()), Ok(Some(t)));
    }

    #[test]
    fn relative_ranges_move_with_now() {
        let now = local(2026, 10, 1, 12, 0, 0);
        let (from, to) = TimeRange::Last(Duration::from_secs(900)).bounds(now);
        assert_eq!(from, Some(local(2026, 10, 1, 11, 45, 0)));
        assert_eq!(to, None);
        assert!(TimeRange::Last(Duration::from_secs(900)).is_relative());
    }

    #[test]
    fn past_window_excludes_now() {
        let range = TimeRange::Between {
            from: None,
            to: Some(SystemTime::UNIX_EPOCH),
        };
        assert!(!range.includes_now());
        assert!(TimeRange::All.includes_now());
    }

    #[test]
    fn labels() {
        assert_eq!(TimeRange::All.label(), "All time");
        assert_eq!(TimeRange::Last(Duration::from_secs(3600)).label(), "Last 1 hour");
        let range = TimeRange::Between {
            from: Some(local(2026, 10, 1, 9, 0, 0)),
            to: Some(local(2026, 10, 1, 10, 30, 0)),
        };
        assert_eq!(range.label(), "Oct 1, 09:00 → 10:30");
    }
}
