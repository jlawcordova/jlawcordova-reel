//! Video types, instruments, roles and feels: the vocabulary of a plan.

use serde::Serialize;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VideoType {
    ColdOpenHook,
    TitleSting,
    Intro,
    Explain,
    VisualDemo,
    Build,
    Reveal,
    Reflect,
    Sponsor,
    RecapCta,
    OutroEndScreen,
}

impl VideoType {
    pub const ALL: [VideoType; 11] = [
        VideoType::ColdOpenHook,
        VideoType::TitleSting,
        VideoType::Intro,
        VideoType::Explain,
        VideoType::VisualDemo,
        VideoType::Build,
        VideoType::Reveal,
        VideoType::Reflect,
        VideoType::Sponsor,
        VideoType::RecapCta,
        VideoType::OutroEndScreen,
    ];

    pub fn id(self) -> &'static str {
        match self {
            VideoType::ColdOpenHook => "cold-open-hook",
            VideoType::TitleSting => "title-sting",
            VideoType::Intro => "intro",
            VideoType::Explain => "explain",
            VideoType::VisualDemo => "visual-demo",
            VideoType::Build => "build",
            VideoType::Reveal => "reveal",
            VideoType::Reflect => "reflect",
            VideoType::Sponsor => "sponsor",
            VideoType::RecapCta => "recap-cta",
            VideoType::OutroEndScreen => "outro-end-screen",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|t| t.id() == s)
    }

    pub fn description(self) -> &'static str {
        match self {
            VideoType::ColdOpenHook => "First 3-30 s: a question or teaser, with speech",
            VideoType::TitleSting => "2-8 s logo or title card, no speech; ends on a hit",
            VideoType::Intro => "Roadmap or promise, with speech",
            VideoType::Explain => "Core teaching, talking head or voiceover",
            VideoType::VisualDemo => "Graphics/pixel-art showcase, little speech",
            VideoType::Build => "Rising tension toward a reveal",
            VideoType::Reveal => "Payoff, the \"wow\" moment",
            VideoType::Reflect => "Quiet, emotional or philosophical beat",
            VideoType::Sponsor => "Mid-roll sponsor read",
            VideoType::RecapCta => "Takeaways plus subscribe/watch-next, with speech",
            VideoType::OutroEndScreen => "End card, little or no speech",
        }
    }

    /// Energy as the table states it, for display.
    pub fn energy_label(self) -> &'static str {
        match self {
            VideoType::ColdOpenHook => "3-4",
            VideoType::TitleSting => "4-5",
            VideoType::Intro => "3",
            VideoType::Explain => "2",
            VideoType::VisualDemo => "4",
            VideoType::Build => "2->5",
            VideoType::Reveal => "5",
            VideoType::Reflect => "1",
            VideoType::Sponsor => "2",
            VideoType::RecapCta => "3, falling",
            VideoType::OutroEndScreen => "2-3",
        }
    }

    /// Energy when the section starts and when it ends (1-5).
    pub fn energy_span(self) -> (f64, f64) {
        match self {
            VideoType::ColdOpenHook => (3.5, 3.5),
            VideoType::TitleSting => (4.5, 4.5),
            VideoType::Intro => (3.0, 3.0),
            VideoType::Explain => (2.0, 2.0),
            VideoType::VisualDemo => (4.0, 4.0),
            VideoType::Build => (2.0, 5.0),
            VideoType::Reveal => (5.0, 5.0),
            VideoType::Reflect => (1.0, 1.0),
            VideoType::Sponsor => (2.0, 2.0),
            VideoType::RecapCta => (3.0, 2.5),
            VideoType::OutroEndScreen => (2.5, 2.5),
        }
    }

    /// Single energy figure used for the loudness target.
    pub fn energy(self) -> f64 {
        let (a, b) = self.energy_span();
        0.5 * (a + b)
    }

    /// Speech-heavy sections never carry the lead and keep picking low.
    pub fn speech_heavy(self) -> bool {
        matches!(
            self,
            VideoType::ColdOpenHook
                | VideoType::Intro
                | VideoType::Explain
                | VideoType::Sponsor
                | VideoType::RecapCta
        )
    }
}

