# cozy-score

A command-line tool that turns a video plan into an original, cozy
farm-sim style background track whose structure follows the video's. You
give it a list of `[video-type, timespan]` sections, plus optionally which
instruments are allowed, and it renders the music.

Everything is synthesised in code: acoustic-ish plucks, a breathy flute
lead, a marimba-like mallet, a warm pad, a gritty or round synth bass and
light percussion. It uses no samples and no audio from existing games or
songs, and every melody is generated from the seed.

> **Listen before you publish.** Loudness, balance and determinism are
> measured and tested. Whether the result sounds good is not, and a person
> still has to listen to it.

## Install

**Prebuilt binaries** come from the repository's GitHub releases:

- `cozy-score-dev` is a prerelease rebuilt from every push to `main`.
- `vX.Y.Z` / `cozy-score-vX.Y.Z` are stable releases, one per tag.

Each release has archives for Linux x86_64, macOS Apple Silicon and
Windows x86_64, plus a `SHA256SUMS` file.

**From source** (the toolchain is pinned in `rust-toolchain.toml`, and
rustup installs it on first use):

```bash
cd tooling/cozy-score
cargo build --release
./target/release/cozy-score --help
```

Run all commands from `tooling/cozy-score/`. Renders go to `out/`, which git ignores.

## Usage

```bash
cargo run --release -- render --plan plans/reference-60s.json --out out/ --feel spring --seed 7
```

```bash
cargo run --release -- render --plan '[["cold-open-hook","0:00-0:04"],["explain","0:04-0:20"]]' --out out/
```

```bash
cargo run --release -- render --plan plan.json --instruments guitar,flute,pad,heavy-bass,kick,shaker --stems --mp3
```

```bash
cargo run --release -- types
```

```bash
cargo run --release -- instruments
```

```bash
cargo run --release -- analyze --wav out/full.wav --cue out/cue.json
```

| Option | Default | Meaning |
|---|---|---|
| `--plan` | required | Path to a JSON plan, or the JSON itself |
| `--out` | `out` | Output directory |
| `--feel spring\|summer` | `spring` | `spring`: 92–100 BPM, slow build, gritty bass. `summer`: 116–124 BPM, four-on-the-floor, soft round bass |
| `--seed` | `7` | Every random choice comes from this seed |
| `--instruments a,b,c` | all | Instruments allowed for the whole score |
| `--stems` | off | One WAV per bus in `<out>/stems/` |
| `--mp3` | off | A 256 kbps MP3 next to each WAV (needs `ffmpeg` on PATH; otherwise warns and skips) |
| `--gain-from cue.json` | off | Reuse another render's master gain and saturation curve, so two renders can be compared sample by sample |

### Outputs

- `full.wav` and `no-melody.wav`: 16-bit stereo, 44.1 kHz, rendered at the
  same gain. `no-melody.wav` leaves out the **lead** role, whichever
  instrument plays it.
- `cue.json` holds:
  - the seed, feel, BPM, tool version and build;
  - the master loudness, peak and gain;
  - for each section: requested and snapped times with the snap offsets,
    chords, the instrument list that applied, each role's instrument and
    status (`preferred` / `substituted` / `dropped`), the measured LUFS
    (absolute and relative to the loudest section), the energy target and
    trim, and the spectral band balance;
  - the transitions;
  - every warning.
- `stems/*.wav` (with `--stems`): one file per bus: `lead`, `flute`,
  `guitar`, `mallet`, `pad`, `heavy-bass`, `soft-bass`, `kick`, `shaker`,
  `rim`.
  - The lead role has its own bus, so `flute.wav` only holds flute playing
    other roles.
  - Stems go through the same reverb, fades, EQ and gain as the mix, but not
    the peak saturation, so their sum is close to `full.wav` but not exact.
- `*.mp3` (with `--mp3`).

Substitutions, dropped roles and warnings are also printed to stderr, one
line each. The tool never ducks for speech; do that in your editor.

## Plans

A plan is a JSON array of sections. A section can take any of three forms:

```jsonc
["explain", "0:04-0:20"]                                    // uses the whole-score instrument list
["explain", "0:04-0:20", ["guitar", "pad", "soft-bass"]]    // this section only uses these
{"type": "explain", "span": "0:04-0:20", "instruments": ["guitar", "pad"]}
```

- **Timespans:** `"m:ss-m:ss"`, `"m:ss.s-m:ss.s"` (also `h:mm:ss`) or
  `[startSeconds, endSeconds]`.
- **Validation:** sections must start at 0:00 and be contiguous, with no
  gaps or overlaps. Unknown types and instrument ids are rejected along
  with the list of valid ones, and every error is reported in one go.
- **Section lists:** a section's own `instruments` list *replaces* the
  whole-score list for that section; the two are not combined. An empty
  list is valid: that section is silent, with a warning.

Examples:

- [`plans/reference-60s.json`](plans/reference-60s.json): the 60 s
  acceptance plan.
- [`plans/summer-tutorial.json`](plans/summer-tutorial.json): a 2:50
  tutorial with a sponsor read, per-section instrument lists and every
  section form.

