use std::time::SystemTime;

/// Formats a timestamp as `HH:MM:SS.mmm` UTC.
pub fn clock(t: SystemTime) -> String {
    let d = t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() % 86_400;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        secs / 3600,
        secs / 60 % 60,
        secs % 60,
        d.subsec_millis()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn formats_utc_clock() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_millis(1_759_320_245_042);
        assert_eq!(clock(t), "12:04:05.042");
    }
}