impl fmt::Display for VideoType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Instrument {
    Flute,
    Guitar,
    Mallet,
    Pad,
    HeavyBass,
    SoftBass,
    Kick,
    Shaker,
    Rim,
    Chip,
    SynthPluck,
    Blip,
    Clap,
    Riser,
}

impl Instrument {
    /// The instruments of the spring and summer feels, whose default
    /// allow-list has always been exactly these.
    pub const CLASSIC: [Instrument; 9] = [
        Instrument::Flute,
        Instrument::Guitar,
        Instrument::Mallet,
        Instrument::Pad,
        Instrument::HeavyBass,
        Instrument::SoftBass,
        Instrument::Kick,
        Instrument::Shaker,
        Instrument::Rim,
    ];

    pub const ALL: [Instrument; 14] = [
        Instrument::Flute,
        Instrument::Guitar,
        Instrument::Mallet,
        Instrument::Pad,
        Instrument::HeavyBass,
        Instrument::SoftBass,
        Instrument::Kick,
        Instrument::Shaker,
        Instrument::Rim,
        Instrument::Chip,
        Instrument::SynthPluck,
        Instrument::Blip,
        Instrument::Clap,
        Instrument::Riser,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Instrument::Flute => "flute",
            Instrument::Guitar => "guitar",
            Instrument::Mallet => "mallet",
            Instrument::Pad => "pad",
            Instrument::HeavyBass => "heavy-bass",
            Instrument::SoftBass => "soft-bass",
            Instrument::Kick => "kick",
            Instrument::Shaker => "shaker",
            Instrument::Rim => "rim",
            Instrument::Chip => "chip",
            Instrument::SynthPluck => "synth-pluck",
            Instrument::Blip => "blip",
            Instrument::Clap => "clap",
            Instrument::Riser => "riser",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|i| i.id() == s)
    }

    pub fn description(self) -> &'static str {
        match self {
            Instrument::Flute => "breathy flute with delayed vibrato",
            Instrument::Guitar => "acoustic-ish additive pluck",
            Instrument::Mallet => "marimba-like mallet",
            Instrument::Pad => "warm detuned pad",
            Instrument::HeavyBass => "gritty saturated saw+sine bass",
            Instrument::SoftBass => "soft round bass",
            Instrument::Kick => "soft pitched kick",
            Instrument::Shaker => "high-passed noise shaker",
            Instrument::Rim => "band-passed rim click",
            Instrument::Chip => "band-limited 25% pulse lead with delayed vibrato",
            Instrument::SynthPluck => "bright short pluck for chiptune arpeggios",
            Instrument::Blip => "short rising sine blip for UI pops and steps",
            Instrument::Clap => "three-burst band-passed noise clap",
            Instrument::Riser => "high-passed noise swell (or its reverse)",
        }
    }

    /// Constant-power pan position, -1 (left) to +1 (right).
    pub fn pan(self) -> f64 {
        match self {
            Instrument::Flute => 0.1,
            Instrument::Guitar => -0.35,
            Instrument::Mallet => 0.4,
            Instrument::Shaker => 0.25,
            Instrument::Rim => -0.15,
            Instrument::Chip => 0.05,
            Instrument::SynthPluck => -0.3,
            Instrument::Blip => 0.35,
            Instrument::Clap => -0.1,
            _ => 0.0,
        }
    }

    /// Reverb send level.
    pub fn send(self) -> f64 {
        match self {
            Instrument::Flute => 0.28,
            Instrument::Guitar => 0.22,
            Instrument::Mallet => 0.25,
            Instrument::Pad => 0.30,
            Instrument::Shaker => 0.12,
            Instrument::Rim => 0.15,
            Instrument::Chip => 0.28,
            Instrument::SynthPluck => 0.22,
            Instrument::Blip => 0.25,
            Instrument::Clap => 0.15,
            Instrument::Riser => 0.12,
            _ => 0.0,
        }
    }
}

