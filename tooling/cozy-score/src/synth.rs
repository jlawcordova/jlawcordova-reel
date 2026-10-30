//! Instrument recipes. Every voice renders to a mono buffer starting at its
//! note-on; the mixer pans it. Noise comes from a per-note fork of the seeded
//! RNG, keyed by the note's stable id.

use crate::arrange::Art;
use crate::dsp::{add_partials, ramp_ends, Biquad, Partial, SR};
use crate::harmony::hz;
use crate::resolve::Voice;
use crate::rng::Rng;
use crate::types::Instrument;
use core::f64::consts::PI;
use std::collections::BTreeMap;
use std::rc::Rc;

fn samples(sec: f64) -> usize {
    (sec * SR).round().max(1.0) as usize
}

/// Renders voices; caches the noise-free part of repeated notes.
pub struct Synth {
    root: Rng,
    cache: BTreeMap<(Instrument, i32, usize, u8), Rc<Vec<f64>>>,
}

impl Synth {
    pub fn new(seed: u64) -> Self {
        Synth {
            root: Rng::new(seed).fork("synth"),
            cache: BTreeMap::new(),
        }
    }

    fn cached(
        &mut self,
        key: (Instrument, i32, usize, u8),
        make: impl FnOnce() -> Vec<f64>,
    ) -> Rc<Vec<f64>> {
        self.cache
            .entry(key)
            .or_insert_with(|| Rc::new(make()))
            .clone()
    }

    /// Mono samples for a voice, already scaled by its amplitude.
    pub fn render(&mut self, v: &Voice) -> Vec<f64> {
        let mut rng = self.root.fork_n(v.event.id);
        let m = v.midi;
        let dur = v.event.dur;
        let mut out = match v.instrument {
            Instrument::Flute => flute(m, dur, &mut rng),
            Instrument::Guitar => {
                let (len, tail) = match v.event.art {
                    Art::Eighth => (rng.range(1.4, 1.5), 0.05),
                    Art::Quarter => (2.2, 0.05),
                    Art::Strum { ring } => (ring, 0.05),
                    Art::Muted => (0.25, 0.04),
                    _ => ((dur + 0.4).clamp(0.6, 2.2), 0.05),
                };
                // Quantise ring length to 10 ms so notes share cache entries.
                let n = samples((len * 100.0).round() / 100.0);
                let tail_n = samples(tail);
                let tone = self.cached((Instrument::Guitar, m, n, 0), || guitar_tone(m, n, tail_n));
                let mut out = tone.to_vec();
                guitar_burst(&mut out, &mut rng);
                ramp_ends(&mut out, samples(0.002), 0);
                out
            }
            Instrument::Mallet => {
                let (len, tail) = match v.event.art {
                    Art::Muted => (0.5, 0.03),
                    _ => (1.6, 0.03),
                };
                let n = samples(len);
                self.cached((Instrument::Mallet, m, n, 0), || {
                    mallet(m, n, samples(tail))
                })
                .to_vec()
            }
            Instrument::Pad => pad(m, dur, &mut rng),
            Instrument::HeavyBass => {
                let n = samples(dur);
                self.cached((Instrument::HeavyBass, m, n, 0), || heavy_bass(m, n))
                    .to_vec()
            }
            Instrument::SoftBass => {
                let n = samples(dur);
                self.cached((Instrument::SoftBass, m, n, 0), || soft_bass(m, n))
                    .to_vec()
            }
            Instrument::Kick => {
                let mut out = self.cached((Instrument::Kick, 0, 0, 0), kick_tone).to_vec();
                for (i, x) in out.iter_mut().take(samples(0.003)).enumerate() {
                    let fade = 1.0 - i as f64 / samples(0.003) as f64;
                    *x += 0.1 * rng.gauss() * fade;
                }
                out
            }
            Instrument::Shaker => shaker(&mut rng),
            Instrument::Rim => rim(&mut rng),
            Instrument::Chip => {
                let hold = match v.event.art {
                    Art::Muted => 0.08,
                    _ => dur.max(0.03),
                };
                let n = samples(hold);
                self.cached((Instrument::Chip, m, n, 0), || chip(m, n))
                    .to_vec()
            }
            Instrument::SynthPluck => {
                let len = match v.event.art {
                    Art::Strum { ring } => ring.clamp(0.6, 1.2),
                    Art::Muted => 0.3,
                    _ => 0.6,
                };
                let n = samples(len);
                let tone = self.cached((Instrument::SynthPluck, m, n, 0), || {
                    synth_pluck_tone(m, n, samples(0.04))
                });
                let mut out = tone.to_vec();
                guitar_burst(&mut out, &mut rng);
                ramp_ends(&mut out, samples(0.002), 0);
                out
            }
            Instrument::Blip => self
                .cached((Instrument::Blip, m, 0, 0), || blip(m))
                .to_vec(),
            Instrument::Clap => clap(&mut rng),
            Instrument::Riser => riser(dur, v.event.art == Art::Reverse, &mut rng),
        };
        match v.event.art {
            Art::Glide => out = glide(&out, 40.0, 0.06),
            Art::Swell => swell(&mut out, 0.12),
            _ => {}
        }
        for x in out.iter_mut() {
            *x *= v.amp;
        }
        out
    }
}

