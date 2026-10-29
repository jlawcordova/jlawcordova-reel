//! Instrument resolution: binds each role to the first allowed instrument in
//! its fallback list, per section, and turns note events into voices.

use crate::arrange::{Arrangement, NoteEvent};
use crate::types::{Instrument, Role};
use serde::Serialize;
use std::collections::BTreeMap;

/// A mix bus. The lead role gets its own bus (whatever instrument plays it)
/// so `no-melody.wav` can leave it out; everything else goes to the bus of
/// the instrument playing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum Bus {
    Lead,
    Inst(Instrument),
}

impl Bus {
    pub fn all() -> Vec<Bus> {
        let mut v = vec![Bus::Lead];
        v.extend(Instrument::ALL.iter().map(|&i| Bus::Inst(i)));
        v
    }

    pub fn id(self) -> String {
        match self {
            Bus::Lead => "lead".into(),
            Bus::Inst(i) => i.id().into(),
        }
    }
}

/// Preferred instrument first, then the fallbacks in order.
pub fn candidates(role: Role, preferred_bass: Instrument) -> Vec<Instrument> {
    use Instrument::*;
    match role {
        Role::Lead => vec![Flute, Mallet, Guitar],
        Role::Motif => vec![Mallet, Flute, Guitar],
        Role::ChordHits => vec![Mallet, Guitar],
        Role::Picking => vec![Guitar, Mallet],
        Role::Pad => vec![Pad],
        Role::Bass => {
            if preferred_bass == SoftBass {
                vec![SoftBass, HeavyBass]
            } else {
                vec![HeavyBass, SoftBass]
            }
        }
        Role::Kick => vec![Kick],
        Role::Shaker => vec![Shaker],
        Role::Rim => vec![Rim],
    }
}

/// Nominal level of `inst` playing `role`; note gains are relative to it.
pub fn nominal(role: Role, inst: Instrument) -> f64 {
    use Instrument::*;
    match (role, inst) {
        (Role::Lead, Flute) => 0.22,
        (Role::Lead, Mallet) => 0.26,
        (Role::Lead, Guitar) => 0.16,
        (Role::Motif, Mallet) => 0.16,
        (Role::Motif, Flute) => 0.16,
        (Role::Motif, Guitar) => 0.14,
        (Role::ChordHits, Mallet) => 0.07,
        (Role::ChordHits, Guitar) => 0.09,
        (Role::Picking, Guitar) => 0.20,
        // Mallet picking: softer gain.
        (Role::Picking, Mallet) => 0.12,
        (Role::Pad, Pad) => 0.13,
        (Role::Bass, HeavyBass) => 0.5,
        (Role::Bass, SoftBass) => 0.36,
        (Role::Kick, Kick) => 0.5,
        (Role::Shaker, Shaker) => 0.18,
        (Role::Rim, Rim) => 0.17,
        _ => 0.0,
    }
}