impl fmt::Display for Instrument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// What a part does in the arrangement. Arrangements are written in roles;
/// instruments are bound to roles afterwards through the allow-list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Lead,
    Motif,
    ChordHits,
    Picking,
    Pad,
    Bass,
    Kick,
    Shaker,
    /// Beats 2 and 4: rim, or clap in the chip feel. Its place in this list
    /// is part of every note id, so new roles only ever go after it.
    Backbeat,
    /// Small accents: ticks, pops, entrance objects.
    Fx,
    /// Noise swells.
    Riser,
}

impl Role {
    pub const COUNT: usize = 11;

    pub const ALL: [Role; 11] = [
        Role::Lead,
        Role::Motif,
        Role::ChordHits,
        Role::Picking,
        Role::Pad,
        Role::Bass,
        Role::Kick,
        Role::Shaker,
        Role::Backbeat,
        Role::Fx,
        Role::Riser,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Role::Lead => "lead",
            Role::Motif => "motif",
            Role::ChordHits => "chord-hits",
            Role::Picking => "picking",
            Role::Pad => "pad",
            Role::Bass => "bass",
            Role::Kick => "kick",
            Role::Shaker => "shaker",
            Role::Backbeat => "backbeat",
            Role::Fx => "fx",
            Role::Riser => "riser",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Role::Lead => "melody",
            Role::Motif => "signature motif, replies and transition runs",
            Role::ChordHits => "off-beat chord stabs",
            Role::Picking => "arpeggios and strums",
            Role::Pad => "sustained chords",
            Role::Bass => "bass line",
            Role::Kick => "kick drum",
            Role::Shaker => "shaker",
            Role::Backbeat => "beats 2 and 4",
            Role::Fx => "ticks, pops and other small accents",
            Role::Riser => "noise swells into and out of hits",
        }
    }

    /// Rough contribution to a section's energy, used for the energy warning.
    pub fn energy_weight(self) -> f64 {
        match self {
            Role::Lead | Role::Picking | Role::Bass | Role::Kick => 1.0,
            _ => 0.5,
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Feel {
    Spring,
    Summer,
    Chip,
}

impl Feel {
    pub const ALL: [Feel; 3] = [Feel::Spring, Feel::Summer, Feel::Chip];

    pub fn id(self) -> &'static str {
        match self {
            Feel::Spring => "spring",
            Feel::Summer => "summer",
            Feel::Chip => "chip",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|f| f.id() == s)
    }

    pub fn description(self) -> &'static str {
        match self {
            Feel::Spring => "92-100 BPM, slow build, gritty bass, D major",
            Feel::Summer => "116-124 BPM, four-on-the-floor, soft round bass, D major",
            Feel::Chip => {
                "140-160 BPM chiptune pop: 7th chords, synth-pluck, chip lead, clap, C major"
            }
        }
    }

    /// Inclusive BPM search range.
    pub fn bpm_range(self) -> (f64, f64) {
        match self {
            Feel::Spring => (92.0, 100.0),
            Feel::Summer => (116.0, 124.0),
            Feel::Chip => (140.0, 160.0),
        }
    }

    /// The allow-list when none is given. Spring and summer keep the
    /// instruments they have always had, so old plans sound the same.
    pub fn default_instruments(self) -> Vec<Instrument> {
        match self {
            Feel::Spring | Feel::Summer => Instrument::CLASSIC.to_vec(),
            Feel::Chip => Instrument::ALL.to_vec(),
        }
    }

    pub fn bpm_center(self) -> f64 {
        let (a, b) = self.bpm_range();
        0.5 * (a + b)
    }
}
