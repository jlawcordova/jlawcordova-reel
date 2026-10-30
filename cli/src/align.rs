//! Matching transcript words to script lines (spec §7.7).
//!
//! Words are normalised (case, punctuation, digits spelled out), then each
//! line is found in a take's transcript with a word-level local alignment
//! (Smith-Waterman), as many times as it was read. Reads of every line
//! compete for transcript words: the best-scoring reads win and may not
//! overlap.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TWord {
    pub w: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    /// The sha256 of the recording it was made from.
    pub sha256: String,
    pub words: Vec<TWord>,
}

/// One read of one line in one take.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Read {
    pub take: String,
    pub line: String,
    pub score: i32,
    pub matched: usize,
    pub total: usize,
    pub complete: bool,
    /// Transcript word range, inclusive.
    pub first_word: usize,
    pub last_word: usize,
    /// Start of the first matched word and end of the last, in take time.
    pub start: f64,
    pub end: f64,
    /// Each script word's time in the take; unmatched words are interpolated.
    pub words: Vec<(f64, f64)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TakeAlignment {
    pub take: String,
    pub reads: Vec<Read>,
    /// Transcript stretches longer than 2 s that no read covers: (start, end).
    pub unmatched: Vec<(f64, f64)>,
}

/// A line to look for: its id and its script words.
pub struct Query<'a> {
    pub id: &'a str,
    pub words: &'a [String],
}

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];

/// Spells out a whole number, one token per word.
pub fn number_words(n: u64) -> Vec<String> {
    let mut out = Vec::new();
    spell(n, &mut out);
    out
}

fn spell(n: u64, out: &mut Vec<String>) {
    match n {
        0..=19 => out.push(ONES[n as usize].into()),
        20..=99 => {
            out.push(TENS[(n / 10) as usize].into());
            if !n.is_multiple_of(10) {
                out.push(ONES[(n % 10) as usize].into());
            }
        }
        100..=999 => {
            out.push(ONES[(n / 100) as usize].into());
            out.push("hundred".into());
            if !n.is_multiple_of(100) {
                spell(n % 100, out);
            }
        }
        1000..=999_999 => {
            spell(n / 1000, out);
            out.push("thousand".into());
            if !n.is_multiple_of(1000) {
                spell(n % 1000, out);
            }
        }
        _ => {
            spell(n / 1_000_000, out);
            out.push("million".into());
            if !n.is_multiple_of(1_000_000) {
                spell(n % 1_000_000, out);
            }
        }
    }
}

/// Normalises one written word into the tokens it is spoken as.
pub fn normalize(word: &str) -> Vec<String> {
    let lower = word.to_lowercase().replace(['\'', '’', '‘'], "");
    // "1,500" is one number.
    let chars: Vec<char> = lower.chars().collect();
    let mut cleaned = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let digit_comma = c == ','
            && i > 0
            && i + 1 < chars.len()
            && chars[i - 1].is_ascii_digit()
            && chars[i + 1].is_ascii_digit();
        if digit_comma {
            continue;
        }
        cleaned.push(if c.is_alphanumeric() { c } else { ' ' });
    }
    let mut out = Vec::new();
    for tok in cleaned.split_whitespace() {
        if tok.chars().all(|c| c.is_ascii_digit()) && tok.len() <= 12 {
            out.extend(number_words(tok.parse().unwrap_or(0)));
        } else {
            out.push(tok.to_string());
        }
    }
    out
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

fn similarity(a: &str, b: &str) -> i32 {
    if a == b {
        2
    } else if a.chars().count() >= 4 && b.chars().count() >= 4 && edit_distance(a, b) <= 1 {
        1
    } else {
        -1
    }
}

const GAP: i32 = -1;
const MASKED: i32 = -1000;

struct Hit {
    score: i32,
    /// (query token, transcript token) pairs that matched.
    pairs: Vec<(usize, usize)>,
}

