//! Spectral band balance, for the cue sheet and `cozy-score analyze`.

use crate::dsp::{Fft, SR};
use core::f64::consts::PI;
use serde::Serialize;

pub const BANDS: [(f64, f64, &str); 6] = [
    (20.0, 60.0, "20-60"),
    (60.0, 250.0, "60-250"),
    (250.0, 1000.0, "250-1k"),
    (1000.0, 3000.0, "1k-3k"),
    (3000.0, 8000.0, "3k-8k"),
    (8000.0, 20000.0, "8k-20k"),
];

#[derive(Clone, Debug, Serialize)]
pub struct BandLevel {
    pub band: &'static str,
    /// Energy relative to the 20 Hz-20 kHz total, in dB.
    pub db: f64,
}

/// Welch power spectrum (8192-point Hann, 50% overlap) of both channels over
/// samples [start, end), summed into bands.
pub fn band_levels(l: &[f64], r: &[f64], start: usize, end: usize) -> Vec<BandLevel> {
    let n = 8192;
    let fft = Fft::new(n);
    let win: Vec<f64> = (0..n)
        .map(|i| 0.5 - 0.5 * libm::cos(2.0 * PI * i as f64 / n as f64))
        .collect();
    let mut power = vec![0.0; n / 2];
    let end = end.min(l.len());
    let mut s = start;
    while s + n <= end {
        for ch in [l, r] {
            let mut re: Vec<f64> = (0..n).map(|i| ch[s + i] * win[i]).collect();
            let mut im = vec![0.0; n];
            fft.forward(&mut re, &mut im);
            for k in 0..n / 2 {
                power[k] += re[k] * re[k] + im[k] * im[k];
            }
        }
        s += n / 2;
    }
    let bin = SR / n as f64;
    let sum = |lo: f64, hi: f64| -> f64 {
        power
            .iter()
            .enumerate()
            .filter(|(k, _)| {
                let f = *k as f64 * bin;
                f >= lo && f < hi
            })
            .map(|(_, p)| *p)
            .sum()
    };
    let total = sum(20.0, 20000.0).max(1e-30);
    BANDS
        .iter()
        .map(|&(lo, hi, name)| BandLevel {
            band: name,
            db: 10.0 * libm::log10(sum(lo, hi).max(1e-30) / total),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_lands_in_its_band() {
        let x: Vec<f64> = (0..44100)
            .map(|i| libm::sin(2.0 * PI * 1500.0 * i as f64 / SR))
            .collect();
        let b = band_levels(&x, &x, 0, x.len());
        let loudest = b.iter().max_by(|a, c| a.db.total_cmp(&c.db)).unwrap();
        assert_eq!(loudest.band, "1k-3k");
        assert!(loudest.db > -0.1);
    }
}