/// Plays `x` starting `cents` flat and gliding up to pitch over `time`
/// seconds (linear-interpolated resampling).
pub fn glide(x: &[f64], cents: f64, time: f64) -> Vec<f64> {
    let n = x.len();
    let mut out = vec![0.0; n];
    let mut pos = 0.0f64;
    for (i, o) in out.iter_mut().enumerate() {
        let t = i as f64 / SR;
        let g = (1.0 - t / time).max(0.0);
        let k = pos as usize;
        if k + 1 >= n {
            break;
        }
        let fr = pos - k as f64;
        *o = x[k] * (1.0 - fr) + x[k + 1] * fr;
        pos += libm::exp2(-cents * g / 1200.0);
    }
    out
}

/// Removes the attack: the first `time` seconds rise linearly from silence.
pub fn swell(x: &mut [f64], time: f64) {
    let n = samples(time);
    for (i, v) in x.iter_mut().take(n).enumerate() {
        *v *= i as f64 / n as f64;
    }
}

/// Chip lead: a band-limited 25% pulse, sum over k with f k <= 11 kHz of
/// sin(pi k duty)/k e^{-k/14} sin(k phi); vibrato 0.3% at 6 Hz fading in over
/// 0.12-0.22 s; 5 ms attack, 0.7 + 0.3 e^{-t/0.08}, 50 ms release.
pub fn chip(m: i32, n_hold: usize) -> Vec<f64> {
    let f = hz(m as f64);
    let rel = samples(0.05);
    let n = n_hold + rel;
    let k_max = ((11000.0 / f).floor() as usize).max(1);
    let coef: Vec<f64> = (1..=k_max)
        .map(|k| {
            let kf = k as f64;
            libm::sin(PI * kf * 0.25) / kf * libm::exp(-kf / 14.0)
        })
        .collect();
    let att = samples(0.005);
    let mut out = vec![0.0; n];
    let mut phase = 0.0f64;
    for (i, x) in out.iter_mut().enumerate() {
        let t = i as f64 / SR;
        let (s1, c1) = libm::sincos(phase);
        // sin(k phi) by the Chebyshev recurrence.
        let (mut prev, mut cur) = (0.0, s1);
        let mut v = 0.0;
        for a in &coef {
            v += a * cur;
            let next = 2.0 * c1 * cur - prev;
            prev = cur;
            cur = next;
        }
        let mut env = 0.7 + 0.3 * libm::exp(-t / 0.08);
        if i < att {
            env *= i as f64 / att as f64;
        }
        if i >= n_hold {
            env *= 1.0 - (i - n_hold) as f64 / rel as f64;
        }
        *x = env * v;
        let fade = ((t - 0.12) / 0.10).clamp(0.0, 1.0);
        let vib = 1.0 + 0.003 * fade * libm::sin(2.0 * PI * 6.0 * t);
        phase += 2.0 * PI * f * vib / SR;
        if phase > 2.0 * PI {
            phase -= 2.0 * PI;
        }
    }
    out
}

