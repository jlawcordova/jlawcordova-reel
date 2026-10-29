//! Deterministic DSP building blocks: biquads, a radix-2 FFT with libm
//! twiddles, FFT convolution, and a recurrence oscillator.
//!
//! The FFT lives in the crate rather than coming from a dependency so its
//! twiddle factors are computed with `libm` and its operation order never
//! depends on runtime CPU feature detection.

use core::f64::consts::PI;

pub const SR: f64 = 44100.0;

/// Direct-form-I biquad.
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    pub fn new(b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) -> Self {
        Biquad {
            b0,
            b1,
            b2,
            a1,
            a2,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn rbj(b: [f64; 3], a: [f64; 3]) -> Self {
        Biquad::new(
            b[0] / a[0],
            b[1] / a[0],
            b[2] / a[0],
            a[1] / a[0],
            a[2] / a[0],
        )
    }

    /// 2nd-order Butterworth low-pass: |H| = 1/sqrt(1+(f/fc)^4).
    pub fn lowpass(fc: f64, fs: f64) -> Self {
        let w0 = 2.0 * PI * fc / fs;
        let (s, c) = (libm::sin(w0), libm::cos(w0));
        let alpha = s / (2.0 * core::f64::consts::FRAC_1_SQRT_2);
        Self::rbj(
            [(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0],
            [1.0 + alpha, -2.0 * c, 1.0 - alpha],
        )
    }

    /// 2nd-order Butterworth high-pass, the mirror image of `lowpass`.
    pub fn highpass(fc: f64, fs: f64) -> Self {
        let w0 = 2.0 * PI * fc / fs;
        let (s, c) = (libm::sin(w0), libm::cos(w0));
        let alpha = s / (2.0 * core::f64::consts::FRAC_1_SQRT_2);
        Self::rbj(
            [(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0],
            [1.0 + alpha, -2.0 * c, 1.0 - alpha],
        )
    }

    /// RBJ high shelf (shelf slope 1).
    pub fn high_shelf(f0: f64, gain_db: f64, fs: f64) -> Self {
        let a = libm::pow(10.0, gain_db / 40.0);
        let w0 = 2.0 * PI * f0 / fs;
        let (s, c) = (libm::sin(w0), libm::cos(w0));
        let alpha = s / 2.0 * core::f64::consts::SQRT_2;
        let sa = 2.0 * libm::sqrt(a) * alpha;
        Self::rbj(
            [
                a * ((a + 1.0) + (a - 1.0) * c + sa),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                a * ((a + 1.0) + (a - 1.0) * c - sa),
            ],
            [
                (a + 1.0) - (a - 1.0) * c + sa,
                2.0 * ((a - 1.0) - (a + 1.0) * c),
                (a + 1.0) - (a - 1.0) * c - sa,
            ],
        )
    }

    #[inline]
    pub fn tick(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    pub fn process(&mut self, buf: &mut [f64]) {
        for x in buf.iter_mut() {
            *x = self.tick(*x);
        }
    }

    /// Magnitude response at `f` Hz.
    pub fn magnitude(&self, f: f64, fs: f64) -> f64 {
        let w = 2.0 * PI * f / fs;
        let (c1, s1) = (libm::cos(w), libm::sin(w));
        let (c2, s2) = (libm::cos(2.0 * w), libm::sin(2.0 * w));
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        libm::sqrt((nr * nr + ni * ni) / (dr * dr + di * di))
    }
}

/// In-place iterative radix-2 complex FFT.
pub struct Fft {
    n: usize,
    rev: Vec<u32>,
    /// Twiddles laid out stage by stage (size 2, 4, ..., n), so each stage
    /// reads them sequentially.
    cos: Vec<f64>,
    sin: Vec<f64>,
}

impl Fft {
    pub fn new(n: usize) -> Self {
        assert!(n.is_power_of_two() && n >= 2);
        let bits = n.trailing_zeros();
        let rev = (0..n as u32)
            .map(|i| i.reverse_bits() >> (32 - bits))
            .collect();
        let mut cos = Vec::with_capacity(n);
        let mut sin = Vec::with_capacity(n);
        let mut size = 2;
        while size <= n {
            for k in 0..size / 2 {
                let a = -2.0 * PI * k as f64 / size as f64;
                cos.push(libm::cos(a));
                sin.push(libm::sin(a));
            }
            size *= 2;
        }
        Fft { n, rev, cos, sin }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Forward transform (e^{-i...}).
    pub fn forward(&self, re: &mut [f64], im: &mut [f64]) {
        let n = self.n;
        assert!(re.len() == n && im.len() == n);
        for i in 0..n {
            let j = self.rev[i] as usize;
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut size = 2;
        let mut tw = 0;
        while size <= n {
            let half = size / 2;
            let (wc, ws) = (&self.cos[tw..tw + half], &self.sin[tw..tw + half]);
            let mut start = 0;
            while start < n {
                let (lo_r, hi_r) = re[start..start + size].split_at_mut(half);
                let (lo_i, hi_i) = im[start..start + size].split_at_mut(half);
                for k in 0..half {
                    let (wr, wi) = (wc[k], ws[k]);
                    let tr = hi_r[k] * wr - hi_i[k] * wi;
                    let ti = hi_r[k] * wi + hi_i[k] * wr;
                    hi_r[k] = lo_r[k] - tr;
                    hi_i[k] = lo_i[k] - ti;
                    lo_r[k] += tr;
                    lo_i[k] += ti;
                }
                start += size;
            }
            tw += half;
            size *= 2;
        }
    }

    /// Inverse transform, scaled by 1/n.
    pub fn inverse(&self, re: &mut [f64], im: &mut [f64]) {
        for x in im.iter_mut() {
            *x = -*x;
        }
        self.forward(re, im);
        let s = 1.0 / self.n as f64;
        for x in re.iter_mut() {
            *x *= s;
        }
        for x in im.iter_mut() {
            *x = -*x * s;
        }
    }
}

/// Convolves mono `x` with a stereo impulse response (overlap-add). Both IR
/// channels ride in one complex FFT: Y = X * FFT(hl + i*hr), whose inverse
/// carries the left output in the real part and the right in the imaginary.
pub struct StereoConvolver {
    fft: Fft,
    block: usize,
    ir_len: usize,
    hr: Vec<f64>,
    hi: Vec<f64>,
}

impl StereoConvolver {
    pub fn new(ir_l: &[f64], ir_r: &[f64]) -> Self {
        assert_eq!(ir_l.len(), ir_r.len());
        let ir_len = ir_l.len();
        let n = (2 * ir_len).next_power_of_two().max(1024);
        let fft = Fft::new(n);
        let mut hr = vec![0.0; n];
        let mut hi = vec![0.0; n];
        hr[..ir_len].copy_from_slice(ir_l);
        hi[..ir_len].copy_from_slice(ir_r);
        fft.forward(&mut hr, &mut hi);
        StereoConvolver {
            block: n - ir_len + 1,
            fft,
            ir_len,
            hr,
            hi,
        }
    }

    /// Adds `gain * (x * ir)` into `out_l` / `out_r`, truncated to their length.
    pub fn process_into(&self, x: &[f64], gain: f64, out_l: &mut [f64], out_r: &mut [f64]) {
        let n = self.fft.len();
        let mut re = vec![0.0; n];
        let mut im = vec![0.0; n];
        let mut start = 0;
        while start < x.len() {
            let end = (start + self.block).min(x.len());
            if x[start..end].iter().all(|v| *v == 0.0) {
                start = end;
                continue;
            }
            re.iter_mut().for_each(|v| *v = 0.0);
            im.iter_mut().for_each(|v| *v = 0.0);
            re[..end - start].copy_from_slice(&x[start..end]);
            self.fft.forward(&mut re, &mut im);
            for k in 0..n {
                let (a, b) = (re[k], im[k]);
                let (c, d) = (self.hr[k], self.hi[k]);
                re[k] = a * c - b * d;
                im[k] = a * d + b * c;
            }
            self.fft.inverse(&mut re, &mut im);
            let out_len = (end - start + self.ir_len - 1).min(out_l.len().saturating_sub(start));
            for k in 0..out_len {
                out_l[start + k] += gain * re[k];
                out_r[start + k] += gain * im[k];
            }
            start = end;
        }
    }
}

/// One sinusoidal partial: amp * e^{-decay*t} * sin(2*pi*f*t + phase).
#[derive(Clone, Copy, Debug)]
pub struct Partial {
    pub f: f64,
    pub amp: f64,
    pub decay: f64,
    pub phase: f64,
}

/// Accumulates many partials, four at a time so their recurrences run in
/// parallel. Each group's four values are summed as (p0 + p1) + (p2 + p3)
/// before being added to the buffer.
pub fn add_partials(buf: &mut [f64], parts: &[Partial]) {
    let live: Vec<Partial> = parts
        .iter()
        .copied()
        .filter(|p| p.f > 0.0 && p.f < SR / 2.0 && p.amp != 0.0)
        .collect();
    for group in live.chunks(4) {
        // Unused slots sit at z = 1 with zero amplitude.
        let mut zr = [1.0; 4];
        let mut zi = [0.0; 4];
        let mut wr = [1.0; 4];
        let mut wi = [0.0; 4];
        let mut env = [0.0; 4];
        let mut d = [1.0; 4];
        let mut floor = 0.0f64;
        let mut decaying = true;
        for (j, p) in group.iter().enumerate() {
            let w = 2.0 * PI * p.f / SR;
            wr[j] = libm::cos(w);
            wi[j] = libm::sin(w);
            zr[j] = libm::cos(p.phase);
            zi[j] = libm::sin(p.phase);
            env[j] = p.amp;
            d[j] = libm::exp(-p.decay / SR);
            floor = floor.max(p.amp.abs() * 1e-6);
            decaying &= p.decay > 0.0;
        }
        for (i, x) in buf.iter_mut().enumerate() {
            let s = (env[0] * zi[0] + env[1] * zi[1]) + (env[2] * zi[2] + env[3] * zi[3]);
            *x += s;
            for j in 0..4 {
                let nr = zr[j] * wr[j] - zi[j] * wi[j];
                zi[j] = zr[j] * wi[j] + zi[j] * wr[j];
                zr[j] = nr;
                env[j] *= d[j];
            }
            if i % 4096 == 4095 {
                for j in 0..4 {
                    let m = libm::sqrt(zr[j] * zr[j] + zi[j] * zi[j]);
                    zr[j] /= m;
                    zi[j] /= m;
                }
                if decaying && env.iter().all(|e| e.abs() < floor) {
                    break;
                }
            }
        }
    }
}

/// Linear attack ramp and end fade, in samples, applied in place.
pub fn ramp_ends(buf: &mut [f64], attack: usize, tail: usize) {
    let n = buf.len();
    for (i, x) in buf.iter_mut().take(attack.min(n)).enumerate() {
        *x *= i as f64 / attack as f64;
    }
    let tail = tail.min(n);
    for k in 0..tail {
        buf[n - 1 - k] *= k as f64 / tail as f64;
    }
}

pub fn db_to_gain(db: f64) -> f64 {
    libm::pow(10.0, db / 20.0)
}

pub fn gain_to_db(g: f64) -> f64 {
    20.0 * libm::log10(g)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_matches_dft() {
        let n = 64;
        let fft = Fft::new(n);
        let xr: Vec<f64> = (0..n)
            .map(|i| libm::sin(i as f64 * 0.37) + 0.1 * i as f64)
            .collect();
        let xi: Vec<f64> = (0..n).map(|i| libm::cos(i as f64 * 1.1)).collect();
        let (mut re, mut im) = (xr.clone(), xi.clone());
        fft.forward(&mut re, &mut im);
        for k in 0..n {
            let (mut sr, mut si) = (0.0, 0.0);
            for t in 0..n {
                let a = -2.0 * PI * (k * t) as f64 / n as f64;
                sr += xr[t] * libm::cos(a) - xi[t] * libm::sin(a);
                si += xr[t] * libm::sin(a) + xi[t] * libm::cos(a);
            }
            assert!((re[k] - sr).abs() < 1e-9 && (im[k] - si).abs() < 1e-9);
        }
        fft.inverse(&mut re, &mut im);
        for t in 0..n {
            assert!((re[t] - xr[t]).abs() < 1e-12 && (im[t] - xi[t]).abs() < 1e-12);
        }
    }

    #[test]
    fn convolution_matches_direct() {
        let ir_l: Vec<f64> = (0..300).map(|i| libm::exp(-(i as f64) / 50.0)).collect();
        let ir_r: Vec<f64> = (0..300).map(|i| libm::sin(i as f64) * 0.5).collect();
        let x: Vec<f64> = (0..5000)
            .map(|i| libm::sin(i as f64 * 0.05) * ((i % 7) as f64))
            .collect();
        let conv = StereoConvolver::new(&ir_l, &ir_r);
        let mut l = vec![0.0; 5200];
        let mut r = vec![0.0; 5200];
        conv.process_into(&x, 1.0, &mut l, &mut r);
        for n in (0..5200).step_by(37) {
            let (mut dl, mut dr) = (0.0, 0.0);
            for k in 0..300 {
                if n >= k && n - k < x.len() {
                    dl += x[n - k] * ir_l[k];
                    dr += x[n - k] * ir_r[k];
                }
            }
            assert!((l[n] - dl).abs() < 1e-9, "{n}");
            assert!((r[n] - dr).abs() < 1e-9, "{n}");
        }
    }

    #[test]
    fn butterworth_magnitudes() {
        let lp = Biquad::lowpass(1000.0, SR);
        assert!((lp.magnitude(1000.0, SR) - core::f64::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((lp.magnitude(10.0, SR) - 1.0).abs() < 1e-4);
        let hp = Biquad::highpass(35.0, SR);
        assert!((hp.magnitude(35.0, SR) - core::f64::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!(hp.magnitude(10.0, SR) < 0.1);
        let sh = Biquad::high_shelf(5000.0, gain_to_db(1.4), SR);
        assert!((sh.magnitude(18000.0, SR) - 1.4).abs() < 0.03);
        assert!((sh.magnitude(100.0, SR) - 1.0).abs() < 0.01);
    }

    #[test]
    fn partials_match_libm() {
        let parts: Vec<Partial> = (1..=6)
            .map(|k| Partial {
                f: 220.0 * k as f64,
                amp: 1.0 / k as f64,
                decay: k as f64,
                phase: 0.1 * k as f64,
            })
            .collect();
        let mut buf = vec![0.0; 44100];
        add_partials(&mut buf, &parts);
        for i in (0..44100).step_by(997) {
            let t = i as f64 / SR;
            let want: f64 = parts
                .iter()
                .map(|p| p.amp * libm::exp(-p.decay * t) * libm::sin(2.0 * PI * p.f * t + p.phase))
                .sum();
            assert!((buf[i] - want).abs() < 1e-8, "{i}");
        }
    }
}