/// Finds up to `max_hits` non-overlapping local alignments of `q` in `t`.
fn find_hits(q: &[String], t: &[String], max_hits: usize) -> Vec<Hit> {
    let m = q.len();
    let n = t.len();
    let threshold = (m as i32).max(4);
    let mut masked = vec![false; n];
    let mut hits = Vec::new();
    if m == 0 || n == 0 {
        return hits;
    }
    let w = n + 1;
    let mut h = vec![0i32; (m + 1) * w];
    // 0 stop, 1 diagonal, 2 up (skip query token), 3 left (skip transcript token)
    let mut dir = vec![0u8; (m + 1) * w];
    for _ in 0..max_hits {
        h.iter_mut().for_each(|v| *v = 0);
        dir.iter_mut().for_each(|v| *v = 0);
        let mut best = (0i32, 0usize, 0usize);
        for i in 1..=m {
            for j in 1..=n {
                let s = if masked[j - 1] {
                    MASKED
                } else {
                    similarity(&q[i - 1], &t[j - 1])
                };
                let diag = h[(i - 1) * w + j - 1] + s;
                let up = h[(i - 1) * w + j] + GAP;
                let left = if masked[j - 1] {
                    MASKED
                } else {
                    h[i * w + j - 1] + GAP
                };
                let (v, d) = if diag >= up && diag >= left && diag > 0 {
                    (diag, 1)
                } else if up >= left && up > 0 {
                    (up, 2)
                } else if left > 0 {
                    (left, 3)
                } else {
                    (0, 0)
                };
                h[i * w + j] = v;
                dir[i * w + j] = d;
                if v > best.0 {
                    best = (v, i, j);
                }
            }
        }
        if best.0 < threshold {
            break;
        }
        let (mut i, mut j) = (best.1, best.2);
        let mut pairs = Vec::new();
        while i > 0 && j > 0 && h[i * w + j] > 0 {
            match dir[i * w + j] {
                1 => {
                    if similarity(&q[i - 1], &t[j - 1]) > 0 {
                        pairs.push((i - 1, j - 1));
                    }
                    i -= 1;
                    j -= 1;
                }
                2 => i -= 1,
                3 => j -= 1,
                _ => break,
            }
        }
        pairs.reverse();
        if pairs.is_empty() {
            break;
        }
        let (lo, hi) = (pairs[0].1, pairs[pairs.len() - 1].1);
        for m in masked.iter_mut().take(hi + 1).skip(lo) {
            *m = true;
        }
        hits.push(Hit {
            score: best.0,
            pairs,
        });
    }
    hits
}

/// Aligns every line against one take's transcript.
pub fn align_take(take: &str, lines: &[Query], tr: &Transcript) -> TakeAlignment {
    // Transcript tokens and the word each came from.
    let mut t_tok = Vec::new();
    let mut t_word = Vec::new();
    for (wi, w) in tr.words.iter().enumerate() {
        for tok in normalize(&w.w) {
            t_tok.push(tok);
            t_word.push(wi);
        }
    }
    struct Cand {
        line: usize,
        hit: Hit,
        lo: usize,
        hi: usize,
    }
    let mut cands = Vec::new();
    let mut q_words: Vec<Vec<usize>> = Vec::new();
    for (li, l) in lines.iter().enumerate() {
        let mut q_tok = Vec::new();
        let mut qw = Vec::new();
        for (si, w) in l.words.iter().enumerate() {
            for tok in normalize(w) {
                q_tok.push(tok);
                qw.push(si);
            }
        }
        for hit in find_hits(&q_tok, &t_tok, 8) {
            let lo = hit.pairs[0].1;
            let hi = hit.pairs[hit.pairs.len() - 1].1;
            cands.push(Cand {
                line: li,
                hit,
                lo,
                hi,
            });
        }
        q_words.push(qw);
    }
    // Best reads first; a read may not share transcript tokens with one
    // already taken.
    cands.sort_by(|a, b| {
        b.hit
            .score
            .cmp(&a.hit.score)
            .then(a.line.cmp(&b.line))
            .then(a.lo.cmp(&b.lo))
    });
    let mut taken: Vec<(usize, usize)> = Vec::new();
    let mut reads = Vec::new();
    for c in cands {
        if taken.iter().any(|&(lo, hi)| c.lo <= hi && lo <= c.hi) {
            continue;
        }
        taken.push((c.lo, c.hi));
        let l = &lines[c.line];
        let total = l.words.len();
        let mut times: Vec<Option<(f64, f64)>> = vec![None; total];
        for &(qi, tj) in &c.hit.pairs {
            let sw = q_words[c.line][qi];
            let tw = &tr.words[t_word[tj]];
            times[sw] = Some(match times[sw] {
                Some((s, e)) => (s.min(tw.start), e.max(tw.end)),
                None => (tw.start, tw.end),
            });
        }
        let matched = times.iter().filter(|t| t.is_some()).count();
        let complete = total > 0
            && times[0].is_some()
            && times[total - 1].is_some()
            && matched as f64 >= 0.85 * total as f64;
        let words = interpolate(&times);
        let first = times.iter().flatten().next().copied().unwrap_or((0.0, 0.0));
        let last = times.iter().flatten().last().copied().unwrap_or((0.0, 0.0));
        reads.push(Read {
            take: take.to_string(),
            line: l.id.to_string(),
            score: c.hit.score,
            matched,
            total,
            complete,
            first_word: t_word[c.lo],
            last_word: t_word[c.hi],
            start: first.0,
            end: last.1,
            words,
        });
    }
    reads.sort_by(|a, b| a.start.total_cmp(&b.start));

    // Stretches no read covers.
    let mut covered = vec![false; tr.words.len()];
    for r in &reads {
        for c in covered.iter_mut().take(r.last_word + 1).skip(r.first_word) {
            *c = true;
        }
    }
    let mut unmatched = Vec::new();
    let mut i = 0;
    while i < tr.words.len() {
        if covered[i] {
            i += 1;
            continue;
        }
        let s = i;
        while i < tr.words.len() && !covered[i] {
            i += 1;
        }
        let (a, b) = (tr.words[s].start, tr.words[i - 1].end);
        if b - a > 2.0 {
            unmatched.push((a, b));
        }
    }
    TakeAlignment {
        take: take.to_string(),
        reads,
        unmatched,
    }
}

