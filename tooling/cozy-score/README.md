# cozy-score

A command-line tool that turns a video plan into an original, cozy
farm-sim style background track whose structure follows the video's. You
give it a list of `[video-type, timespan]` sections, plus optionally which
instruments are allowed, and it renders the music.

It can also hit things on screen: **events** put runs, ticks and hits at
exact video times, and pixellab scenes can be scored piece by piece as they
build up (`entrance`) or break apart (`exit`). The **chip** feel turns the
same plans into cheerful chiptune pop for tech and UI-style pixel art.

Everything is synthesised in code: acoustic-ish plucks, a breathy flute
lead, a marimba-like mallet, a warm pad, a gritty or round synth bass, light
percussion, and for chip a pulse lead, a synth pluck, blips, a clap and noise
risers. It uses no samples and no audio from existing games or songs, and
every melody is generated from the seed.

> **Listen before you publish.** Loudness, balance, timing and determinism
> are measured and tested. Whether the result sounds good is not, and a
> person still has to listen to it.

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
cargo run --release -- render --plan plans/startup-sting.json --out out/startup-sting --seed 7 --stems --video clip.mp4
```

```bash
cargo run --release -- detect --video clip.mp4 --out plan.json
```

```bash
cargo run --release -- from-pixellab --sequence seq.json --entrances entrances/ --out plan.json
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

`render` options:

| Option | Default | Meaning |
|---|---|---|
| `--plan` | required | Path to a JSON plan, or the JSON itself |
| `--out` | `out` | Output directory |
| `--feel spring\|summer\|chip` | the plan's `feel`, else `spring` | `spring`: 92–100 BPM, slow build, gritty bass. `summer`: 116–124 BPM, four-on-the-floor, soft round bass. `chip`: 140–160 BPM chiptune pop (see [The chip feel](#the-chip-feel)) |
| `--seed` | `7` | Every random choice comes from this seed |
| `--instruments a,b,c` | the feel's | Instruments allowed for the whole score. Spring and summer allow the first nine instruments; chip allows all of them |
| `--bpm <n>` | searched | An exact tempo, 60–200 (overrides the plan's `bpm`) |
| `--key <C\|C#\|Db\|…\|B>` | the feel's (D, or C for chip) | A major key (overrides the plan's `key`) |
| `--fade-out <s>` | `min(3, 0.1 × length)` | Seconds of fade at the end |
| `--tail <s>` | `0` | Extra audio after the last section, so the final hit rings out |
| `--video <in.mp4>` | off | Also write `with-music.mp4` (needs `ffmpeg`) |
| `--fps <n>` | off | Align entrance and exit sounds to this frame rate |
| `--pixellab-document <file>` | none | The `.pixellab` document `scene` sources read (needs `pixellab` on PATH) |
| `--stems` | off | One WAV per bus in `<out>/stems/` |
| `--mp3` | off | A 256 kbps MP3 next to each WAV (needs `ffmpeg` on PATH; otherwise warns and skips) |
| `--gain-from cue.json` | off | Reuse another render's master gain and saturation curve, so two renders can be compared sample by sample |

A bare array plan rendered without `--bpm`, `--key`, `--tail` or the chip
feel keeps the original fade, `min(3 s, the last section)`, so it renders
byte for byte as before.

### Outputs

- `full.wav` and `no-melody.wav`: 16-bit stereo, 44.1 kHz, rendered at the
  same gain. `no-melody.wav` leaves out the **lead** role, whichever
  instrument plays it. Events are never the lead, so both files have them.
- `cue.json` holds:
  - the seed, feel, BPM (and whether it was searched or given), key, tool
    version and build;
  - the video's length and the audio's (they differ with `--tail`), the tail
    and the fade-out;
  - the master loudness, peak and gain;
  - for each section: requested and snapped times with the snap offsets,
    chords, the instrument list that applied, each role's instrument and
    status (`preferred` / `substituted` / `dropped`), the measured LUFS
    (absolute and relative to the loudest section), the energy target and
    trim, and the spectral band balance;
  - the transitions;
  - every event: its resolved times, the instruments it used, each sound it
    made (time, the time before frame alignment, what it is, its role,
    section, instrument, MIDI note and level), what was skipped, and for
    entrances and exits the scene's `settledBy` and counts;
  - every warning.
