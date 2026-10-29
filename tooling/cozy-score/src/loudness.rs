//! ITU-R BS.1770-4 loudness: K-weighting, 400 ms blocks with 75% overlap,
//! absolute (-70 LUFS) and relative (-10 LU) gating.

use crate::dsp::Biquad;
use core::f64::consts::PI;

/// K-weighting filters for any sample rate (the BS.1770 analogue prototypes,
/// re-derived per rate as libebur128 does).
pub fn k_filters(fs: f64) -> (Biquad, Biquad) {
    // Stage 1: high-shelf "head" filter.
    let f0 = 1681.974450955533;
    let g = 3.999843853973347;
    let q = 0.7071752369554196;
    let k = libm::tan(PI * f0 / fs);
    let vh = libm::pow(10.0, g / 20.0);
    let vb = libm::pow(vh, 0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad::new(
        (vh + vb * k / q + k * k) / a0,
        2.0 * (k * k - vh) / a0,
        (vh - vb * k / q + k * k) / a0,
        2.0 * (k * k - 1.0) / a0,
        (1.0 - k / q + k * k) / a0,
    );
    // Stage 2: RLB high-pass.
    let f0 = 38.13547087602444;
    let q = 0.5003270373238773;
    let k = libm::tan(PI * f0 / fs);
    let a0 = 1.0 + k / q + k * k;
    let hp = Biquad::new(
        1.0,
        -2.0,
        1.0,
        2.0 * (k * k - 1.0) / a0,
        (1.0 - k / q + k * k) / a0,
    );
    (shelf, hp)
}

/// Squared K-weighted samples summed over channels (all weights 1, as for
/// L and R).
pub fn k_power(channels: &[&[f64]], fs: f64) -> Vec<f64> {
    let n = channels.iter().map(|c| c.len()).max().unwrap_or(0);
    let mut out = vec![0.0; n];
    for ch in channels {
        let (mut a, mut b) = k_filters(fs);
        for (i, &x) in ch.iter().enumerate() {
            let y = b.tick(a.tick(x));
            out[i] += y * y;
        }
    }
    out
}

fn to_lufs(z: f64) -> f64 {
    -0.691 + 10.0 * libm::log10(z)
}

/// Gated loudness over samples [start, end) of a K-weighted power signal.
/// None when the range is shorter than one block or entirely gated out.
pub fn gated(power: &[f64], fs: f64, start: usize, end: usize) -> Option<f64> {
    let end = end.min(power.len());
    let block = (0.4 * fs).round() as usize;
    let hop = (0.1 * fs).round() as usize;
    if end <= start || end - start < block {
        return None;
    }
    // Prefix sums for the block means.
    let mut prefix = Vec::with_capacity(end - start + 1);
    let mut acc = 0.0;
    prefix.push(0.0);
    for &p in &power[start..end] {
        acc += p;
        prefix.push(acc);
    }
    let mut blocks = Vec::new();
    let mut s = 0;
    while s + block <= end - start {
        blocks.push((prefix[s + block] - prefix[s]) / block as f64);
        s += hop;
    }
    let abs: Vec<f64> = blocks
        .into_iter()
        .filter(|&z| z > 0.0 && to_lufs(z) > -70.0)
        .collect();
    if abs.is_empty() {
        return None;
    }
    let rel = to_lufs(abs.iter().sum::<f64>() / abs.len() as f64) - 10.0;
    let kept: Vec<f64> = abs.into_iter().filter(|&z| to_lufs(z) > rel).collect();
    if kept.is_empty() {
        return None;
    }
    Some(to_lufs(kept.iter().sum::<f64>() / kept.len() as f64))
}

/// Integrated loudness of a whole stereo (or mono) signal.
pub fn integrated(channels: &[&[f64]], fs: f64) -> Option<f64> {
    let p = k_power(channels, fs);
    gated(&p, fs, 0, p.len())
}

pub fn sample_peak(channels: &[&[f64]]) -> f64 {
    channels
        .iter()
        .flat_map(|c| c.iter())
        .fold(0.0, |a, v| a.max(v.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::SR;

    #[test]
    fn sine_1k_at_minus_20_dbfs_one_channel() {
        // A 1 kHz sine with peak 0.1 (-20 dBFS) on one channel for 5 s.
        let x: Vec<f64> = (0..(5.0 * SR) as usize)
            .map(|i| 0.1 * libm::sin(2.0 * PI * 1000.0 * i as f64 / SR))
            .collect();
        let silent = vec![0.0; x.len()];
        let got = integrated(&[&x, &silent], SR).unwrap();
        // Expected: mean square 0.1^2/2 = 0.005 -> -23.0103 dB, plus the
        // K-weighting gain at 1 kHz (|H|^2 in dB), minus 0.691.
        let (a, b) = k_filters(SR);
        let h = a.magnitude(1000.0, SR) * b.magnitude(1000.0, SR);
        let expected = -0.691 + 10.0 * libm::log10(0.005 * h * h);
        // At 44.1 kHz the K-filter gain at 1 kHz is +0.7005 dB, so the
        // expected reading is -23.0103 + 0.7005 - 0.691 = -23.0008 LUFS.
        assert!(
            (expected - (-23.0008)).abs() < 0.0005,
            "expected {expected}"
        );
        assert!(
            (got - expected).abs() < 0.01,
            "got {got}, expected {expected}"
        );
    }

    #[test]
    fn stereo_sine_reads_like_ebu_reference() {
        // EBU Tech 3341 case 1: 1 kHz at -23 dBFS on both channels -> -23 LUFS.
        let amp = libm::pow(10.0, -23.0 / 20.0);
        let x: Vec<f64> = (0..(10.0 * SR) as usize)
            .map(|i| amp * libm::sin(2.0 * PI * 1000.0 * i as f64 / SR))
            .collect();
        let got = integrated(&[&x, &x], SR).unwrap();
        assert!((got + 23.0).abs() < 0.1, "{got}");
    }

    #[test]
    fn gating_ignores_silence() {
        let amp = 0.1;
        let mut x: Vec<f64> = (0..(4.0 * SR) as usize)
            .map(|i| amp * libm::sin(2.0 * PI * 1000.0 * i as f64 / SR))
            .collect();
        x.extend(vec![0.0; (2.0 * SR) as usize]);
        let short_tail = integrated(&[&x], SR).unwrap();
        x.extend(vec![0.0; (30.0 * SR) as usize]);
        let long_tail = integrated(&[&x], SR).unwrap();
        // Silent blocks are gated out, so more silence changes nothing.
        assert!((short_tail - long_tail).abs() < 1e-9);
        assert!((long_tail + 23.0).abs() < 0.3, "{long_tail}");
        assert!(gated(&k_power(&[&x], SR), SR, 0, 100).is_none());
    }
}
