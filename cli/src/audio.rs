//! The mix (spec §9.2): buses at 48 kHz, the duck under speech, BS.1770-4
//! loudness (cozy-score's K-weighting), true peak at 4× oversampling, and a
//! lookahead limiter for the ceiling.

use serde::Serialize;
use std::path::Path;

/// Stereo f32 audio.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Buf {
    pub l: Vec<f32>,
    pub r: Vec<f32>,
}

impl Buf {
    pub fn silent(n: usize) -> Buf {
        Buf {
            l: vec![0.0; n],
            r: vec![0.0; n],
        }
    }

    pub fn len(&self) -> usize {
        self.l.len()
    }

    pub fn is_empty(&self) -> bool {
        self.l.is_empty()
    }

    /// Adds `src` from sample `at`, cut off at the end of `self`.
    pub fn add_at(&mut self, src: &Buf, at: usize, gain: f32) {
        let n = self.len();
        for i in 0..src.len() {
            let j = at + i;
            if j >= n {
                break;
            }
            self.l[j] += src.l[i] * gain;
            self.r[j] += src.r[i] * gain;
        }
    }

    pub fn scale(&mut self, g: f32) {
        self.l
            .iter_mut()
            .chain(self.r.iter_mut())
            .for_each(|x| *x *= g);
    }
}

/// Raised-cosine fades of `n` samples at both ends, against clicks at edits.
pub fn fade_edges(b: &mut Buf, n: usize) {
    let len = b.len();
    let n = n.min(len / 2);
    for i in 0..n {
        let g = 0.5 - 0.5 * (std::f32::consts::PI * (i as f32 + 0.5) / n as f32).cos();
        b.l[i] *= g;
        b.r[i] *= g;
        b.l[len - 1 - i] *= g;
        b.r[len - 1 - i] *= g;
    }
}

/// The music's gain over time: `duck_db` under every voice span, eased in
/// over `attack_ms` (starting that long before the voice) and out over
/// `release_ms`.
pub fn duck_envelope(
    n: usize,
    spans: &[(f64, f64)],
    duck_db: f64,
    attack_ms: f64,
    release_ms: f64,
    sr: u32,
) -> Vec<f32> {
    let sr = f64::from(sr);
    let duck = 10f64.powf(duck_db / 20.0);
    let mut target = vec![1.0f64; n];
    let lead = attack_ms / 1000.0;
    for &(s, e) in spans {
        let a = (((s - lead).max(0.0)) * sr) as usize;
        let b = ((e * sr) as usize).min(n);
        for t in target.iter_mut().take(b).skip(a.min(n)) {
            *t = duck;
        }
    }
    let down = (-1.0 / (attack_ms / 1000.0 * sr / 5.0)).exp();
    let up = (-1.0 / (release_ms / 1000.0 * sr / 5.0)).exp();
    let mut g = 1.0f64;
    target
        .into_iter()
        .map(|t| {
            let c = if t < g { down } else { up };
            g = t + (g - t) * c;
            g as f32
        })
        .collect()
}

/// Integrated loudness (BS.1770-4) in LUFS; None for silence.
pub fn integrated_lufs(b: &Buf, sr: u32) -> Option<f64> {
    let fs = f64::from(sr);
    let hop = (0.1 * fs).round() as usize;
    if hop == 0 {
        return None;
    }
    let hops = b.len() / hop;
    let mut hop_power = vec![0.0f64; hops];
    for ch in [&b.l, &b.r] {
        let (mut shelf, mut hp) = cozy_score::loudness::k_filters(fs);
        for (i, &x) in ch.iter().enumerate() {
            let y = hp.tick(shelf.tick(f64::from(x)));
            let h = i / hop;
            if h < hops {
                hop_power[h] += y * y;
            }
        }
    }
    if hops < 4 {
        return None;
    }
    let block = (4 * hop) as f64;
    let to_lufs = |z: f64| -0.691 + 10.0 * z.log10();
    let blocks: Vec<f64> = (0..=hops - 4)
        .map(|i| hop_power[i..i + 4].iter().sum::<f64>() / block)
        .filter(|&z| z > 0.0 && to_lufs(z) > -70.0)
        .collect();
    if blocks.is_empty() {
        return None;
    }
    let rel = to_lufs(blocks.iter().sum::<f64>() / blocks.len() as f64) - 10.0;
    let kept: Vec<f64> = blocks.into_iter().filter(|&z| to_lufs(z) > rel).collect();
    if kept.is_empty() {
        return None;
    }
    Some(to_lufs(kept.iter().sum::<f64>() / kept.len() as f64))
}