## Video types

Energy runs from 1 to 5. Transitions are added automatically (see below).

| id | Role in the video | Energy | Arrangement (preferred instruments) |
|---|---|---|---|
| `cold-open-hook` | First 3–30 s: question or teaser, with speech | 3–4 | 8th-note guitar (low), a 1–2 bar fragment of the signature motif on mallet, shaker 8ths, soft kick on 1 and 3, short heavy-bass pulses at reduced gain. No lead |
| `title-sting` | 2–8 s logo or title card, no speech | 4–5 | Full band. The signature motif (the same for a given seed every time) on lead and motif, and a hit (strum, kick, bass) on the downbeat after it |
| `intro` | Roadmap or promise, with speech | 3 | Slow quarter-note guitar, a few long soft lead notes, pad. Shaker joins halfway. No bass, no kick |
| `explain` | Core teaching, talking head or voiceover | 2 | Bass groove at ~80%, kick, shaker, quiet rim, low guitar 8ths, pad, a 3-note mallet reply every 2 bars. **No lead** |
| `visual-demo` | Graphics/pixel-art showcase, little speech | 4 | Everything: bass groove, kick, rim on 2 and 4, shaker, guitar 8ths, lead melody, off-beat mallet chord-hits |
| `build` | Rising tension toward a reveal | 2→5 | Long bass notes that turn into 8th pulses, rising shaker density and pad level, kick from halfway, a mallet run into the next section |
| `reveal` | Payoff, the "wow" moment | 5 | `visual-demo` plus a hit on the first downbeat, a lead melody up to F#6 and a louder rim |
| `reflect` | Quiet, emotional or philosophical beat | 1 | Pad, slow guitar, soft long lead notes (4+ bars), soft held bass. No drums. Loop `G D/F# Em A` |
| `sponsor` | Mid-roll sponsor read | 2 | Neutral bed: pad, soft bass in half notes, low guitar 8ths, light shaker. `D G` loop, no lead, no replies |
| `recap-cta` | Takeaways plus subscribe/watch-next, with speech | 3, falling | Guitar, pad, soft bass, shaker. The lead returns at low gain and resolves on a IV–V–I (`G A D`) cadence |
| `outro-end-screen` | End card, little or no speech | 2–3 | A recap of the first melody, slow picking, held bass, a final strummed D chord. A 3 s fade if it is the last section |

Per-section loudness targets, relative to the loudest section: energy
5 = 0 LU, 4 = −1.5, 3 = −3, 2 = −5, 1 = −8 (ranges are interpolated).

## Instruments and roles

Arrangements are written in roles. Each role is filled by the first
allowed instrument in its row; a substitute keeps the role's rhythm and
notes but uses its own sound.

| Role | Preferred | Fallbacks, in order | If none is allowed |
|---|---|---|---|
| lead (melody) | `flute` | `mallet`, `guitar` (single notes, one octave up) | no melody |
| motif (signature, replies, transition runs) | `mallet` | `flute`, `guitar` | dropped |
| chord-hits (off-beat stabs) | `mallet` | `guitar` (short muted plucks) | dropped |
| picking (arpeggios, strums) | `guitar` | `mallet` (softer gain, short notes) | dropped |
| pad | `pad` | – | dropped |
| bass | `heavy-bass` in spring, `soft-bass` in summer and in sponsor, recap-cta and reflect | the other bass | no bass |
| kick, shaker, rim | themselves | – | dropped |

| Instrument | Sound | Pan | Reverb send |
|---|---|---|---|
| `flute` | sine plus 3 harmonics, delayed 5.2 Hz vibrato, band-passed breath and chiff | +0.1 | 0.28 |
| `guitar` | 13 slightly inharmonic decaying partials plus a pick noise burst | −0.35 | 0.22 |
| `mallet` | three inharmonic modes (1, 3.98, 9.2) | +0.4 | 0.25 |
| `pad` | 3 detunes × 6 harmonics, random phases, slow attack and release | centre | 0.30 |
| `heavy-bass` | saw up to 2.2 kHz plus a fundamental sine, `tanh` grit (73–123 Hz roots, nothing an octave below) | centre | – |
| `soft-bass` | sine with soft 2nd and 3rd harmonics, gentle `tanh` | centre | – |
| `kick` | 158→48 Hz pitch sweep with a click | centre | – |
| `shaker` | noise high-passed at 5 kHz, 90 ms | +0.25 | 0.12 |
| `rim` | band-passed noise plus a 410 Hz tone, 120 ms | −0.15 | 0.15 |

If dropped roles mean a section can't reach its energy (for example a
`reveal` with only `pad`), the tool renders what is allowed and warns.
Changing the instruments never changes the notes: melodies are generated
before the allow-lists are applied.

## How it works

The pipeline is a chain of pure stages: parse → validate → timing
(tempo and snapping) → arrangement (note events by role) → instrument
resolution → synthesis → mix → master → encode.