- `stems/*.wav` (with `--stems`): one file per bus: `lead`, `flute`,
  `guitar`, `mallet`, `pad`, `heavy-bass`, `soft-bass`, `kick`, `shaker`,
  `rim`, and `chip`, `synth-pluck`, `blip`, `clap`, `riser` when they play.
  - The lead role has its own bus, so `flute.wav` only holds flute playing
    other roles.
  - Stems go through the same reverb, fades, EQ and gain as the mix, but not
    the peak saturation, so their sum is close to `full.wav` but not exact.
- `*.mp3` (with `--mp3`).
- `with-music.mp4` (with `--video`): the video stream copied, `full.wav` as
  256 kbps AAC, cut (or padded) to the video's length.

Substitutions, dropped roles and warnings are also printed to stderr, one
line each. The tool never ducks for speech; do that in your editor.

## Plans

A plan is either a JSON array of sections (the original form, still valid
with unchanged output) or an object:

```json
{
  "feel": "chip",
  "bpm": 150,
  "key": "C",
  "sections": [
    ["build", "0:00-0:01.2", ["mallet", "synth-pluck", "riser"]],
    ["visual-demo", "0:01.2-0:05.9"],
    ["outro-end-screen", "0:05.9-0:07"]
  ],
  "events": [
    {"kind": "run", "direction": "up", "start": 0.2, "count": 10, "step": "1/16", "land": 1.2, "riser": true},
    {"kind": "ticks", "times": [1.4, 1.8, 2.2, 2.4, 2.8, 3.0, 3.4, 3.8, 4.4, 4.8, 5.4], "accent": [3.8]},
    {"kind": "run", "direction": "down", "start": 5.95, "count": 9, "step": "1/16", "riser": "reverse"},
    {"kind": "hit", "at": 6.8, "strength": "pop"}
  ]
}
```

`feel`, `bpm`, `key`, `fps` and `events` are optional (command-line flags
override them), and `notes` is free text for people (drafted plans use it).

A section can take any of three forms:

```jsonc
["explain", "0:04-0:20"]                                    // uses the whole-score instrument list
["explain", "0:04-0:20", ["guitar", "pad", "soft-bass"]]    // this section only uses these
{"type": "explain", "span": "0:04-0:20", "instruments": ["guitar", "pad"]}
```

- **Timespans:** `"m:ss-m:ss"`, `"m:ss.s-m:ss.s"` (also `h:mm:ss`) or
  `[startSeconds, endSeconds]`.
- **Validation:** sections must start at 0:00 and be contiguous, with no
  gaps or overlaps. Unknown types, instrument ids, keys and event fields are
  rejected along with what is valid, and every error is reported in one go.
- **Section lists:** a section's own `instruments` list *replaces* the
  whole-score list for that section; the two are not combined. An empty
  list is valid: that section is silent, with a warning.

Examples:

- [`plans/reference-60s.json`](plans/reference-60s.json): the 60 s
  acceptance plan.
- [`plans/summer-tutorial.json`](plans/summer-tutorial.json): a 2:50
  tutorial with a sponsor read, per-section instrument lists and every
  section form.
- [`plans/startup-sting.json`](plans/startup-sting.json): the 7 s "startup
  office" sting above, in the chip feel.
- [`plans/entrance-hero.json`](plans/entrance-hero.json): pixellab's hero
  island building up, scored piece by piece.

## Events

Events are sounds tied to things on screen. Times are in seconds, or
`"m:ss.s"` strings like the sections use.

### `run`

A staggered sequence of notes, for a wave of pieces appearing.