/// A windowed-sinc low-pass for 4× oversampling, 48 taps.
fn oversampling_filter() -> Vec<f64> {
    const TAPS: usize = 48;
    let c = (TAPS - 1) as f64 / 2.0;
    (0..TAPS)
        .map(|j| {
            let x = (j as f64 - c) / 4.0;
            let sinc = if x.abs() < 1e-12 {
                1.0
            } else {
                (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
            };
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * (j as f64 + 0.5) / TAPS as f64).cos();
            sinc * w
        })
        .collect()
}

/// True peak, linear: the largest sample of the 4× oversampled signal.
pub fn true_peak(b: &Buf) -> f64 {
    let h = oversampling_filter();
    // Normalise each phase to unity gain.
    let mut phases: Vec<Vec<f64>> = (0..4)
        .map(|p| h.iter().skip(p).step_by(4).copied().collect())
        .collect();
    for ph in phases.iter_mut() {
        let s: f64 = ph.iter().sum();
        if s.abs() > 1e-12 {
            ph.iter_mut().for_each(|v| *v /= s);
        }
    }
    let mut peak = 0.0f64;
    for ch in [&b.l, &b.r] {
        for n in 0..ch.len() {
            peak = peak.max(f64::from(ch[n]).abs());
            for ph in &phases {
                let mut y = 0.0;
                for (k, &c) in ph.iter().enumerate() {
                    let idx = n + ph.len() / 2;
                    if idx >= k && idx - k < ch.len() {
                        y += c * f64::from(ch[idx - k]);
                    }
                }
                peak = peak.max(y.abs());
            }
        }
    }
    peak
}

/// A lookahead peak limiter: the gain is down before each peak arrives and
/// recovers over `release_ms`, so no sample passes `ceiling` (linear).
pub fn limit(b: &mut Buf, ceiling: f32, sr: u32) -> bool {
    let n = b.len();
    let need: Vec<f32> = (0..n)
        .map(|i| {
            let p = b.l[i].abs().max(b.r[i].abs());
            if p > ceiling {
                ceiling / p
            } else {
                1.0
            }
        })
        .collect();
    if need.iter().all(|&g| g >= 1.0) {
        return false;
    }
    let look = (0.0015 * f64::from(sr)) as usize + 1;
    // m[k] = min(need[k..=k+look]) with a monotonic deque.
    let mut m = vec![1.0f32; n];
    let mut dq: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for i in (0..n).rev() {
        while dq.back().is_some_and(|&j| need[j] >= need[i]) {
            dq.pop_back();
        }
        dq.push_back(i);
        while dq.front().is_some_and(|&j| j > i + look) {
            dq.pop_front();
        }
        m[i] = need[*dq.front().unwrap()];
    }
    // s[i] = mean(m[i-look+1..=i]): smooth, and never above need[i].
    let mut s = vec![1.0f32; n];
    let mut acc = 0.0f64;
    for i in 0..n {
        acc += f64::from(m[i]);
        if i >= look {
            acc -= f64::from(m[i - look]);
        }
        let count = (i + 1).min(look) as f64;
        let pad = look as f64 - count;
        s[i] = ((acc + pad) / look as f64) as f32;
    }
    let rel = (-1.0 / (0.06 * f64::from(sr))).exp() as f32;
    let mut g = 1.0f32;
    for (i, &si) in s.iter().enumerate() {
        g = (1.0 - (1.0 - g) * rel).min(si);
        b.l[i] *= g;
        b.r[i] *= g;
    }
    true
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Master {
    pub integrated_lufs: Option<f64>,
    pub true_peak_dbtp: f64,
    pub gain_db: f64,
    /// The limiter had to work, or the ceiling held the level down.
    pub limited: bool,
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-12).log10()
}

/// Brings the mix to `target_lufs` with true peak at or under `ceiling_db`.
pub fn master(b: &mut Buf, target_lufs: f64, ceiling_db: f64, sr: u32) -> Master {
    let Some(start) = integrated_lufs(b, sr) else {
        return Master {
            integrated_lufs: None,
            true_peak_dbtp: db(true_peak(b)),
            gain_db: 0.0,
            limited: false,
        };
    };
    let ceiling = 10f64.powf(ceiling_db / 20.0);
    // The limiter works on samples; peaks between samples can still pass
    // the ceiling, so its ceiling tightens until the true peak fits.
    let mut sample_ceiling_db = ceiling_db - 0.3;
    let mut total_db = 0.0;
    let mut limited = false;
    let mut lufs = start;
    let mut tp = 0.0;
    for _ in 0..3 {
        let sample_ceiling = 10f64.powf(sample_ceiling_db / 20.0) as f32;
        limited |= limit(b, sample_ceiling, sr);
        lufs = integrated_lufs(b, sr).unwrap_or(lufs);
        for _ in 0..16 {
            let step = target_lufs - lufs;
            if step.abs() < 0.05 {
                break;
            }
            b.scale(10f64.powf(step / 20.0) as f32);
            total_db += step;
            limited |= limit(b, sample_ceiling, sr);
            lufs = integrated_lufs(b, sr).unwrap_or(lufs);
        }
        tp = true_peak(b);
        if tp <= ceiling {
            break;
        }
        sample_ceiling_db -= db(tp / ceiling) + 0.1;
    }
    if tp > ceiling {
        let g = ceiling / tp;
        b.scale(g as f32);
        total_db += db(g);
        limited = true;
        lufs = integrated_lufs(b, sr).unwrap_or(lufs);
    }
    Master {
        integrated_lufs: Some((lufs * 100.0).round() / 100.0),
        true_peak_dbtp: (db(true_peak(b)) * 100.0).round() / 100.0,
        gain_db: (total_db * 100.0).round() / 100.0,
        limited,
    }
}