- **Key and tempo.** The key is D major. The tempo is searched in 0.1 BPM
  steps within the feel's range so the beat grid best fits all section
  boundaries. Each boundary snaps to the nearest beat; the offsets go in
  `cue.json`, with a warning for any boundary that moves more than 0.25 s.
  The audio is exactly as long as the plan.
- **Harmony.** One chord per bar. Each section's loop is picked with the
  seeded RNG, never repeating the previous section's. Sections are filled
  bar by bar, and a partial last bar cuts the pattern short.
- **Melody.**
  - Phrases are built from 1-bar rhythm cells, as 2+2 or 4+4 bar
    statements followed by varied repeats (A A′).
  - Strong beats land on chord tones.
  - Motion is mostly stepwise; leaps are at most a major 6th and are
    followed by a step back the other way.
  - Each phrase ends on a held chord tone.
  - The range is D5–E6 (D5–F#6 in `reveal`).
- **Transitions,** decided by the energy change at each boundary:

  | Energy change | What happens |
  |---|---|
  | +2 or more | 6-note motif run up into the boundary, plus a shaker crescendo |
  | +1 | 3-note pickup |
  | −1 or less | drums rest for the last beat, plus a descending 3-note figure |

  Going into `title-sting` or `reveal`, or out of `build`, always ends on a
  hit. A transition belongs to the section it leads into and uses that
  section's instruments.
- **Mix and master.**
  - Constant-power panning, then a seeded 1.9 s stereo convolution reverb
    (wet 0.9).
  - A 0.4 s fade-in and a 3 s fade-out.
  - High-pass at 35 Hz and a +2.9 dB shelf above 5 kHz.
  - `tanh` saturation on the peaks.
  - Normalisation to −14 LUFS integrated (BS.1770-4, implemented here) with
    a −1 dBFS sample-peak ceiling. If the ceiling sets the gain, the tool
    says so.
- **Section levels.** Each section is trimmed toward its energy target.
  - The trims are calibrated by measuring the *unrestricted* arrangement
    (every instrument allowed), so they depend only on the plan, feel and
    seed.
  - A restricted section keeps the trim of its full version and comes only
    as close to its target as its instruments allow.

## Determinism

The same plan, instruments, seed, feel and tool version give a
byte-identical WAV:

- **One seeded PRNG.** All randomness comes from one xoshiro256** generator
  in the crate. Independent streams are forked by label (section, role,
  note id), so restricting one section can't shift another section's noise.
- **No platform maths.** All `sin`, `exp`, `tanh`, `pow` and `log` calls in
  the signal path use the pure-Rust `libm` crate. `clippy.toml` forbids the
  `f64` methods that call the platform library, as well as `mul_add`.
- **Deterministic FFT.** The convolution reverb's FFT is part of the crate,
  with `libm` twiddles, and doesn't depend on CPU feature detection.
- **Fixed order.** No `HashMap` iteration. Sections render in parallel but
  are summed in a fixed order, and each note's audio depends only on its
  id.
- **Pinned toolchain.** The toolchain and dependencies are pinned
  (`rust-toolchain.toml`, exact versions, `Cargo.lock`).

`tests/determinism.rs` renders `tests/fixtures/reference-short.json` and
compares it against `tests/fixtures/reference-short.sha256`. CI runs it on
Linux, macOS and Windows. When the sound changes on purpose, update the
hash deliberately:

```bash
COZY_UPDATE_HASH=1 cargo test --release --test determinism
```

## Development

```bash
cargo fmt --check
```

```bash
cargo clippy --all-targets -- -D warnings
```

```bash
cargo test --release
```

- **Unit tests** cover timespan parsing, the section forms and validation,
  tempo search and snapping, voicings, melody constraints, the RNG, the FFT,
  convolution, filters and the loudness meter. A 1 kHz sine at −20 dBFS on
  one channel reads −23.0008 LUFS at 44.1 kHz: the mean square is −23.0103
  dB, the K-weighting gain at 1 kHz is +0.7005 dB, and the offset is −0.691.
- **Integration tests** (`tests/allow_lists.rs`) cover allow-list overrides,
  fallback order, dropped roles, empty lists, melody invariance and
  transition selection.

Rendering the 60 s reference plan takes about 3 s on a 4-core machine
(about 5 s with `--stems`).

To compare a variant against a base render sample by sample, reuse the
base render's master gain:

```bash
cargo run --release -- render --plan base.json --out out/base
```

```bash
cargo run --release -- render --plan variant.json --out out/variant --gain-from out/base/cue.json
```

## Notes on interpretation

- When mallet fills the picking role it plays the same notes at 0.6× gain,
  with its own short decay instead of the guitar's long ring.
- The plan's last end time is the length of the audio. Internal boundaries
  snap to beats; the final section plays to the end of the audio.
- `title-sting` uses a `D G` progression so its signature motif always fits.
  `cold-open-hook` plays a fragment of the same motif on its first D bar.
- The A/C# voicing's bass note (C#2, 69 Hz) is taken from the voicing
  table, so it is the one root just below the 73–123 Hz range.