| Field | Default | Meaning |
|---|---|---|
| `start` | required | When the first note plays |
| `count` + `step` | | `step` is a note value (`"1/8"`, `"1/16"`, `"1/32"`, `"1/8t"`) or seconds (`0.085`) |
| `count` + `end` | | The step is `(end - start) / (count - 1)` |
| `times` | | An explicit list, for irregular staggers |
| `direction` | `up` | `up` (ends on the top note), `down` (starts there), `up-down`, `down-up`, `random` (never the same note twice running) |
| `scale` | `pentatonic` | `pentatonic`, `major`, or `chord` (the tones of the chord sounding at each note), in the score's key |
| `range` | `[60, 88]` | `[lowMidi, highMidi]` |
| `gain` | `[0.09, 0.2]` up, the reverse down | A linear ramp of absolute levels across the notes |
| `instrument` | the motif role's | Asked for first; the **motif** role's fallbacks apply otherwise |
| `riser` | `false` | `true`: a noise swell from `start` to the landing (or the run's end). `"reverse"`: a falling swell from `start` across the run |
| `land` | none | A `land` hit here. If the last note would fall at or after it, the step is compressed so the landing becomes the next step, with a warning |
| `quantize` | `"1/16"` | See [Quantizing](#quantizing) |

Give `times`, or `count` with exactly one of `step` and `end`; anything else
is an error.

### `ticks`

Small repeated accents: character steps, UI pops, items appearing.

| Field | Default | Meaning |
|---|---|---|
| `times` | | A list of times |
| `start` + `every` + `count` | | `every` is a note value or seconds, or a list cycled to describe a stagger pattern: `[0.4, 0.4, 0.2]` |
| `sound` | `blip` | `blip`, `mallet`, `clap` or `shaker`, through the **fx** role |
| pitch | seeded | A seeded choice from the high pentatonic set (MIDI 84, 86, 88, 91, 93, 96 in C, moved to the key), or `"contour": "up" \| "down" \| "wave"` over that set, or explicit `"notes": [...]` (cycled) |
| `accent` | `[]` | Ticks at these times get +50% gain and a second note 50 ms later, a fourth higher |
| `gain` | `1` | A share of the fx instrument's level |
| `quantize` | `"1/16"` | |

### `hit`

A one-shot accent at `at`: a kick, the current chord on mallet one octave up
(the **motif** role's fallbacks apply), and a bass root.

| `strength` | Adds |
|---|---|
| `soft` | nothing (and plays at 60%) |
| `land` (default) | a 4-note sparkle on blip: MIDI 96, 100, 103, 108 (in C), 30 ms apart |
| `pop` | a blip on the tonic and a short chip note on the tonic |
| `big` | both, at 120% |

### `riser`

A noise swell on its own: `{"kind": "riser", "start": 3, "end": 4}`, or
`"reverse": true` for a falling one. `from-pixellab` uses it for fades and
dips.

### `entrance` and `exit`

A pixellab scene building up, or breaking apart; see
[pixellab entrances, exits and sequences](#pixellab-entrances-exits-and-sequences).

### Quantizing

Event times snap to the nearest 1/16 note when within 15 ms. Times further
off stay exactly where they are (they are tied to video frames), with one
warning per event. `"quantize": "off"` keeps every time exact, and a note
value (`"1/8"`, `"1/32"`, ...) changes the grid. Entrance and exit events
are off by default.

### How events fit the music

- **Tempo:** without `bpm`, the tempo search picks the tempo in the
  feel's range whose 1/16 grid best fits the event times (`ticks` count four
  times as much; each distance counts at most 15 ms, since a time further
  off stays exact anyway), while moving the section boundaries as little as
  it can onto the beat: a move counts at a tenth of an event's weight up to
  0.125 s, and in full beyond. The reference plan's ticks fit 150 BPM
  exactly.
- **Instruments:** each sound uses the allow-list of the section it falls
  in, through roles: runs and hits use **motif**, ticks **fx**, swells
  **riser**. A sound nothing can play is skipped and listed in `cue.json`.
- **Transitions:** a boundary where a `run` ends (its last note plus a
  step) or lands within one beat gets no automatic transition, so the two
  don't double up; that includes the `build` section's own run into the
  next section. Entrances and exits that settle within a beat of a boundary
  count too.
- **Randomness:** each event draws from its own stream (`events/<index>/run`,
  `events/<index>/ticks`, ...), so adding an event never changes anything
  else.

## pixellab entrances, exits and sequences

[pixellab](https://github.com/jlawcordova/jlawcordova-lab) builds pixel-art
scenes. A scene's **entrance** builds it up piece by piece: a shadow fades
in, the ground blocks enter in an order, the objects follow (or ride in with
the block under them), and the sky drifts in. An **exit** is the entrance
played backwards; sequences use one for their `blocks` transition.

cozy-score ports pixellab's schedule exactly, from
`crates/engine/src/entrance.rs` and `crates/engine/src/sequence.rs` at commit
[`8a3179f`](https://github.com/jlawcordova/jlawcordova-lab/tree/8a3179f) of
jlawcordova/jlawcordova-lab. `fixtures/pixellab/parity.json` is pixellab's own
parity fixture (`fixtures/entrance/parity.json` at that commit, copied as
is), and the port reproduces every one of its 42 cases to 1e-9 s. The
fixture's pieces are on the site's 32×16 tiles; like pixellab's own test,
cozy-score's scales their x and y by 44/32 onto pixellab's 44×22 tiles.

### The entrance configuration

All times are whole ms (0–60000); a field left out takes pixellab's default.
Unknown fields and out-of-range values are rejected with pixellab's own
messages.

| field | default | meaning |
|---|---|---|
| `start` | 200 | when the entrance begins (the shadow fades in here) |
| `groundDelay` | 120 | from `start` to the first ground block |
| `groundSpan` | 1000 | how long the ground order is spread over |
| `cascade` | 250 | extra delay from the left of the ground to the right |
| `ground` | `sweep` | `rows` `cols` `back-front` `front-back` `sweep` `center` `edges` `clock` `checker` `random` `together` |
| `objects` | `same` | without `with`: `ltr` `same` `back-front` `front-back` `random` `together` |
| `with` | true | each object rides in with its nearest block, `lag` ms after it |
| `objectLead` / `objectSpan` / `lag` | 80 / 600 / 500 | object timing |
| `move` / `objectMove` | `float` / `drop` | `float` `drop` `pop` `stepped` `fade` (`objectMove` also `same`) |
| `duration` / `objectDuration` | 350 / 300 | length of each piece's move |
| `peak` / `objectPeak` | 0.6 / 0.5 | share of the move spent reaching the overshoot (the impact) |
| `rise` / `objectRise` | 16 / 24 px | travel distance |
| `overshoot` / `objectOvershoot` | 6 / 1.5 px | overshoot distance |
| `popFrom`/`popPeak`, `objectPopFrom`/`objectPopPeak` | 0.3/1.1 | scale range of a `pop` |
| `shadowDuration` / `skyDuration` / `skyDrift` | 400 / 600 / 6 px | shadow and sky |
| `seed` | 1 | seed for the `random` orders |

### Three ways to give a scene

Each `entrance` or `exit` event takes exactly one source.

1. **`from`** (preferred): the output of `pixellab scene entrance show
   <scene> --json`, with exact per-piece times. Paths are relative to the
   plan file.

   ```json
   {"kind": "entrance", "from": "entrances/office.json", "at": 0.0}
   ```

2. **`pieces` + `timing`**: the pieces without times (`role` or `enter`,
   `col`, `row`, `x`, `y`, in pixellab's scene pixels) and any entrance
   fields in ms; cozy-score works out the schedule. `siteTiming` takes the
   site's seconds form instead, as `pixellab scene entrance set --timing`
   does.

   ```json
   {"kind": "entrance", "at": 0, "timing": {"ground": "center"},
    "pieces": [{"role": "shadow"}, {"role": "ground", "col": 0, "row": 0, "x": 0, "y": 0},
               {"role": "ground", "col": 1, "row": 0, "x": 22, "y": 11},
               {"role": "object", "x": 22, "y": 11}]}
   ```

   ```json
   {"kind": "entrance", "at": 0, "siteTiming": {"start": 0.2, "ground": "sweep"}, "pieces": [...]}
   ```

3. **`grid`**: a synthetic isometric grid when there is no scene file. Block
   (u, v) sits at x = (u − v)·22, y = (u + v)·11, and objects go on seeded
   distinct cells. `cue.json` marks it `"approximate": true`.

   ```json
   {"kind": "entrance", "at": 0, "grid": {"cols": 11, "rows": 7, "objects": 12, "shadow": true, "sky": false},
    "timing": {"ground": "clock"}}
   ```

With `pixellab` on PATH, `"scene": "hero-island"` plus
`--pixellab-document <file.pixellab>` runs the read-only `pixellab scene
entrance show <scene> --json --document <file>` and uses its output as a
`from` source. Without `pixellab` it fails and says to pass `from`.

### How a scene sounds

`at` is when the entrance or exit begins in the video; pixellab's own
`start` still applies inside it.

| Piece | Sound (through the allow-list) |
|---|---|
| ground | the **motif** role (mallet; synth-pluck in chip) |
| object | the **fx** role (blip) |
| shadow | a **riser** swell over its fade (reversed on an exit) |
| sky | a pad swell over its drift, plus a 2-note sparkle where the drift ends |

`"voices": {"ground": "...", "object": "...", "shadow": "...", "sky": "..."}`
picks an instrument for any of these, or `"none"`.

| move | sync point | sound |
|---|---|---|
| `float` | start | soft note gliding up into its pitch |
| `drop` | impact (`delay + peak · duration`) | note plus a quiet low thud (a soft kick at 0.15) |
| `pop` | impact | short note |
| `stepped` | start | short note, plus up to `min(rise, 3)` quieter repeats across the move |
| `fade` | start | no attack: a 120 ms swell |

- `"sync": "start" | "impact" | "settle"` forces one sync point for every
  piece. On an exit, every sync time is mirrored: `at + settledBy − t`.
- **Pitch** (`"pitch"`): `time` (default: rising across an entrance, falling
  across an exit), `x` (left to right, low to high), `y` (back to front),
  `order` (the order's own score, so `center`, `clock` and `checker` sound
  like their shapes) or `random` (seeded). `scale` and `range` work as for
  `run`. A `checker` ground alternates between two chord tones.
- **Density:** sync points within `"cluster"` ms (default 35) of a
  cluster's first point merge into one note at that point, with gain
  `base · (1 + 0.25 ln count)`, at most 1.6 × `base`. Then at most
  `"maxRate"` notes a second are kept (default 14): a note is dropped when it
  comes sooner than `1 / maxRate` after the last one kept, so the run keeps
  its shape. `cue.json` reports the pieces, sync points, clusters and notes
  kept. An order of `together` with every piece at one time becomes a single
  chord.
- **Ending:** `"land"` (entrance, default `"land"`) or `"end"` (exit,
  default `"pop"`) puts a hit at `at + settledBy`; `false` leaves it out.
  `"riser"` (default `true` on an entrance, `"reverse"` on an exit) spans from
  the first sync point to the settle time.
- **Frames:** `"fps": 30` (or `--fps`) moves each sound to the first video
  frame at or after it, `ceil(t · fps) / fps`, matching pixellab's export,
  where frame i shows `from + i · 1000 / fps`. `cue.json` keeps both times.
- **Tempo:** the search uses `at` and `at + settledBy`, not every piece.

### Importing a sequence

```bash
cargo run --release -- from-pixellab --sequence seq.json --entrances dir/ --types '{"0":"cold-open-hook","1":"visual-demo","2":"outro-end-screen"}' --out plan.json
```

- `seq.json` is the output of `pixellab sequence show <name> --json`, and
  `dir/` holds one `pixellab scene entrance show <scene> --json` output per
  scene that enters or exits, as `<scene>.json`.
- **Sections:** one per shot, from its start to its end (hold included). The
  types come from `--types` (shot index to type); by default the first shot
  is `intro`, the last `outro-end-screen` and the rest `visual-demo`. A
  `fade`, `dip`, `slide` or `wipe` overlap is split at its midpoint; a
  `blocks` gap belongs to the following section.
- **Events:** an `entrance` at each shot that opens with its entrance (its
  own flag, or a `blocks` join before it); an `exit` at
  `shot.start + length + hold − settledBy` before each `blocks` join; a soft
  `hit` at each `cut`; a `run` across each `slide` and `wipe` (left/up → up,
  right/down → down); a `riser` across each `fade` and `dip`.
- Every time is checked against `spans()` as pixellab computes it, and the
  plan's `notes` say where each one came from.

## Detecting a plan from a video

For videos made outside pixellab, `cozy-score detect --video clip.mp4 [--out
plan.json]` drafts a plan from the motion (it needs `ffmpeg` and `ffprobe`
on PATH). For pixellab scenes and sequences, use `entrance` events and
`from-pixellab` instead: they have the exact times, where detection guesses.

1. Motion per frame is the mean luma of the difference to the previous
   frame: `ffmpeg -i clip.mp4 -vf "scale=320:-1,tblend=all_mode=difference,signalstats,metadata=print:key=lavfi.signalstats.YAVG" -f null -`.
2. Runs of at least 4 frames above a threshold (25% of the clip's 99th
   percentile; `--threshold`) are **windows**: scenes assembling,
   transforming or breaking. One quiet frame inside a window is bridged.
   Isolated 1–2 frame spikes outside them are **ticks**: sprite steps and
   pops.
3. The first window becomes an up `run` landing on the first 1/16 after it
   ends, with a riser. The last (if separate) becomes a down `run` with a
   reverse riser and a `pop` hit where the motion stops. The ticks become
   one `ticks` event with the largest spike as its accent.
4. Sections: `build` up to the landing, `visual-demo` in between, and
   `outro-end-screen` from the last window to the end.
5. Tempo: the BPM (60–200) whose 1/16 grid fits every tick within half a
   frame, preferring the feel's range (`--feel`, default `chip`).

The draft's `notes` explain each detection; edit the plan before rendering.

## The chip feel

`--feel chip`: cheerful lo-fi chiptune pop for tech, startup and UI-style
pixel art. Section types keep their energy and role rules; only the
instruments, chords and tempo change.

- **Tempo** 140–160 BPM; **key** C major.
- **Chord loops**, one chord per bar: `Cmaj7 Am7 Fmaj7 G7`,
  `Fmaj7 G7 Em7 Am7`, `Cmaj7 Fmaj7 Dm7 G7`. Sponsor uses `Cmaj7 Fmaj7`,
  reflect `Fmaj7 Cmaj7 Dm7 G7`, and endings resolve `Fmaj7 G7 C(add9)`.
- **Voicings:**

  | Chord | Bass | Pad | Arpeggio (as written) |
  |---|---|---|---|
  | Cmaj7 | 36 | 60 64 67 71 | 60 64 67 71 72 71 67 64 |
  | Am7 | 45 | 57 60 64 67 | 57 60 64 67 69 67 64 60 |
  | Fmaj7 | 41 | 57 60 64 65 | 53 57 60 64 65 64 60 57 |
  | G7 | 43 | 55 59 62 65 | 55 59 62 65 67 65 62 59 |
  | Dm7 | 38 | 57 60 62 65 | 50 53 57 60 62 60 57 53 |
  | Em7 | 40 | 55 59 62 64 | 52 55 59 62 64 62 59 55 |
  | C(add9) | 36 | 60 62 64 67 | 60 64 67 74 72 67 64 62 |

- **Groove:** synth-pluck 8ths walking the 8-note arpeggio (0.28 on the
  beat, 0.2 off it); soft bass on 8ths alternating root and octave (0.34 and
  0.24); kick on 1 and 3, clap on 2 and 4; shaker on every 8th (0.16 off the
  beat, 0.12 on it); pad at 0.12; a chip melody in MIDI 72–88 with the usual
  melody rules.

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
| `outro-end-screen` | End card, little or no speech | 2–3 | A recap of the first melody, slow picking, held bass, a final strummed D chord. A fade if it is the last section |

The instruments, chords and keys above are the spring and summer feels';
chip plays the same parts with its own (see [The chip feel](#the-chip-feel)).

Per-section loudness targets, relative to the loudest section: energy
5 = 0 LU, 4 = −1.5, 3 = −3, 2 = −5, 1 = −8 (ranges are interpolated).

## Instruments and roles

Arrangements are written in roles. Each role is filled by the first
allowed instrument in its row; a substitute keeps the role's rhythm and
notes but uses its own sound.

| Role | Preferred | Fallbacks, in order | If none is allowed |
|---|---|---|---|
| lead (melody) | `flute`; `chip` in chip | spring/summer: `chip`, `mallet`, `guitar` (single notes, one octave up). chip: `flute`, `mallet`, `guitar` | no melody |
| motif (signature, replies, transition runs, run events, hits) | `mallet`; `synth-pluck` in chip | `flute`, `guitar` (chip: `mallet`, `flute`, `guitar`) | dropped |
| chord-hits (off-beat stabs) | `mallet` | `guitar` (short muted plucks) | dropped |
| picking (arpeggios, strums) | `guitar`; `synth-pluck` in chip | the other one, then `mallet` (softer gain, short notes) | dropped |
| pad | `pad` | – | dropped |
| bass | `heavy-bass` in spring, `soft-bass` in summer, chip and in sponsor, recap-cta and reflect | the other bass | no bass |
| kick, shaker | themselves | – | dropped |
| backbeat (2 and 4) | `rim`; `clap` in chip | the other one | dropped |
| fx (ticks, pops, entrance objects) | `blip` | `mallet` (short), `shaker` | skipped |
| riser (swells) | `riser` | – | skipped |

`cozy-score instruments` prints both feels' tables.

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
| `chip` | band-limited 25% pulse (harmonics up to 11 kHz, rolled off as e^{−k/14}), 0.3% vibrato at 6 Hz from 0.12 s, 5 ms attack, `0.7 + 0.3e^{−t/0.08}`, 50 ms release; 0.14 | +0.05 | 0.28 |
| `synth-pluck` | the guitar's partials decaying twice as fast (`e^{−t(1.6 + 0.9k/2)}`), 0.6 s; 0.28 on the beat, 0.2 off it | −0.3 | 0.22 |
| `blip` | `sin φ + 0.2 sin 2φ` gliding up from 0.72 f (τ = 18 ms), `e^{−t/0.035}`, 0.14 s; 0.2 | +0.35 | 0.25 |
| `clap` | noise band-passed 0.9–4.5 kHz, three 6 ms bursts 11 ms apart and a 70 ms tail, 0.25 s; 0.3 | −0.1 | 0.15 |
| `riser` | noise high-passed at 2.5 kHz, `(t/dur)^2.2` (0.07), or reversed `(1 − t/dur)^2` (0.06) | centre | 0.12 |

If dropped roles mean a section can't reach its energy (for example a
`reveal` with only `pad`), the tool renders what is allowed and warns.
Changing the instruments never changes the notes: melodies are generated
before the allow-lists are applied.

## How it works

The pipeline is a chain of pure stages: parse → validate → timing
(tempo and snapping) → arrangement (note events by role) → events (timed
notes) → instrument resolution → synthesis → mix → master → encode.

- **Key and tempo.** Spring and summer are written in D major and chip in C
  major. `--key` transposes everything into another major key by the
  nearest octave: up or down, whichever keeps more voicing roots between
  MIDI 33 and 47; any other root folds by an octave into that range, and the
  lead is generated where it lands between MIDI 72 and 90. Without `--bpm`,
  the tempo is searched in 0.1 BPM steps within the feel's range: a plan
  without events fits the section boundaries to the beat grid, and a plan
  with events fits boundaries and event times to the 1/16 grid (see
  [How events fit the music](#how-events-fit-the-music)). Each boundary
  snaps to the nearest beat; the offsets go in `cue.json`, with a warning for
  any boundary that moves more than 0.25 s. The audio is exactly as long as
  the plan, plus `--tail`.
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
  - The range is D5–E6 (D5–F#6 in `reveal`); C5–E6 in chip.
- **Transitions,** decided by the energy change at each boundary:

  | Energy change | What happens |
  |---|---|
  | +2 or more | 6-note motif run up into the boundary, plus a shaker crescendo |
  | +1 | 3-note pickup |
  | −1 or less | drums rest for the last beat, plus a descending 3-note figure |

  Going into `title-sting` or `reveal`, or out of `build`, always ends on a
  hit. A transition belongs to the section it leads into and uses that
  section's instruments. A run event landing on the boundary replaces it.
- **Mix and master.**
  - Constant-power panning, then a seeded 1.9 s stereo convolution reverb
    (wet 0.9).
  - A 0.4 s fade-in and the fade-out: `--fade-out`, or `min(3 s, 0.1 ×
    length)` (a 7 s sting gets 0.7 s). Untouched array plans keep the
    original `min(3 s, the last section)`.
  - High-pass at 35 Hz and a +2.9 dB shelf above 5 kHz.
  - `tanh` saturation on the peaks.
  - Normalisation to −14 LUFS integrated (BS.1770-4, implemented here) with
    a −1 dBFS sample-peak ceiling. If the ceiling sets the gain, the tool
    says so.
  - With `--tail`, the gain, saturation and section trims are those of the
    same render without the tail, so the two match sample for sample up to
    where the shorter one's fade begins.
- **Section levels.** Each section is trimmed toward its energy target.
  - The trims are calibrated by measuring the *unrestricted* arrangement
    (every instrument allowed, events included), so they depend only on the
    plan, feel and seed.
  - A restricted section keeps the trim of its full version and comes only
    as close to its target as its instruments allow.

## Determinism

The same plan, instruments, seed, feel and tool version give a
byte-identical WAV:

- **One seeded PRNG.** All randomness comes from one xoshiro256** generator
  in the crate. Independent streams are forked by label (section, role,
  note id, `events/<index>/<what>`), so restricting one section can't shift
  another section's noise, and adding an event can't change anything else.
- **No platform maths.** All `sin`, `exp`, `tanh`, `pow`, `log`, `hypot` and
  `atan2` calls in the signal path and the pixellab port use the pure-Rust
  `libm` crate. `clippy.toml` forbids the `f64` methods that call the
  platform library, as well as `mul_add`.
- **Deterministic FFT.** The convolution reverb's FFT is part of the crate,
  with `libm` twiddles, and doesn't depend on CPU feature detection.
- **Fixed order.** No `HashMap` iteration. Sections render in parallel but
  are summed in a fixed order, and each note's audio depends only on its
  id.
- **Pinned toolchain.** The toolchain and dependencies are pinned
  (`rust-toolchain.toml`, exact versions, `Cargo.lock`).
- **Old plans.** Plans that use none of the newer features render exactly
  as before: the spring and summer feels keep their nine instruments by
  default, and the new roles and instruments never appear in them.

`tests/determinism.rs` renders `tests/fixtures/reference-short.json` and
compares it against `tests/fixtures/reference-short.sha256`, and does the
same for `plans/startup-sting.json` and `plans/entrance-hero.json`
(`tests/fixtures/startup-sting.sha256`, `entrance-hero.sha256`). CI runs it
on Linux, macOS and Windows. When the sound changes on purpose, update the
hashes deliberately:

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
  tempo search and snapping, note values, voicings (both feels), keys,
  melody constraints, contours, the RNG, the FFT, convolution, filters and
  the loudness meter. A 1 kHz sine at −20 dBFS on one channel reads
  −23.0008 LUFS at 44.1 kHz: the mean square is −23.0103 dB, the
  K-weighting gain at 1 kHz is +0.7005 dB, and the offset is −0.691.
- **Integration tests:**
  - `tests/allow_lists.rs`: allow-list overrides, fallback order, dropped
    roles, empty lists, melody invariance and transition selection.
  - `tests/events.rs`: plan forms and every event field, bad combinations,
    run timing and landing, contours and gain ramps, tick patterns and
    accents, the tempo search, quantizing, transitions, the new roles'
    fallbacks, keys, fade-out and tail.
  - `tests/pixellab.rs`: parity with pixellab on every fixture case, every
    scene source giving the same schedule, pixellab's error messages, each
    move's sync point, exits, clustering and the rate cap, `together`, pitch
    modes, `checker`, frame alignment and sequence import.
  - `tests/detect.rs`: motion classification and the drafted plan on
    synthetic data shaped like the reference clip, and (with ffmpeg) the
    whole command on a generated video.
  - `tests/determinism.rs`: the three golden hashes.

Rendering the 60 s reference plan takes about 3 s on a 4-core machine
(about 5 s with `--stems`); the 7 s sting about 1 s. Memory grows with
length: a 10-minute plan peaks at roughly 2.2 GB.

To compare a variant against a base render sample by sample, reuse the
base render's master gain:

```bash
cargo run --release -- render --plan base.json --out out/base
```

```bash
cargo run --release -- render --plan variant.json --out out/variant --gain-from out/base/cue.json
```

## Notes on interpretation

- Speech-heavy types never carry the lead, except where the type table
  explicitly gives one: `intro` has a few long soft lead notes and
  `recap-cta` brings the lead back at low gain. `cold-open-hook`, `explain`
  and `sponsor` have no lead.
- When mallet fills the picking role it plays the same notes at 0.6× gain,
  with its own short decay instead of the guitar's long ring.
- The plan's last end time is the length of the video. Internal boundaries
  snap to beats; the final section plays to the end of the audio.
- `title-sting` uses a I–IV progression (`D G`, or `Cmaj7 Fmaj7` in chip)
  so its signature motif always fits. `cold-open-hook` plays a fragment of
  the same motif on its first tonic bar.
- The A/C# voicing's bass note (C#2, 69 Hz) is taken from the voicing
  table, so it is the one root just below the 73–123 Hz range.
- **Chip arpeggios sound an octave above their table,** as the guitar's
  always have (the tables are written before the picking role's octave
  shift). Played as written, the synth-pluck sat on top of the pad at
  150–400 Hz and muddied the groove.
- **A four-note pad chord plays each voice at 3/4,** so a 7th chord is as
  loud as a triad.
- **Clap and riser noise is uniform** (−1 to 1), as a prototype's
  `Math.random()` would be; the older instruments keep their Gaussian noise.
- **Event sounds are not humanised**: they sit exactly on their times.
- **`hit` chords ask for mallet** but go through the motif role, so they
  fall back like any motif note. The `land` sparkle and the `pop` blip use
  the fx role.
- **A `riser` event** exists so `from-pixellab` can mark fades and dips;
  it's a plain noise swell.