/// Pitch change when `inst` plays `role`.
pub fn transpose(role: Role, inst: Instrument) -> i32 {
    match (role, inst) {
        // Guitar takes the lead as single notes, one octave up.
        (Role::Lead, Instrument::Guitar) => 12,
        _ => 0,
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RoleUse {
    pub role: Role,
    /// None when the role was dropped.
    pub instrument: Option<Instrument>,
    pub preferred: Instrument,
    /// "preferred", "substituted" or "dropped".
    pub status: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct SectionResolution {
    pub section: usize,
    /// "score" (whole-score list) or "section" (the section's own list).
    pub list_source: &'static str,
    pub allowed: Vec<Instrument>,
    /// Roles this section's notes (including transitions it owns) use.
    pub roles: Vec<RoleUse>,
    pub instruments_used: Vec<Instrument>,
    pub warnings: Vec<String>,
}

impl SectionResolution {
    pub fn binding(&self, role: Role) -> Option<Instrument> {
        self.roles
            .iter()
            .find(|r| r.role == role)
            .and_then(|r| r.instrument)
    }
}

/// A note bound to an instrument and bus.
#[derive(Clone, Debug)]
pub struct Voice {
    pub event: NoteEvent,
    pub instrument: Instrument,
    pub bus: Bus,
    pub midi: i32,
    /// Absolute amplitude.
    pub amp: f64,
}

/// Resolves every section's roles. `score` is the whole-score allow-list and
/// `own[i]` section i's own list, which replaces it when present.
pub fn resolve_sections(
    arr: &Arrangement,
    score: &[Instrument],
    own: &[Option<Vec<Instrument>>],
) -> Vec<SectionResolution> {
    let mut used: BTreeMap<usize, Vec<Role>> = BTreeMap::new();
    for e in &arr.events {
        let v = used.entry(e.section).or_default();
        if !v.contains(&e.role) {
            v.push(e.role);
        }
    }
    arr.sections
        .iter()
        .map(|sp| {
            let (allowed, list_source) = match own.get(sp.index).and_then(|o| o.as_ref()) {
                Some(l) => (l.clone(), "section"),
                None => (score.to_vec(), "score"),
            };
            let mut roles_used = used.get(&sp.index).cloned().unwrap_or_default();
            roles_used.sort();
            let mut roles = Vec::new();
            let mut warnings = Vec::new();
            let mut kept_w = 0.0;
            let mut total_w = 0.0;
            for &role in &roles_used {
                let cands = candidates(role, sp.preferred_bass);
                let inst = cands.iter().copied().find(|c| allowed.contains(c));
                let status = match inst {
                    Some(i) if i == cands[0] => "preferred",
                    Some(_) => "substituted",
                    None => "dropped",
                };
                total_w += role.energy_weight();
                if inst.is_some() {
                    kept_w += role.energy_weight();
                }
                roles.push(RoleUse {
                    role,
                    instrument: inst,
                    preferred: cands[0],
                    status,
                });
            }
            let mut instruments_used: Vec<Instrument> =
                roles.iter().filter_map(|r| r.instrument).collect();
            instruments_used.sort();
            instruments_used.dedup();
            let label = format!("section {} ({})", sp.index + 1, sp.kind);
            if allowed.is_empty() {
                warnings.push(format!("{label}: empty instrument list, the section is silent"));
            } else if total_w > 0.0 {
                let energy = sp.kind.energy();
                let reachable = energy * kept_w / total_w;
                if reachable < energy - 1.0 + 1e-9 {
                    let dropped: Vec<&str> = roles
                        .iter()
                        .filter(|r| r.instrument.is_none())
                        .map(|r| r.role.id())
                        .collect();
                    warnings.push(format!(
                        "{label}: energy {} can't be reached with the allowed instruments (dropped: {}); rendering what is allowed",
                        sp.kind.energy_label(),
                        dropped.join(", ")
                    ));
                }
            }
            SectionResolution {
                section: sp.index,
                list_source,
                allowed,
                roles,
                instruments_used,
                warnings,
            }
        })
        .collect()
}

/// Binds events to instruments. Events of dropped roles disappear.
pub fn voices(events: &[NoteEvent], res: &[SectionResolution]) -> Vec<Voice> {
    events
        .iter()
        .filter_map(|e| {
            let inst = res[e.section].binding(e.role)?;
            let bus = if e.role == Role::Lead {
                Bus::Lead
            } else {
                Bus::Inst(inst)
            };
            Some(Voice {
                event: e.clone(),
                instrument: inst,
                bus,
                midi: e.midi + transpose(e.role, inst),
                amp: e.gain * nominal(e.role, inst),
            })
        })
        .collect()
}

/// One-line stderr summaries of substitutions and dropped roles.
pub fn summary_lines(arr: &Arrangement, res: &[SectionResolution]) -> Vec<String> {
    let mut out = Vec::new();
    for r in res {
        let sp = &arr.sections[r.section];
        for u in &r.roles {
            match (u.status, u.instrument) {
                ("substituted", Some(i)) => out.push(format!(
                    "section {} ({}): {} -> {} (instead of {})",
                    sp.index + 1,
                    sp.kind,
                    u.role,
                    i,
                    u.preferred
                )),
                ("dropped", _) => out.push(format!(
                    "section {} ({}): {} dropped (no allowed instrument)",
                    sp.index + 1,
                    sp.kind,
                    u.role
                )),
                _ => {}
            }
        }
    }
    out
}
