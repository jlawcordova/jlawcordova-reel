//! Parsing and formatting times.

/// Parses a time: `4s`, `23.5s`, `2000ms`, `0:23`, `1:02:03.5`, or a bare
/// number of seconds. Returns seconds.
pub fn parse_time(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(ms) = s.strip_suffix("ms") {
        return parse_number(ms).map(|v| v / 1000.0);
    }
    if let Some(sec) = s.strip_suffix('s') {
        return parse_number(sec);
    }
    if s.contains(':') {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() > 3 {
            return None;
        }
        let mut total = 0.0;
        for (i, p) in parts.iter().enumerate() {
            let last = i == parts.len() - 1;
            if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit() || (last && c == '.')) {
                return None;
            }
            let v: f64 = p.parse().ok()?;
            if i > 0 && v >= 60.0 {
                return None;
            }
            total = total * 60.0 + v;
        }
        return Some(total);
    }
    parse_number(s)
}

/// Parses a signed offset in seconds: `+0.21`, `-0.2`, `0.4`, also with `s`.
pub fn parse_offset(s: &str) -> Option<f64> {
    let s = s.trim();
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => (-1.0, r),
        None => (1.0, s.strip_prefix('+').unwrap_or(s)),
    };
    parse_time(rest).map(|v| sign * v)
}

fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let v: f64 = s.parse().ok()?;
    v.is_finite().then_some(v)
}

/// `m:ss`, or `h:mm:ss` from an hour on, rounding down to the second, as
/// YouTube chapter timestamps are written.
pub fn clock(t: f64) -> String {
    let total = (t.max(0.0) + 1e-6).floor() as u64;
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `hh:mm:ss,mmm` for SubRip.
pub fn srt_time(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        ms % 1000
    )
}

/// Rounds to the nearest frame of `fps`.
pub fn to_frame(t: f64, fps: u32) -> f64 {
    let f = f64::from(fps);
    (t * f).round() / f
}

/// Rounds to the millisecond.
pub fn to_ms(t: f64) -> f64 {
    (t * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_time_form() {
        assert_eq!(parse_time("4s"), Some(4.0));
        assert_eq!(parse_time("23.5s"), Some(23.5));
        assert_eq!(parse_time("2000ms"), Some(2.0));
        assert_eq!(parse_time("0:23"), Some(23.0));
        assert_eq!(parse_time("1:02:03.5"), Some(3723.5));
        assert_eq!(parse_time("12"), Some(12.0));
        assert_eq!(parse_time("0:75"), None);
        assert_eq!(parse_time("abc"), None);
        assert_eq!(parse_time(""), None);
        assert_eq!(parse_time("-1s"), None);
    }

    #[test]
    fn parses_offsets() {
        assert_eq!(parse_offset("+0.21"), Some(0.21));
        assert_eq!(parse_offset("-0.2"), Some(-0.2));
        assert_eq!(parse_offset("0.4s"), Some(0.4));
        assert_eq!(parse_offset("x"), None);
    }

    #[test]
    fn formats_clock_and_srt() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(125.9), "2:05");
        assert_eq!(clock(3725.0), "1:02:05");
        assert_eq!(srt_time(3725.0425), "01:02:05,043");
    }

    #[test]
    fn rounds_to_frames() {
        assert!((to_frame(1.016, 30) - 1.0).abs() < 1e-9);
        assert!((to_frame(1.02, 30) - 31.0 / 30.0).abs() < 1e-9);
    }
}
