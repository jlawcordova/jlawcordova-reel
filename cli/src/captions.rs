//! `captions.srt` and `chapters.txt` (spec §7.16).

use crate::time::{clock, srt_time};
use crate::timeline::Timeline;

const ROW: usize = 42;

/// One caption: its time and its rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Caption {
    pub start: f64,
    pub end: f64,
    pub rows: Vec<String>,
}

/// Captions from the script's lines on their locked word times. A line
/// longer than two rows of 42 characters is split at word boundaries.
pub fn captions(t: &Timeline) -> Vec<Caption> {
    let mut out: Vec<Caption> = Vec::new();
    for line in t.lines() {
        let mut rows: Vec<String> = vec![String::new()];
        let mut start: Option<f64> = None;
        let mut end = line.start;
        for w in &line.words {
            let row = rows.last_mut().unwrap();
            let fits = row.is_empty() || row.chars().count() + 1 + w.w.chars().count() <= ROW;
            if !fits {
                if rows.len() == 2 {
                    out.push(Caption {
                        start: start.unwrap_or(line.start),
                        end,
                        rows: std::mem::take(&mut rows),
                    });
                    rows.push(String::new());
                    start = None;
                } else {
                    rows.push(String::new());
                }
            }
            let row = rows.last_mut().unwrap();
            if !row.is_empty() {
                row.push(' ');
            }
            row.push_str(&w.w);
            start.get_or_insert(w.start);
            end = w.end.max(end);
        }
        if rows.iter().any(|r| !r.is_empty()) {
            out.push(Caption {
                start: start.unwrap_or(line.start),
                end: end.max(line.end.min(end + 0.5)),
                rows,
            });
        }
    }
    // At least 0.7 s on screen where there's room; never overlapping.
    for i in 0..out.len() {
        let next = out.get(i + 1).map_or(f64::MAX, |c| c.start);
        let c = &mut out[i];
        if c.end - c.start < 0.7 {
            c.end = (c.start + 0.7).min(next);
        }
        c.end = c.end.min(next);
    }
    out
}

pub fn srt(caps: &[Caption]) -> String {
    let mut s = String::new();
    for (i, c) in caps.iter().enumerate() {
        s.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            srt_time(c.start),
            srt_time(c.end),
            c.rows.join("\n")
        ));
    }
    s
}

/// `m:ss Title` for each chapter beat.
pub fn chapters(t: &Timeline) -> Vec<(f64, String)> {
    t.beats
        .iter()
        .filter_map(|b| b.chapter.as_ref().map(|c| (b.start, c.clone())))
        .collect()
}

pub fn chapters_txt(t: &Timeline) -> String {
    chapters(t)
        .into_iter()
        .map(|(s, c)| format!("{} {c}\n", clock(s)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::align::TWord;
    use crate::timeline::{TBeat, TLine};

    fn line(text: &str, t0: f64) -> TLine {
        let words: Vec<TWord> = text
            .split_whitespace()
            .enumerate()
            .map(|(i, w)| TWord {
                w: w.into(),
                start: t0 + i as f64 * 0.3,
                end: t0 + i as f64 * 0.3 + 0.25,
            })
            .collect();
        TLine {
            id: "a.1".into(),
            voice: "you".into(),
            text: text.into(),
            start: t0,
            end: words.last().unwrap().end + 0.1,
            source: None,
            words,
        }
    }

    fn tl(lines: Vec<TLine>, chapters: &[(f64, &str)]) -> Timeline {
        let mut beats = vec![TBeat {
            id: "a".into(),
            kind: "explain".into(),
            picture: "face".into(),
            start: 0.0,
            end: 100.0,
            chapter: None,
            voice_start: 0.0,
            voice_end: 100.0,
            lines,
        }];
        for (s, c) in chapters {
            beats.push(TBeat {
                id: "c".into(),
                kind: "explain".into(),
                picture: "face".into(),
                start: *s,
                end: *s + 1.0,
                chapter: Some(c.to_string()),
                voice_start: 0.0,
                voice_end: 0.0,
                lines: vec![],
            });
        }
        Timeline {
            pixelreel: String::new(),
            locked: true,
            inputs: String::new(),
            fps: 30,
            duration: 100.0,
            beats,
            cues: Default::default(),
        }
    }

    #[test]
    fn long_lines_split_into_two_row_captions() {
        let text = "Here's the plan: three systems, thirty days, one rule, and absolutely no manual farming of any kind whatsoever, starting today.";
        let caps = captions(&tl(vec![line(text, 1.0), line("Short one.", 10.0)], &[]));
        assert_eq!(caps.len(), 3, "{caps:?}");
        for c in &caps {
            assert!(c.rows.len() <= 2);
            assert!(c.rows.iter().all(|r| r.chars().count() <= 42), "{c:?}");
        }
        assert_eq!(caps[0].start, 1.0);
        assert!(caps[0].end <= caps[1].start);
        let all: Vec<String> = caps[..2].iter().flat_map(|c| c.rows.clone()).collect();
        assert_eq!(all.join(" "), text);
        let s = srt(&caps);
        assert!(s.starts_with("1\n00:00:01,000 --> "));
        assert!(
            s.contains("3\n00:00:10,000 --> 00:00:10,700\nShort one.\n"),
            "{s}"
        );
    }

    #[test]
    fn chapters_are_listed_by_start() {
        let t = tl(vec![], &[(0.0, "Intro"), (40.2, "Day 1"), (3725.0, "Late")]);
        assert_eq!(chapters_txt(&t), "0:00 Intro\n0:40 Day 1\n1:02:05 Late\n");
    }
}
