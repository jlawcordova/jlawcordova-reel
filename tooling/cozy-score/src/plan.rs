//! Parsing and validation of the video plan.

use crate::types::{Instrument, VideoType};
use serde_json::Value;

/// One section of the plan as the user wrote it (times in seconds).
#[derive(Clone, Debug, PartialEq)]
pub struct SectionSpec {
    pub kind: VideoType,
    pub start: f64,
    pub end: f64,
    /// This section's own allow-list; replaces the whole-score list.
    pub instruments: Option<Vec<Instrument>>,
}

fn valid_types() -> String {
    VideoType::ALL
        .iter()
        .map(|t| t.id())
        .collect::<Vec<_>>()
        .join(", ")
}

fn valid_instruments() -> String {
    Instrument::ALL
        .iter()
        .map(|t| t.id())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parses "m:ss", "m:ss.s" or "h:mm:ss" into seconds.
pub fn parse_time(s: &str) -> Option<f64> {
    let s = s.trim();
    let parts: Vec<&str> = s.split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
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
    Some(total)
}

/// Parses a timespan: "m:ss-m:ss", "m:ss.s-m:ss.s" or [start, end].
pub fn parse_span(v: &Value) -> Result<(f64, f64), String> {
    match v {
        Value::String(s) => {
            let (a, b) = s
                .split_once('-')
                .ok_or_else(|| format!("timespan \"{s}\" must look like \"m:ss-m:ss\""))?;
            let start = parse_time(a)
                .ok_or_else(|| format!("bad start time \"{a}\" in timespan \"{s}\""))?;
            let end =
                parse_time(b).ok_or_else(|| format!("bad end time \"{b}\" in timespan \"{s}\""))?;
            Ok((start, end))
        }
        Value::Array(a) if a.len() == 2 => {
            let start = a[0]
                .as_f64()
                .ok_or_else(|| format!("timespan {v} must hold two numbers"))?;
            let end = a[1]
                .as_f64()
                .ok_or_else(|| format!("timespan {v} must hold two numbers"))?;
            Ok((start, end))
        }
        _ => Err(format!(
            "timespan {v} must be \"m:ss-m:ss\" or [startSeconds, endSeconds]"
        )),
    }
}

/// Parses a list of instrument ids, collecting every unknown id.
pub fn parse_instruments<'a, I>(ids: I, context: &str, errors: &mut Vec<String>) -> Vec<Instrument>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut out = Vec::new();
    for id in ids {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        match Instrument::from_id(id) {
            Some(i) => {
                if !out.contains(&i) {
                    out.push(i);
                }
            }
            None => errors.push(format!(
                "{context}: unknown instrument \"{id}\" (valid: {})",
                valid_instruments()
            )),
        }
    }
    out
}

/// Parses the `--instruments` comma list.
pub fn parse_instrument_arg(s: &str) -> Result<Vec<Instrument>, Vec<String>> {
    let mut errors = Vec::new();
    let list = parse_instruments(s.split(','), "--instruments", &mut errors);
    if errors.is_empty() {
        Ok(list)
    } else {
        Err(errors)
    }
}