/// Synth pluck: the guitar's partials with a brighter decay,
/// e^{-t(1.6 + 0.9k/2)}, and the given tail.
pub fn synth_pluck_tone(m: i32, n: usize, tail: usize) -> Vec<f64> {
    let f = hz(m as f64);
    let mut out = vec![0.0; n];
    let mut parts = Vec::new();
    for k in 1..=13 {
        let kf = k as f64;
        let fk = f * kf * (1.0 + 0.0004 * kf * kf);
        if fk > 12000.0 {
            break;
        }
        parts.push(Partial {
            f: fk,
            amp: libm::fabs(libm::sin(PI * kf * 0.18)) / libm::pow(kf, 1.1),
            decay: 1.6 + 0.9 * kf / 2.0,
            phase: 0.0,
        });
    }
    add_partials(&mut out, &parts);
    ramp_ends(&mut out, 0, tail);
    out
}

/// Blip: sin(phi) + 0.2 sin(2 phi), the frequency gliding up as
/// f (0.72 + 0.28 (1 - e^{-t/0.018})); e^{-t/0.035}, 2 ms attack, 0.14 s.
pub fn blip(m: i32) -> Vec<f64> {
    let f = hz(m as f64);
    let tau = 0.018;
    let n = samples(0.14);
    let mut out: Vec<f64> = (0..n)
        .map(|i| {
            let t = i as f64 / SR;
            // The integral of the gliding frequency.
            let phi = 2.0 * PI * f * (t - 0.28 * tau * (1.0 - libm::exp(-t / tau)));
            (libm::sin(phi) + 0.2 * libm::sin(2.0 * phi)) * libm::exp(-t / 0.035)
        })
        .collect();
    ramp_ends(&mut out, samples(0.002), samples(0.01));
    out
}

/// Clap: uniform noise high-passed at 900 Hz and low-passed at 4.5 kHz, three
/// bursts e^{-(t-d)/0.006} at d = 0, 11, 22 ms and a tail
/// 0.6 e^{-(t-0.03)/0.07}; 0.25 s.
pub fn clap(rng: &mut Rng) -> Vec<f64> {
    let n = samples(0.25);
    let mut hp = Biquad::highpass(900.0, SR);
    let mut lp = Biquad::lowpass(4500.0, SR);
    let mut out: Vec<f64> = (0..n)
        .map(|i| {
            let t = i as f64 / SR;
            let mut env = 0.0;
            for d in [0.0, 0.011, 0.022] {
                if t >= d {
                    env += libm::exp(-(t - d) / 0.006);
                }
            }
            if t >= 0.03 {
                env += 0.6 * libm::exp(-(t - 0.03) / 0.07);
            }
            lp.tick(hp.tick(rng.range(-1.0, 1.0))) * env
        })
        .collect();
    ramp_ends(&mut out, samples(0.0005), samples(0.01));
    out
}

/// Riser: uniform noise high-passed at 2.5 kHz, (t/dur)^2.2 over `dur` (then a
/// 20 ms release), or reversed (1 - t/dur)^2 at 0.06/0.07 of the level.
pub fn riser(dur: f64, reverse: bool, rng: &mut Rng) -> Vec<f64> {
    let dur = dur.max(0.02);
    let n_hold = samples(dur);
    let rel = if reverse { 0 } else { samples(0.02) };
    let mut hp = Biquad::highpass(2500.0, SR);
    let mut out: Vec<f64> = (0..n_hold + rel)
        .map(|i| {
            let p = (i as f64 / n_hold as f64).min(1.0);
            let env = if reverse {
                libm::pow(1.0 - p, 2.0) * (0.06 / 0.07)
            } else {
                libm::pow(p, 2.2)
            };
            hp.tick(rng.range(-1.0, 1.0)) * env
        })
        .collect();
    ramp_ends(&mut out, samples(0.002), rel.max(samples(0.005)));
    out
}

