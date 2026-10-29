//! The render pipeline and its cue sheet.

use crate::analysis::{band_levels, BandLevel};
use crate::arrange::{arrange, Arrangement, TransitionInfo};
use crate::dsp::{db_to_gain, gain_to_db, StereoConvolver, SR};
use crate::harmony::Chord;
use crate::loudness::{gated, integrated, k_power};
use crate::mix::{master_eq, mix_voices, reverb_ir, saturate, Stereo, WET};
use crate::plan::SectionSpec;
use crate::resolve::{
    resolve_sections, summary_lines, voices, Bus, RoleUse, SectionResolution, Voice,
};
use crate::synth::Synth;
use crate::timing::{search_tempo, snap};
use crate::types::{Feel, Instrument, VideoType};
use serde::Serialize;

pub const TARGET_LUFS: f64 = -14.0;
pub const PEAK_CEILING_DBFS: f64 = -1.0;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Master settings to reuse from an earlier render (`--gain-from`).
#[derive(Clone, Copy, Debug)]
pub struct MasterRef {
    pub gain: f64,
    pub saturation_peak: f64,
}

pub struct Request<'a> {
    pub specs: &'a [SectionSpec],
    pub feel: Feel,
    pub seed: u64,
    /// Whole-score allow-list.
    pub instruments: Vec<Instrument>,
    pub stems: bool,
    pub gain_from: Option<MasterRef>,
}

