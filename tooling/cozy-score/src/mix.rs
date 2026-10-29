//! Mixing and mastering: constant-power panning, convolution reverb, fades,
//! EQ, peak saturation and loudness normalisation.

use crate::dsp::{Biquad, SR};
use crate::rng::Rng;
use crate::types::Instrument;
use core::f64::consts::PI;

#[derive(Clone, Debug, Default)]
pub struct Stereo {
    pub l: Vec<f64>,
    pub r: Vec<f64>,
}

impl Stereo {
    pub fn silent(len: usize) -> Self {
        Stereo {
            l: vec![0.0; len],
            r: vec![0.0; len],
        }
    }

    pub fn len(&self) -> usize {
        self.l.len()
    }

    pub fn is_empty(&self) -> bool {
        self.l.is_empty()
    }

    pub fn add(&mut self, other: &Stereo) {
        for (a, b) in self.l.iter_mut().zip(&other.l) {
            *a += b;
        }
        for (a, b) in self.r.iter_mut().zip(&other.r) {
            *a += b;
        }
    }

    pub fn scale(&mut self, g: f64) {
        self.l.iter_mut().for_each(|x| *x *= g);
        self.r.iter_mut().for_each(|x| *x *= g);
    }

    pub fn peak(&self) -> f64 {
        self.l
            .iter()
            .chain(&self.r)
            .fold(0.0, |a, v| a.max(v.abs()))
    }

    pub fn channels(&self) -> [&[f64]; 2] {
        [&self.l, &self.r]
    }
}

/// Dry stereo plus the mono reverb send of a group of voices.
pub struct Bed {
    pub dry: Stereo,
    pub send: Vec<f64>,
}

/// Constant-power pan gains for p in [-1, 1].
pub fn pan_gains(p: f64) -> (f64, f64) {
    let theta = (p + 1.0) * PI / 4.0;
    (libm::cos(theta), libm::sin(theta))
}

/// Adds one rendered note at sample `at`, panned and sent to the reverb.
pub fn add_voice(bed: &mut Bed, at: usize, x: &[f64], inst: Instrument) {
    let len = bed.dry.len();
    if at >= len {
        return;
    }
    let (gl, gr) = pan_gains(inst.pan());
    let sl = inst.send();
    let n = x.len().min(len - at);
    for (k, v) in x[..n].iter().enumerate() {
        let i = at + k;
        bed.dry.l[i] += gl * v;
        bed.dry.r[i] += gr * v;
        if sl > 0.0 {
            bed.send[i] += sl * v;
        }
    }
}

/// 1.9 s stereo impulse response: two seeded noise IRs with an e^{-t/0.45}
/// decay, low-passed at 5 kHz, the first 12 ms zeroed, energy-normalised.
pub fn reverb_ir(seed: u64) -> (Vec<f64>, Vec<f64>) {
    let n = (1.9 * SR).round() as usize;
    let pre = (0.012 * SR).round() as usize;
    let root = Rng::new(seed).fork("reverb");
    let make = |label: &str| {
        let mut r = root.fork(label);
        let mut lp = Biquad::lowpass(5000.0, SR);
        let mut ir: Vec<f64> = (0..n)
            .map(|i| lp.tick(r.gauss() * libm::exp(-(i as f64 / SR) / 0.45)))
            .collect();
        ir[..pre].iter_mut().for_each(|x| *x = 0.0);
        let e: f64 = ir.iter().map(|x| x * x).sum();
        let g = 1.0 / libm::sqrt(e);
        ir.iter_mut().for_each(|x| *x *= g);
        ir
    };
    (make("left"), make("right"))
}

pub const WET: f64 = 0.9;

/// High-pass at 35 Hz, then a ~+3 dB high shelf above 5 kHz.
pub fn master_eq(st: &mut Stereo) {
    for ch in [&mut st.l, &mut st.r] {
        let mut hp = Biquad::highpass(35.0, SR);
        let mut sh = Biquad::high_shelf(5000.0, 20.0 * libm::log10(1.4), SR);
        for x in ch.iter_mut() {
            *x = sh.tick(hp.tick(*x));
        }
    }
}

/// Light saturation on the peaks: tanh(x/peak*1.4)*peak/1.4.
pub fn saturate(st: &mut Stereo, peak: f64) {
    if peak <= 0.0 {
        return;
    }
    for x in st.l.iter_mut().chain(st.r.iter_mut()) {
        *x = libm::tanh(*x / peak * 1.4) * peak / 1.4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_power() {
        for p in [-1.0, -0.35, 0.0, 0.1, 0.4, 1.0] {
            let (l, r) = pan_gains(p);
            assert!((l * l + r * r - 1.0).abs() < 1e-12);
        }
        let (l, r) = pan_gains(0.0);
        assert!((l - r).abs() < 1e-12);
    }

    #[test]
    fn ir_is_normalised_and_predelayed() {
        let (l, r) = reverb_ir(7);
        assert_eq!(l.len(), (1.9 * SR).round() as usize);
        let e: f64 = l.iter().map(|x| x * x).sum();
        assert!((e - 1.0).abs() < 1e-9);
        assert!(l[..529].iter().all(|x| *x == 0.0));
        assert_ne!(l, r);
    }
}