/// Breathy flute: vibrato fading in from 0.18 s to 0.48 s at 5.2 Hz, depth
/// 0.45%; sin + 0.16 sin2 + 0.09 sin3 + 0.03 sin4 on the integrated phase;
/// band-passed breath at 0.05 plus a decaying chiff; 50 ms attack, 200 ms
/// release, +-8% swell at 0.7 Hz; sounds for 0.95 of the written length.
pub fn flute(m: i32, dur: f64, rng: &mut Rng) -> Vec<f64> {
    let f = hz(m as f64);
    let hold = 0.95 * dur;
    let rel = 0.2;
    let n = samples(hold + rel);
    let mut hp = Biquad::highpass(1800.0, SR);
    let mut lp = Biquad::lowpass(6000.0, SR);
    let mut out = vec![0.0; n];
    let mut phase = 0.0f64;
    let att = 0.05;
    for (i, x) in out.iter_mut().enumerate() {
        let t = i as f64 / SR;
        let fade = ((t - 0.18) / 0.30).clamp(0.0, 1.0);
        let vib = 1.0 + 0.0045 * fade * libm::sin(2.0 * PI * 5.2 * t);
        let (s1, c1) = libm::sincos(phase);
        let s2 = 2.0 * s1 * c1;
        let c2 = 1.0 - 2.0 * s1 * s1;
        let s3 = s2 * c1 + c2 * s1;
        let s4 = 2.0 * s2 * c2;
        let tone = s1 + 0.16 * s2 + 0.09 * s3 + 0.03 * s4;
        let noise = lp.tick(hp.tick(rng.gauss()));
        let breath = noise * (0.05 + 0.25 * 0.5 * libm::exp(-t / 0.03));
        let env = if t < att {
            t / att
        } else if t < hold {
            1.0
        } else {
            (1.0 - (t - hold) / rel).max(0.0)
        };
        let env = if hold < att {
            env.min(1.0 - ((t - hold) / rel).max(0.0))
        } else {
            env
        };
        let swell = 1.0 + 0.08 * libm::sin(2.0 * PI * 0.7 * t);
        *x = env * swell * (tone + breath);
        phase += 2.0 * PI * f * vib / SR;
        if phase > 2.0 * PI {
            phase -= 2.0 * PI;
        }
    }
    out
}

/// Additive pluck: 13 slightly sharp partials (stopping above 12 kHz),
/// amplitude |sin(pi k 0.18)| / k^1.1, decaying at 1.6 + 0.9k per second,
/// with a 50 ms (or given) tail.
pub fn guitar_tone(m: i32, n: usize, tail: usize) -> Vec<f64> {
    let f = hz(m as f64);
    let mut out = vec![0.0; n];
    let mut parts = Vec::new();
    for k in 1..=13 {
        let kf = k as f64;
        let fk = f * kf * (1.0 + 0.0004 * kf * kf);
        if fk > 12000.0 {
            break;
        }
        parts.push(Partial {
            f: fk,
            amp: libm::fabs(libm::sin(PI * kf * 0.18)) / libm::pow(kf, 1.1),
            decay: 1.6 + 0.9 * kf,
            phase: 0.0,
        });
    }
    add_partials(&mut out, &parts);
    ramp_ends(&mut out, 0, tail);
    out
}

/// Pick noise: 0.15 N(0,1) e^{-t/0.004}.
fn guitar_burst(out: &mut [f64], rng: &mut Rng) {
    for (i, x) in out.iter_mut().take(samples(0.03)).enumerate() {
        let t = i as f64 / SR;
        *x += 0.15 * rng.gauss() * libm::exp(-t / 0.004);
    }
}

/// Marimba-like mallet: sin(f) e^{-5.5t} + 0.25 sin(3.98f) e^{-18t} +
/// 0.08 sin(9.2f) e^{-40t}, 2 ms attack.
pub fn mallet(m: i32, n: usize, tail: usize) -> Vec<f64> {
    let f = hz(m as f64);
    let mut out = vec![0.0; n];
    add_partials(
        &mut out,
        &[
            Partial {
                f,
                amp: 1.0,
                decay: 5.5,
                phase: 0.0,
            },
            Partial {
                f: 3.98 * f,
                amp: 0.25,
                decay: 18.0,
                phase: 0.0,
            },
            Partial {
                f: 9.2 * f,
                amp: 0.08,
                decay: 40.0,
                phase: 0.0,
            },
        ],
    );
    ramp_ends(&mut out, samples(0.002), tail);
    out
}