pub struct Rendered {
    pub full: Stereo,
    pub no_melody: Stereo,
    /// (bus id, audio), one per bus, in a fixed order.
    pub stems: Vec<(String, Stereo)>,
    pub cue: Cue,
    pub arrangement: Arrangement,
    pub resolution: Vec<SectionResolution>,
    /// Lines for stderr: substitutions, dropped roles and warnings.
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Cue {
    pub tool: &'static str,
    pub version: &'static str,
    pub seed: u64,
    pub feel: Feel,
    pub bpm: f64,
    pub key: &'static str,
    pub sample_rate: u32,
    pub duration_s: f64,
    pub score_instruments: Vec<Instrument>,
    pub master: MasterInfo,
    pub sections: Vec<CueSection>,
    pub transitions: Vec<TransitionInfo>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MasterInfo {
    pub integrated_lufs: Option<f64>,
    pub no_melody_integrated_lufs: Option<f64>,
    pub sample_peak_dbfs: f64,
    pub no_melody_sample_peak_dbfs: f64,
    /// Linear gain applied to both full.wav and no-melody.wav.
    pub gain: f64,
    pub gain_db: f64,
    /// Peak the saturation curve was scaled to.
    pub saturation_peak: f64,
    /// True when the -1 dBFS ceiling, not the loudness target, set the gain.
    pub peak_limited: bool,
    /// True when gain and saturation came from `--gain-from`.
    pub gain_reused: bool,
    pub reverb_wet: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CueSection {
    pub index: usize,
    #[serde(rename = "type")]
    pub kind: VideoType,
    pub energy: &'static str,
    pub requested_start_s: f64,
    pub requested_end_s: f64,
    pub start_s: f64,
    pub end_s: f64,
    pub start_offset_s: f64,
    pub end_offset_s: f64,
    pub start_beat: u32,
    pub beats: u32,
    pub bars: usize,
    pub chord_loop: String,
    pub chords: Vec<Chord>,
    pub instrument_list: &'static str,
    pub allowed_instruments: Vec<Instrument>,
    pub instruments_used: Vec<Instrument>,
    pub roles: Vec<RoleUse>,
    pub substitutions: Vec<String>,
    pub dropped_roles: Vec<String>,
    pub lufs: Option<f64>,
    pub relative_lu: Option<f64>,
    pub target_relative_lu: f64,
    pub trim_db: f64,
    pub bands: Vec<BandLevel>,
}

/// Loudness target relative to the loudest section, by energy.
pub fn target_relative_lu(energy: f64) -> f64 {
    const T: [(f64, f64); 5] = [
        (1.0, -8.0),
        (2.0, -5.0),
        (3.0, -3.0),
        (4.0, -1.5),
        (5.0, 0.0),
    ];
    let e = energy.clamp(1.0, 5.0);
    for w in T.windows(2) {
        let ((e0, l0), (e1, l1)) = (w[0], w[1]);
        if e <= e1 {
            return l0 + (l1 - l0) * (e - e0) / (e1 - e0);
        }
    }
    0.0
}

/// One section's audio (its own notes, including transitions it owns) after
/// reverb, fades and EQ, placed at sample `start`. Everything up to the
/// saturation stage is linear, so the full mix is the gain-weighted sum of
/// the parts, and per-section trims can be calibrated without re-rendering.
pub struct Part {
    pub start: usize,
    pub audio: Stereo,
}

/// Extra length after a part's last note: reverb tail plus filter settling.
const EQ_TAIL: f64 = 0.5;

pub struct Chain<'a> {
    pub reverb: &'a StereoConvolver,
    pub ir_len: usize,
    pub len: usize,
    pub fade_out: f64,
}

pub fn render_parts(
    synth: &mut Synth,
    vs: &[Voice],
    pick: impl Fn(&Voice) -> bool,
    sections: usize,
    chain: &Chain,
) -> Vec<Option<Part>> {
    (0..sections)
        .map(|s| {
            let mine: Vec<Voice> = vs
                .iter()
                .filter(|v| v.event.section == s && pick(v))
                .filter(|v| v.amp != 0.0 && ((v.event.time * SR).round() as usize) < chain.len)
                .cloned()
                .collect();
            let first = mine
                .iter()
                .map(|v| (v.event.time * SR).round() as usize)
                .min()?;
            let mut shifted = mine;
            for v in shifted.iter_mut() {
                v.event.time -= first as f64 / SR;
            }
            // Render once to find the extent, then mix into a local buffer.
            let room = chain.len - first;
            let bed = mix_voices(synth, shifted.iter(), room);
            let last = bed
                .dry
                .l
                .iter()
                .zip(&bed.dry.r)
                .rposition(|(l, r)| *l != 0.0 || *r != 0.0)
                .unwrap_or(0);
            let end = (last + 1 + chain.ir_len + (EQ_TAIL * SR) as usize).min(room);
            let mut send = bed.send;
            send.truncate(end);
            let mut audio = Stereo {
                l: bed.dry.l[..end].to_vec(),
                r: bed.dry.r[..end].to_vec(),
            };
            chain
                .reverb
                .process_into(&send, WET, &mut audio.l, &mut audio.r);
            apply_fades_at(&mut audio, first, chain.len, chain.fade_out);
            master_eq(&mut audio);
            Some(Part {
                start: first,
                audio,
            })
        })
        .collect()
}

/// Master fades for a buffer that starts at absolute sample `start`.
fn apply_fades_at(st: &mut Stereo, start: usize, total: usize, fade_out: f64) {
    let fi = ((0.4 * SR) as usize).min(total);
    let fo = ((fade_out * SR) as usize).min(total);
    for k in 0..st.len() {
        let i = start + k;
        let mut g = 1.0;
        if i < fi {
            g *= i as f64 / fi as f64;
        }
        if i + fo >= total {
            g *= (total - 1 - i) as f64 / fo as f64;
        }
        if g != 1.0 {
            st.l[k] *= g;
            st.r[k] *= g;
        }
    }
}

/// Adds `gain[s] * part[s]` for every section, in section order.
pub fn sum_parts(dst: &mut Stereo, parts: &[Option<Part>], gains: &[f64]) {
    for (s, p) in parts.iter().enumerate() {
        let Some(p) = p else { continue };
        let g = gains[s];
        let n = p.audio.len().min(dst.len() - p.start);
        for k in 0..n {
            dst.l[p.start + k] += g * p.audio.l[k];
            dst.r[p.start + k] += g * p.audio.r[k];
        }
    }
}

fn section_window(sp: &crate::arrange::SectionPlan, duration: f64) -> (usize, usize) {
    (
        (sp.start * SR).round() as usize,
        (sp.end.min(duration) * SR).round() as usize,
    )
}

/// Per-section level trims (dB) that bring each section's loudness, relative
/// to the others, to its energy target.
///
/// Calibrated on the unrestricted arrangement (every instrument allowed), so
/// trims depend on the plan, feel and seed but never on the allow-lists: a
/// restricted section keeps the trim of its full version and only comes as
/// close to its target as its instruments allow.
pub fn calibrate_trims(arr: &Arrangement, parts: &[&[Option<Part>]], len: usize) -> Vec<f64> {
    let n = arr.sections.len();
    let targets: Vec<f64> = arr
        .sections
        .iter()
        .map(|s| target_relative_lu(s.kind.energy()))
        .collect();
    let mut trims = vec![0.0; n];
    for _ in 0..8 {
        let gains: Vec<f64> = trims.iter().map(|t| db_to_gain(*t)).collect();
        let mut mix = Stereo::silent(len);
        for p in parts {
            sum_parts(&mut mix, p, &gains);
        }
        let kp = k_power(&mix.channels(), SR);
        let measured: Vec<Option<f64>> = arr
            .sections
            .iter()
            .map(|sp| {
                let (a, b) = section_window(sp, arr.duration);
                gated(&kp, SR, a, b)
            })
            .collect();
        let diffs: Vec<f64> = measured
            .iter()
            .zip(&targets)
            .filter_map(|(m, t)| m.map(|m| m - t))
            .collect();
        if diffs.is_empty() {
            break;
        }
        let c = diffs.iter().sum::<f64>() / diffs.len() as f64;
        let mut worst = 0.0f64;
        for s in 0..n {
            if let Some(m) = measured[s] {
                let err = targets[s] + c - m;
                worst = worst.max(err.abs());
                trims[s] = (trims[s] + err).clamp(-10.0, 6.0);
            }
        }
        if worst < 0.05 {
            break;
        }
    }
    trims
}

pub fn render(req: &Request) -> Result<Rendered, Vec<String>> {
    let specs = req.specs;
    let ends: Vec<f64> = specs.iter().map(|s| s.end).collect();
    let bpm = search_tempo(&ends, req.feel).map_err(|e| {
        e.0.iter()
            .map(|&i| {
                format!(
                    "section {} ({}) is shorter than one beat",
                    i + 1,
                    specs[i].kind
                )
            })
            .collect::<Vec<_>>()
    })?;
    let timing = snap(&ends, bpm);
    let mut warnings = Vec::new();
    for (i, s) in timing.sections.iter().enumerate() {
        if i + 1 < timing.sections.len() && s.end_offset.abs() > 0.25 {
            warnings.push(format!(
                "boundary after section {} moved by {:+.3} s to land on a beat",
                i + 1,
                s.end_offset
            ));
        }
    }
    let kinds: Vec<VideoType> = specs.iter().map(|s| s.kind).collect();
    let arr = arrange(&kinds, &timing, req.feel, req.seed);
    let n = arr.sections.len();
    let mut synth = Synth::new(req.seed);
    let len = (arr.duration * SR).round() as usize;
    let fade_out = {
        let last = arr.sections.last().expect("sections");
        3.0f64.min(arr.duration - last.start)
    };
    let (irl, irr) = reverb_ir(req.seed);
    let reverb = StereoConvolver::new(&irl, &irr);
    let chain = Chain {
        reverb: &reverb,
        ir_len: irl.len(),
        len,
        fade_out,
    };

    // Unrestricted render: calibrates the trims, and is the final audio
    // when the allow-lists change nothing.
    let all_res = resolve_sections(&arr, &Instrument::ALL, &vec![None; n]);
    let all_vs = voices(&arr.events, &all_res);
    let lead_u = render_parts(&mut synth, &all_vs, |v| v.bus == Bus::Lead, n, &chain);
    let rest_u = render_parts(&mut synth, &all_vs, |v| v.bus != Bus::Lead, n, &chain);
    let trims = calibrate_trims(&arr, &[&rest_u, &lead_u], len);
    let gains: Vec<f64> = trims.iter().map(|t| db_to_gain(*t)).collect();

    let own: Vec<Option<Vec<Instrument>>> = specs.iter().map(|s| s.instruments.clone()).collect();
    let res = resolve_sections(&arr, &req.instruments, &own);
    let vs = voices(&arr.events, &res);
    let mut notes = summary_lines(&arr, &res);
    for r in &res {
        warnings.extend(r.warnings.iter().cloned());
    }
    let unchanged = res.iter().zip(&all_res).all(|(a, b)| {
        a.roles
            .iter()
            .map(|r| r.instrument)
            .eq(b.roles.iter().map(|r| r.instrument))
    });
    let (lead, rest) = if unchanged {
        (lead_u, rest_u)
    } else {
        drop((lead_u, rest_u));
        (
            render_parts(&mut synth, &vs, |v| v.bus == Bus::Lead, n, &chain),
            render_parts(&mut synth, &vs, |v| v.bus != Bus::Lead, n, &chain),
        )
    };
    let mut no_melody = Stereo::silent(len);
    sum_parts(&mut no_melody, &rest, &gains);
    let mut full = no_melody.clone();
    sum_parts(&mut full, &lead, &gains);
    drop((lead, rest));

    let sat_peak = req
        .gain_from
        .map_or_else(|| full.peak(), |m| m.saturation_peak);
    saturate(&mut full, sat_peak);
    saturate(&mut no_melody, sat_peak);

    let ceiling = db_to_gain(PEAK_CEILING_DBFS);
    let (gain, peak_limited) = match req.gain_from {
        Some(m) => (m.gain, false),
        None => {
            let pre = integrated(&full.channels(), SR);
            let want = pre.map_or(1.0, |l| db_to_gain(TARGET_LUFS - l));
            let peak = full.peak().max(no_melody.peak());
            if peak > 0.0 && want * peak > ceiling {
                (ceiling / peak, true)
            } else {
                (want, false)
            }
        }
    };
    full.scale(gain);
    no_melody.scale(gain);
    let full_lufs = integrated(&full.channels(), SR);
    let nm_lufs = integrated(&no_melody.channels(), SR);
    if peak_limited {
        warnings.push(format!(
            "peak ceiling limited the gain: integrated loudness is {:.2} LUFS instead of {TARGET_LUFS}",
            full_lufs.unwrap_or(f64::NEG_INFINITY)
        ));
    }
    if req.gain_from.is_some() && full.peak().max(no_melody.peak()) > ceiling {
        warnings.push("reused gain puts the peak above -1 dBFS".into());
    }

    // Stems: each bus through the same linear chain and gain (no saturation,
    // which is not linear), so they line up with the full mix.
    let mut stems = Vec::new();
    if req.stems {
        for bus in Bus::all() {
            let parts = render_parts(&mut synth, &vs, |v| v.bus == bus, n, &chain);
            let mut st = Stereo::silent(len);
            let g: Vec<f64> = gains.iter().map(|x| x * gain).collect();
            sum_parts(&mut st, &parts, &g);
            stems.push((bus.id(), st));
        }
    }

    // Per-section loudness and bands on the final full mix.
    let kp = k_power(&full.channels(), SR);
    let mut lufs: Vec<Option<f64>> = Vec::new();
    let mut bands = Vec::new();
    for sp in &arr.sections {
        let (a, b) = section_window(sp, arr.duration);
        lufs.push(gated(&kp, SR, a, b));
        bands.push(band_levels(&full.l, &full.r, a, b));
    }
    let loudest = lufs
        .iter()
        .flatten()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);

    let sections = arr
        .sections
        .iter()
        .zip(&res)
        .zip(lufs.iter().zip(bands))
        .map(|((sp, r), (l, bands))| {
            let sn = &timing.sections[sp.index];
            let spec = &specs[sp.index];
            CueSection {
                index: sp.index,
                kind: sp.kind,
                energy: sp.kind.energy_label(),
                requested_start_s: spec.start,
                requested_end_s: spec.end,
                start_s: sp.start,
                end_s: sp.end,
                start_offset_s: sn.start_offset,
                end_offset_s: sn.end_offset,
                start_beat: sn.start_beat,
                beats: sp.beats,
                bars: sp.bars,
                chord_loop: sp.loop_name.clone(),
                chords: sp.chords.clone(),
                instrument_list: r.list_source,
                allowed_instruments: r.allowed.clone(),
                instruments_used: r.instruments_used.clone(),
                roles: r.roles.clone(),
                substitutions: r
                    .roles
                    .iter()
                    .filter(|u| u.status == "substituted")
                    .map(|u| {
                        format!(
                            "{}: {} -> {}",
                            u.role,
                            u.preferred,
                            u.instrument.map_or("-", |i| i.id())
                        )
                    })
                    .collect(),
                dropped_roles: r
                    .roles
                    .iter()
                    .filter(|u| u.status == "dropped")
                    .map(|u| u.role.id().to_string())
                    .collect(),
                lufs: *l,
                relative_lu: l.map(|v| v - loudest),
                target_relative_lu: target_relative_lu(sp.kind.energy()),
                trim_db: trims[sp.index],
                bands,
            }
        })
        .collect();

    notes.extend(warnings.iter().map(|w| format!("warning: {w}")));
    let cue = Cue {
        tool: "cozy-score",
        version: VERSION,
        seed: req.seed,
        feel: req.feel,
        bpm,
        key: "D major",
        sample_rate: SR as u32,
        duration_s: arr.duration,
        score_instruments: req.instruments.clone(),
        master: MasterInfo {
            integrated_lufs: full_lufs,
            no_melody_integrated_lufs: nm_lufs,
            sample_peak_dbfs: gain_to_db(full.peak().max(1e-12)),
            no_melody_sample_peak_dbfs: gain_to_db(no_melody.peak().max(1e-12)),
            gain,
            gain_db: gain_to_db(gain),
            saturation_peak: sat_peak,
            peak_limited,
            gain_reused: req.gain_from.is_some(),
            reverb_wet: WET,
        },
        sections,
        transitions: arr.transitions.clone(),
        warnings,
    };
    Ok(Rendered {
        full,
        no_melody,
        stems,
        cue,
        arrangement: arr,
        resolution: res,
        notes,
    })
}

/// 16-bit PCM stereo WAV bytes.
pub fn wav_bytes(st: &Stereo) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: SR as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cur = std::io::Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cur, spec).expect("wav header");
        for (l, r) in st.l.iter().zip(&st.r) {
            for x in [l, r] {
                let s = (x * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
                w.write_sample(s).expect("wav sample");
            }
        }
        w.finalize().expect("wav finalize");
    }
    cur.into_inner()
}