/// Writes 32-bit float stereo WAV.
pub fn write_wav(path: &Path, b: &Buf, sr: u32) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: sr,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let mut w = hound::WavWriter::create(path, spec)
        .map_err(|e| format!("can't write {}: {e}", path.display()))?;
    for i in 0..b.len() {
        w.write_sample(b.l[i]).map_err(|e| e.to_string())?;
        w.write_sample(b.r[i]).map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, amp: f32, secs: f64, sr: u32) -> Buf {
        let n = (secs * f64::from(sr)) as usize;
        let l: Vec<f32> = (0..n)
            .map(|i| {
                amp * (2.0 * std::f64::consts::PI * freq * i as f64 / f64::from(sr)).sin() as f32
            })
            .collect();
        Buf { l: l.clone(), r: l }
    }

    #[test]
    fn a_sine_measures_as_bs1770_says() {
        // A 1 kHz sine at -20 dBFS on one channel reads -23.0 LUFS (the
        // reference cozy-score's meter is tested against).
        let mut b = sine(1000.0, 0.1, 5.0, 48_000);
        b.r.iter_mut().for_each(|x| *x = 0.0);
        let l = integrated_lufs(&b, 48_000).unwrap();
        assert!((l + 23.0).abs() < 0.05, "{l}");
        assert!(integrated_lufs(&Buf::silent(48_000 * 2), 48_000).is_none());
    }

    #[test]
    fn true_peak_finds_intersample_peaks() {
        // A sine at fs/4 sampled at 45° phase peaks between samples, 3 dB up.
        let sr = 48_000;
        let n = 4800;
        let l: Vec<f32> = (0..n)
            .map(|i| {
                (std::f64::consts::FRAC_PI_2 * i as f64 + std::f64::consts::FRAC_PI_4).sin() as f32
                    * 0.5
            })
            .collect();
        let b = Buf { l: l.clone(), r: l };
        let sample_peak = b.l.iter().fold(0.0f32, |a, x| a.max(x.abs()));
        let tp = true_peak(&b);
        assert!((f64::from(sample_peak) - 0.3536).abs() < 1e-3);
        assert!(tp > 0.45 && tp < 0.55, "{tp}");
        let _ = sr;
    }

    #[test]
    fn duck_eases_down_before_speech_and_back_after() {
        let sr = 1000;
        let env = duck_envelope(3000, &[(1.0, 2.0)], -9.0, 60.0, 350.0, sr);
        let duck = 10f32.powf(-9.0 / 20.0);
        assert!((env[0] - 1.0).abs() < 1e-6);
        assert!(
            (env[1000] - duck).abs() < 0.02,
            "down by the time the voice starts: {}",
            env[1000]
        );
        assert!((env[1999] - duck).abs() < 1e-3);
        assert!(env[2100] > duck && env[2100] < 1.0);
        assert!((env[2999] - 1.0).abs() < 0.01);
    }

    #[test]
    fn master_hits_the_target_under_the_ceiling() {
        // Speech-like: quiet sine with loud bursts.
        let sr = 48_000;
        let mut b = sine(220.0, 0.05, 10.0, sr);
        for burst in 0..10 {
            let s = burst * sr as usize + 1000;
            for i in s..s + 2000 {
                b.l[i] *= 14.0;
                b.r[i] *= 14.0;
            }
        }
        let m = master(&mut b, -14.0, -1.0, sr);
        let l = m.integrated_lufs.unwrap();
        assert!((l + 14.0).abs() < 0.3, "{m:?}");
        assert!(m.true_peak_dbtp <= -1.0 + 1e-6, "{m:?}");
    }

    #[test]
    fn fades_reach_zero_at_the_edges() {
        let mut b = Buf {
            l: vec![1.0; 100],
            r: vec![1.0; 100],
        };
        fade_edges(&mut b, 10);
        assert!(b.l[0] < 0.05 && b.l[99] < 0.05);
        assert_eq!(b.l[50], 1.0);
    }
}