/// Warm pad tone: 3 detunes (-0.06, 0, +0.07 semitones) x 6 harmonics at
/// 0.5^(k-1) with random phases; 0.6 s attack, 0.8 s release; divided by the
/// three tones of a pad chord.
pub fn pad(m: i32, dur: f64, rng: &mut Rng) -> Vec<f64> {
    let rel = 0.8;
    let n = samples(dur + rel);
    let mut out = vec![0.0; n];
    let mut parts = Vec::with_capacity(18);
    for det in [-0.06, 0.0, 0.07] {
        let f = hz(m as f64 + det);
        for k in 1..=6 {
            parts.push(Partial {
                f: f * k as f64,
                amp: libm::pow(0.5, (k - 1) as f64),
                decay: 0.0,
                phase: 2.0 * PI * rng.uniform(),
            });
        }
    }
    add_partials(&mut out, &parts);
    let att = 0.6;
    for (i, x) in out.iter_mut().enumerate() {
        let t = i as f64 / SR;
        let a = (t / att).min(1.0);
        let r = if t > dur {
            (1.0 - (t - dur) / rel).max(0.0)
        } else {
            1.0
        };
        *x *= a * r / 3.0;
    }
    out
}

/// Gritty bass: saw from harmonics up to 2.2 kHz (1/k, each decaying at
/// 2.5(k-1)/6 per second) at 0.55 plus a sine at the fundamental at 0.8;
/// 8 ms attack, 0.75 + 0.25 e^{-6t}, 30 ms release; tanh(1.8x) * 0.85.
pub fn heavy_bass(m: i32, n_hold: usize) -> Vec<f64> {
    let f = hz(m as f64);
    let rel = samples(0.03);
    let n = n_hold + rel;
    let mut out = vec![0.0; n];
    let mut parts = Vec::new();
    let mut k = 1;
    while f * k as f64 <= 2200.0 {
        let kf = k as f64;
        parts.push(Partial {
            f: f * kf,
            amp: 0.55 / kf,
            decay: 2.5 * (kf - 1.0) / 6.0,
            phase: 0.0,
        });
        k += 1;
    }
    parts.push(Partial {
        f,
        amp: 0.8,
        decay: 0.0,
        phase: 0.0,
    });
    add_partials(&mut out, &parts);
    let att = samples(0.008);
    for (i, x) in out.iter_mut().enumerate() {
        let t = i as f64 / SR;
        let mut env = 0.75 + 0.25 * libm::exp(-6.0 * t);
        if i < att {
            env *= i as f64 / att as f64;
        }
        if i >= n_hold {
            env *= 1.0 - (i - n_hold) as f64 / (rel - 1) as f64;
        }
        *x = libm::tanh(1.8 * env * *x) * 0.85;
    }
    out
}

/// Soft round bass: sin + 0.35 sin2 + 0.12 sin3, 10 ms attack,
/// 0.55 + 0.45 e^{-3t}, 80 ms release; tanh(1.3x) * 0.8.
pub fn soft_bass(m: i32, n_hold: usize) -> Vec<f64> {
    let f = hz(m as f64);
    let rel = samples(0.08);
    let n = n_hold + rel;
    let mut out = vec![0.0; n];
    add_partials(
        &mut out,
        &[
            Partial {
                f,
                amp: 1.0,
                decay: 0.0,
                phase: 0.0,
            },
            Partial {
                f: 2.0 * f,
                amp: 0.35,
                decay: 0.0,
                phase: 0.0,
            },
            Partial {
                f: 3.0 * f,
                amp: 0.12,
                decay: 0.0,
                phase: 0.0,
            },
        ],
    );
    let att = samples(0.01);
    for (i, x) in out.iter_mut().enumerate() {
        let t = i as f64 / SR;
        let mut env = 0.55 + 0.45 * libm::exp(-3.0 * t);
        if i < att {
            env *= i as f64 / att as f64;
        }
        if i >= n_hold {
            env *= 1.0 - (i - n_hold) as f64 / (rel - 1) as f64;
        }
        *x = libm::tanh(1.3 * env * *x) * 0.8;
    }
    out
}