/// Returns the section (if valid) and its span (if that part parsed), so
/// contiguity can still be checked around a section with other errors.
fn parse_section(
    i: usize,
    v: &Value,
    errors: &mut Vec<String>,
) -> (Option<SectionSpec>, Option<(f64, f64)>) {
    let ctx = format!("section {}", i + 1);
    let (kind_v, span_v, inst_v) = match v {
        Value::Array(a) if a.len() == 2 || a.len() == 3 => (Some(&a[0]), Some(&a[1]), a.get(2)),
        Value::Object(o) => {
            for k in o.keys() {
                if !matches!(k.as_str(), "type" | "span" | "instruments") {
                    errors.push(format!(
                        "{ctx}: unknown key \"{k}\" (expected type, span, instruments)"
                    ));
                }
            }
            (o.get("type"), o.get("span"), o.get("instruments"))
        }
        _ => {
            errors.push(format!(
                "{ctx}: must be [type, span], [type, span, [instruments]] or {{\"type\", \"span\", \"instruments\"}}, got {v}"
            ));
            return (None, None);
        }
    };
    let mut ok = true;
    let kind = match kind_v {
        Some(Value::String(s)) => match VideoType::from_id(s) {
            Some(t) => Some(t),
            None => {
                errors.push(format!(
                    "{ctx}: unknown video type \"{s}\" (valid: {})",
                    valid_types()
                ));
                None
            }
        },
        Some(other) => {
            errors.push(format!("{ctx}: type must be a string, got {other}"));
            None
        }
        None => {
            errors.push(format!("{ctx}: missing \"type\""));
            None
        }
    };
    let span = match span_v {
        Some(s) => match parse_span(s) {
            Ok(sp) => Some(sp),
            Err(e) => {
                errors.push(format!("{ctx}: {e}"));
                None
            }
        },
        None => {
            errors.push(format!("{ctx}: missing \"span\""));
            None
        }
    };
    let instruments = match inst_v {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => {
            let mut ids = Vec::new();
            for x in a {
                match x.as_str() {
                    Some(s) => ids.push(s),
                    None => {
                        ok = false;
                        errors.push(format!("{ctx}: instrument ids must be strings, got {x}"));
                    }
                }
            }
            let before = errors.len();
            let list = parse_instruments(ids, &ctx, errors);
            ok &= errors.len() == before;
            Some(list)
        }
        Some(other) => {
            errors.push(format!(
                "{ctx}: instruments must be an array of ids, got {other}"
            ));
            ok = false;
            None
        }
    };
    if let Some((s, e)) = span {
        if !(s.is_finite() && e.is_finite()) || s < 0.0 {
            errors.push(format!("{ctx}: times must be finite and not negative"));
            ok = false;
        } else if e <= s {
            errors.push(format!(
                "{ctx}: end {} is not after start {}",
                fmt_time(e),
                fmt_time(s)
            ));
            ok = false;
        }
    }
    let spec = match (kind, span, ok) {
        (Some(kind), Some((start, end)), true) => Some(SectionSpec {
            kind,
            start,
            end,
            instruments,
        }),
        _ => None,
    };
    (spec, span.filter(|(s, e)| s.is_finite() && e.is_finite()))
}

pub fn fmt_time(t: f64) -> String {
    let m = (t / 60.0).floor();
    let s = t - m * 60.0;
    format!("{}:{:05.2}", m as u64, s)
}

/// Parses plan JSON text. Every problem is reported, not just the first.
pub fn parse_plan(text: &str) -> Result<Vec<SectionSpec>, Vec<String>> {
    let v: Value =
        serde_json::from_str(text).map_err(|e| vec![format!("plan is not valid JSON: {e}")])?;
    let arr = match v {
        Value::Array(a) => a,
        _ => return Err(vec!["plan must be a JSON array of sections".into()]),
    };
    if arr.is_empty() {
        return Err(vec!["plan has no sections".into()]);
    }
    let mut errors = Vec::new();
    type Parsed = (Vec<Option<SectionSpec>>, Vec<Option<(f64, f64)>>);
    let (parsed, spans): Parsed = arr
        .iter()
        .enumerate()
        .map(|(i, s)| parse_section(i, s, &mut errors))
        .unzip();
    // Contiguity checks between neighbours that both parsed.
    const EPS: f64 = 1e-6;
    if let Some(Some((start, _))) = spans.first() {
        if start.abs() > EPS {
            errors.push(format!(
                "section 1: plan must start at 0:00, starts at {}",
                fmt_time(*start)
            ));
        }
    }
    for i in 1..spans.len() {
        if let (Some((_, a_end)), Some((b_start, _))) = (spans[i - 1], spans[i]) {
            if b_start < a_end - EPS {
                errors.push(format!(
                    "section {}: starts at {} but overlaps section {} which ends at {}",
                    i + 1,
                    fmt_time(b_start),
                    i,
                    fmt_time(a_end)
                ));
            } else if b_start > a_end + EPS {
                errors.push(format!(
                    "section {}: starts at {} leaving a gap after section {} which ends at {}",
                    i + 1,
                    fmt_time(b_start),
                    i,
                    fmt_time(a_end)
                ));
            }
        }
    }
    if errors.is_empty() {
        Ok(parsed.into_iter().map(|s| s.expect("parsed")).collect())
    } else {
        Err(errors)
    }
}