/// Fills in unmatched words' times from their neighbours.
fn interpolate(times: &[Option<(f64, f64)>]) -> Vec<(f64, f64)> {
    let n = times.len();
    let known: Vec<usize> = (0..n).filter(|&i| times[i].is_some()).collect();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if let Some(t) = times[i] {
            out.push(t);
            continue;
        }
        let prev = known.iter().rev().find(|&&k| k < i).copied();
        let next = known.iter().find(|&&k| k > i).copied();
        out.push(match (prev, next) {
            (Some(p), Some(q)) => {
                // The gap between the neighbours, shared evenly.
                let a = times[p].unwrap().1;
                let b = times[q].unwrap().0.max(a);
                let slot = (b - a) / (q - p - 1) as f64;
                let k = (i - p - 1) as f64;
                (a + k * slot, a + (k + 1.0) * slot)
            }
            (Some(p), None) => {
                let e = times[p].unwrap().1;
                (e, e)
            }
            (None, Some(q)) => {
                let s = times[q].unwrap().0;
                (s, s)
            }
            (None, None) => (0.0, 0.0),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    /// A transcript of these words, 0.4 s each with 0.1 s gaps, starting at `t0`.
    fn transcript(text: &str, t0: f64) -> Vec<TWord> {
        text.split_whitespace()
            .enumerate()
            .map(|(i, w)| TWord {
                w: w.into(),
                start: t0 + i as f64 * 0.5,
                end: t0 + i as f64 * 0.5 + 0.4,
            })
            .collect()
    }

    #[test]
    fn normalises_case_punctuation_and_numbers() {
        assert_eq!(normalize("Thirty-one,"), vec!["thirty", "one"]);
        assert_eq!(normalize("31"), vec!["thirty", "one"]);
        assert_eq!(
            normalize("1,500"),
            vec!["one", "thousand", "five", "hundred"]
        );
        assert_eq!(normalize("haven't"), vec!["havent"]);
        assert_eq!(normalize("—"), Vec::<String>::new());
        assert_eq!(number_words(2026).join(" "), "two thousand twenty six");
    }

    #[test]
    fn finds_retakes_and_marks_the_complete_ones() {
        let a = words("Thirty days ago this field was empty.");
        let b = words("Today it plants, waters and harvests itself.");
        let lines = [
            Query {
                id: "hook.1",
                words: &a,
            },
            Query {
                id: "hook.2",
                words: &b,
            },
        ];
        // A false start, a full read, an ad-lib, the second line twice.
        let mut tw = transcript("thirty days ago this", 0.0);
        tw.extend(transcript("thirty days ago this field was empty", 3.0));
        tw.extend(transcript(
            "um wait let me grab some coffee before we keep going okay",
            7.0,
        ));
        tw.extend(transcript(
            "today it plants waters and harvests itself",
            14.0,
        ));
        tw.extend(transcript(
            "today it plants waters and harvest itself",
            19.0,
        ));
        let al = align_take(
            "A001",
            &lines,
            &Transcript {
                sha256: String::new(),
                words: tw,
            },
        );
        let l1: Vec<&Read> = al.reads.iter().filter(|r| r.line == "hook.1").collect();
        let l2: Vec<&Read> = al.reads.iter().filter(|r| r.line == "hook.2").collect();
        assert_eq!(l1.len(), 2, "{:?}", al.reads);
        assert!(!l1[0].complete && l1[1].complete);
        assert!((l1[1].start - 3.0).abs() < 1e-9);
        assert!((l1[1].end - 6.4).abs() < 1e-9);
        assert_eq!(l2.len(), 2);
        assert!(l2.iter().all(|r| r.complete), "harvest ~ harvests counts");
        assert_eq!(al.unmatched.len(), 1);
        assert!((al.unmatched[0].0 - 7.0).abs() < 1e-9);
    }

    #[test]
    fn digits_in_the_script_match_spoken_numbers() {
        let a = words("We planted 30 rows.");
        let lines = [Query {
            id: "x.1",
            words: &a,
        }];
        let tw = transcript("we planted thirty rows", 1.0);
        let al = align_take(
            "T",
            &lines,
            &Transcript {
                sha256: String::new(),
                words: tw,
            },
        );
        assert_eq!(al.reads.len(), 1);
        assert!(al.reads[0].complete);
        assert_eq!(al.reads[0].words.len(), 4);
    }

    #[test]
    fn unmatched_words_are_interpolated() {
        let t = interpolate(&[Some((0.0, 1.0)), None, Some((2.0, 3.0)), None]);
        assert_eq!(t[1], (1.0, 2.0));
        assert_eq!(t[3], (3.0, 3.0));
    }
}