/// Kick: 48 + 110 e^{-t/0.03} Hz, amplitude e^{-t/0.11}, 0.35 s.
pub fn kick_tone() -> Vec<f64> {
    let n = samples(0.35);
    let mut out: Vec<f64> = (0..n)
        .map(|i| {
            let t = i as f64 / SR;
            let ph = 2.0 * PI * (48.0 * t + 110.0 * 0.03 * (1.0 - libm::exp(-t / 0.03)));
            libm::sin(ph) * libm::exp(-t / 0.11)
        })
        .collect();
    ramp_ends(&mut out, 0, samples(0.005));
    out
}

/// Shaker: noise high-passed at 5 kHz, 8 ms attack, e^{-t/0.022}, 90 ms.
pub fn shaker(rng: &mut Rng) -> Vec<f64> {
    let n = samples(0.09);
    let mut hp = Biquad::highpass(5000.0, SR);
    let att = samples(0.008);
    let mut out: Vec<f64> = (0..n)
        .map(|i| {
            let t = i as f64 / SR;
            let a = if i < att { i as f64 / att as f64 } else { 1.0 };
            hp.tick(rng.gauss()) * a * libm::exp(-t / 0.022)
        })
        .collect();
    ramp_ends(&mut out, 0, samples(0.005));
    out
}

/// Rim: noise band-passed 1.5-5 kHz at 0.6 plus a 410 Hz sine at 0.6,
/// e^{-t/0.025}, 120 ms.
pub fn rim(rng: &mut Rng) -> Vec<f64> {
    let n = samples(0.12);
    let mut hp = Biquad::highpass(1500.0, SR);
    let mut lp = Biquad::lowpass(5000.0, SR);
    let mut out: Vec<f64> = (0..n)
        .map(|i| {
            let t = i as f64 / SR;
            let noise = lp.tick(hp.tick(rng.gauss()));
            (0.6 * noise + 0.6 * libm::sin(2.0 * PI * 410.0 * t)) * libm::exp(-t / 0.025)
        })
        .collect();
    ramp_ends(&mut out, samples(0.0005), samples(0.005));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(x: &[f64]) -> f64 {
        x.iter().fold(0.0, |a, v| a.max(v.abs()))
    }

    #[test]
    fn recipes_are_finite_and_bounded() {
        let mut r = Rng::new(1);
        for buf in [
            flute(81, 1.0, &mut r),
            guitar_tone(74, samples(1.45), samples(0.05)),
            mallet(81, samples(1.6), samples(0.03)),
            pad(66, 2.5, &mut r),
            heavy_bass(38, samples(0.3)),
            soft_bass(43, samples(0.5)),
            kick_tone(),
            shaker(&mut r),
            rim(&mut r),
            chip(76, samples(0.4)),
            synth_pluck_tone(64, samples(0.6), samples(0.04)),
            blip(88),
            clap(&mut r),
            riser(1.0, false, &mut r),
            riser(0.8, true, &mut r),
            glide(&mallet(81, samples(1.6), samples(0.03)), 40.0, 0.06),
        ] {
            assert!(buf.iter().all(|x| x.is_finite()));
            let p = peak(&buf);
            assert!(p > 0.01 && p < 3.0, "peak {p}");
            // Every voice ends at silence (no clicks at the cut).
            assert!(
                buf.last().unwrap().abs() < 1e-3,
                "{} {}",
                buf.len(),
                buf.last().unwrap()
            );
        }
    }

    #[test]
    fn heavy_bass_has_no_sub_octave() {
        // The fundamental of D2 is 73.4 Hz: nothing should sit an octave below.
        let buf = heavy_bass(38, samples(1.0));
        let n = 32768;
        let fft = crate::dsp::Fft::new(n);
        let mut re: Vec<f64> = buf[..n.min(buf.len())].to_vec();
        re.resize(n, 0.0);
        let mut im = vec![0.0; n];
        fft.forward(&mut re, &mut im);
        let mag = |f: f64| {
            let k = (f * n as f64 / SR).round() as usize;
            (k - 2..=k + 2)
                .map(|i| re[i].hypot(im[i]))
                .fold(0.0, f64::max)
        };
        assert!(mag(36.7) < 0.05 * mag(73.4));
    }
}