/// `--plan` accepts a path or inline JSON.
pub fn load_plan_arg(arg: &str) -> Result<String, String> {
    let t = arg.trim_start();
    if t.starts_with('[') {
        Ok(arg.to_string())
    } else {
        std::fs::read_to_string(arg).map_err(|e| format!("cannot read plan file \"{arg}\": {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time("0:04"), Some(4.0));
        assert_eq!(parse_time("1:00"), Some(60.0));
        assert_eq!(parse_time("0:04.5"), Some(4.5));
        assert_eq!(parse_time("1:02:03"), Some(3723.0));
        assert_eq!(parse_time("0:75"), None);
        assert_eq!(parse_time("a:00"), None);
        assert_eq!(parse_time(""), None);
    }

    #[test]
    fn spans() {
        assert_eq!(parse_span(&serde_json::json!("0:04-0:20")), Ok((4.0, 20.0)));
        assert_eq!(
            parse_span(&serde_json::json!("0:04.5-0:20.25")),
            Ok((4.5, 20.25))
        );
        assert_eq!(parse_span(&serde_json::json!([4, 20.5])), Ok((4.0, 20.5)));
        assert!(parse_span(&serde_json::json!("0:04")).is_err());
        assert!(parse_span(&serde_json::json!([1])).is_err());
    }

    #[test]
    fn three_section_forms() {
        let p = parse_plan(
            r#"[["cold-open-hook","0:00-0:04"],
                ["explain","0:04-0:20",["guitar","pad","soft-bass"]],
                {"type":"reveal","span":[20,30],"instruments":["guitar","pad"]}]"#,
        )
        .unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].kind, VideoType::ColdOpenHook);
        assert_eq!(p[0].instruments, None);
        assert_eq!(
            p[1].instruments,
            Some(vec![
                Instrument::Guitar,
                Instrument::Pad,
                Instrument::SoftBass
            ])
        );
        assert_eq!(p[2].start, 20.0);
        assert_eq!(p[2].end, 30.0);
        assert_eq!(
            p[2].instruments,
            Some(vec![Instrument::Guitar, Instrument::Pad])
        );
    }

    #[test]
    fn empty_instrument_list_is_valid() {
        let p = parse_plan(r#"[["explain","0:00-0:10",[]]]"#).unwrap();
        assert_eq!(p[0].instruments, Some(vec![]));
    }

    #[test]
    fn reports_all_errors_at_once() {
        let errs = parse_plan(
            r#"[["cold-open","0:01-0:04"],
                ["explain","0:03-0:20",["guitar","banjo"]],
                ["reveal","0:21-0:30"],
                {"type":"intro","span":"0:30-0:25"}]"#,
        )
        .unwrap_err();
        let all = errs.join("\n");
        assert!(all.contains("unknown video type \"cold-open\""), "{all}");
        assert!(all.contains("valid: cold-open-hook"), "{all}");
        assert!(all.contains("unknown instrument \"banjo\""), "{all}");
        assert!(all.contains("valid: flute, guitar"), "{all}");
        assert!(all.contains("leaving a gap"), "{all}");
        assert!(all.contains("is not after start"), "{all}");
        assert!(errs.len() >= 4, "{errs:?}");
    }

    #[test]
    fn rejects_overlap_and_late_start() {
        let errs = parse_plan(r#"[["intro","0:01-0:10"],["explain","0:08-0:20"]]"#).unwrap_err();
        let all = errs.join("\n");
        assert!(all.contains("must start at 0:00"), "{all}");
        assert!(all.contains("overlaps"), "{all}");
    }

    #[test]
    fn instrument_arg() {
        assert_eq!(
            parse_instrument_arg("guitar, pad,guitar").unwrap(),
            vec![Instrument::Guitar, Instrument::Pad]
        );
        assert_eq!(parse_instrument_arg("").unwrap(), vec![]);
        let e = parse_instrument_arg("guitar,oboe,tuba").unwrap_err();
        assert_eq!(e.len(), 2);
    }
}
