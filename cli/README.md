# pixelreel

A command-line tool that turns a Markdown storyboard into one finished
video. It brings together:

- talking-head takes;
- ElevenLabs lines;
- [Pixel Lab](https://github.com/jlawcordova/jlawcordova-lab) scenes;
- outside animations (logo stings, quote cards, end screens);
- music from [`cozy-score`](../tooling/cozy-score).

The storyboard is the source of truth. Every timing decision is stored as
text in the project folder, and raw media is never modified.

The design record is in
[`intents/features/INT-20261029-reel-pipeline/`](../intents/features/INT-20261029-reel-pipeline/spec.md)
(intent, spec, plan).

> **Watch before you publish.** pixelreel measures timing and loudness
> and checks YouTube's rules. It can't tell whether the video is good.

## Install

The toolchain is pinned in `rust-toolchain.toml`; rustup installs it on
first use.

```bash
cd cli
cargo build --release
./target/release/pixelreel --help
```

It also needs these programs:

| Program | For |
|---|---|
| `ffmpeg`, `ffprobe` | Every render, `take add`, `asset add`, `vo gen` |
| `whisper-cli` ([whisper.cpp](https://github.com/ggml-org/whisper.cpp)) and a model | `take add` |
| `pixellab` (Pixel Lab) | `scene:` beats, `scene list`, `scene preview` |
| `curl` and `ELEVENLABS_API_KEY` | `vo gen`, `animatic --tts` |

## A video, start to finish

```bash
pixelreel init farm-devlog --size 1920x1080 --fps 30 --feel spring --document ~/Art/farm.pixellab
```

```bash
cd farm-devlog
```

1. Write `storyboard.md` and check it. `scene list` shows what's in the
   Pixel Lab document. Register the channel's animations, then render a
   rough cut and a first music sketch:

   ```bash
   pixelreel board
   pixelreel scene list
   pixelreel asset add ~/channel/logo-intro.mov --id logo-intro
   pixelreel asset add ~/channel/end-screen.mov --id end-screen --fit hold-last
   pixelreel animatic --tts
   pixelreel score render --seeds 7
   ```

2. Generate the ElevenLabs lines:

   ```bash
   pixelreel vo gen --takes 3
   pixelreel vo pick quote 2
   ```

3. Record, then align the takes and cut:

   ```bash
   pixelreel take add takes/A001.mp4 takes/A002.mp4
   pixelreel cut tighten --max-gap 0.3 --keep-breath 0.12
   pixelreel cut show hook
   pixelreel cut set hook.2 --out -0.2
   pixelreel cut lead hook-tease 0.4
   pixelreel cut lock
   ```

4. Finish the scenes in Pixel Lab, then check them in place:

   ```bash
   pixelreel scene preview hook-tease
   ```

5. Render the music for the locked edit and pick a seed:

   ```bash
   pixelreel score render --seeds 3,7,11
   pixelreel score pick 7
   ```

6. Render and check:

   ```bash
   pixelreel render --preview
   pixelreel check --youtube
   pixelreel render --final --size 3840x2160
   ```

`pixelreel status` shows, at any point, what's done and what's out of
date, beat by beat.

## The storyboard

```markdown
---
title: I Built a Farm That Runs Itself
---

## hook | cold-open-hook | face
chapter: Can a farm run itself?
Thirty days ago this field was empty.

## hook-tease | cold-open-hook | scene:farm-night
cue lights-on: lamp-1 on
And I haven't touched it in a week{#lights-on}.

## title | title-sting | asset:logo-intro

## demo | visual-demo | scene:field-build
fit: hold
VO(sprout): Day one. Soil quality: questionable.

## quote | reflect | asset:quote-card
under: scene:farm-night
VO(sprout): "The best fertilizer is the farmer's footsteps."

## outro | outro-end-screen | asset:end-screen
length: 20s
```

- **Heading:** each beat starts with `## <id> | <type> | <picture>`.
  - `type` is a `cozy-score` video type.
  - `picture` is `face`, `card`, `scene:<name>` (a Pixel Lab scene, or
    `scene:sequences/<name>`) or `asset:<id>`.
- **Lines:** plain lines are yours. `VO(<voice>):` lines are generated
  with ElevenLabs.
- **Cues:** `{#name}` after a word marks a cue.
- **Directives** go right after the heading:

| Directive | Meaning |
|---|---|
| `chapter: <title>` | Starts a YouTube chapter |
| `length: <time>` | A beat with no lines: its length. With lines: a minimum |
| `under: face\|scene:<name>` | What shows beneath an asset with alpha |
| `fit: loop\|hold` | A scene: loop it over the beat, or play one period and hold |
| `from: <time>` | A scene: where on its clock to start |
| `entrance: true` | A scene: open with its block build-up |
| `cue <name>: <item> <state>` | A scene: at `{#name}`, the Pixel Lab item enters that state |

## Commands

Every command takes `-C <project>` (the default is `.`). All errors are
reported at once, as `error: …` lines, and the exit code is 1.

| Command | Does |
|---|---|
| `init <dir> [--size --fps --feel --document]` | Make a project folder |
| `board [--json]` | Check the storyboard; show beats with estimated (or locked) times |
| `animatic [--tts]` | A rough cut at 960×540 from estimates: cards, Pixel Lab scenes, assets; music if picked |
| `vo gen [beat…] [--voice --takes N --force]` | Generate ElevenLabs lines, cached by text and voice settings |
| `vo pick <beat>[.<n>] <take>` | Choose a VO take |
| `take add <file…>` | Transcribe takes and find each line's reads (the last complete read wins) |
| `cut show\|tighten\|set\|take\|lead\|lock\|unlock` | Timing edits in `cuts.yaml`, and the lock |
| `scene list` | Scenes and sequences in the Pixel Lab document |
| `scene preview <beat>` | One beat with its voice, to `build/preview/<beat>.mp4` |
| `asset add <file> --id <id> [--fit trim\|hold-last\|speed] [--alpha] [--audio keep\|mute]` | Register an outside animation |
| `score plan\|render [--seeds a,b --instruments --stems]\|pick <seed>` | Music from `cozy-score` |
| `render --preview\|--final [--size WxH] [--codec h264\|prores]` | The video, captions, chapters and a report, in `build/render/<mode>/` |
| `status [--json]` | What's done and what's out of date |
| `check --youtube [--strict --json]` | Chapters, end screen, loudness, captions, media, placeholders, music hits, hook |

## The project folder

```
pixelreel.yaml     # settings: size, fps, feel, Pixel Lab document, voices, mix
storyboard.md      # script + beats
cuts.yaml          # timing edits (pixelreel cut)
picks.yaml         # chosen VO takes and music seed
takes.yaml         # recordings, in the order they were added
takes/             # raw recordings, never modified
vo/                # ElevenLabs audio + word timings
assets/assets.yaml # registered outside animations
build/             # derived; safe to delete
```

## How it works

- **Timing.**
  - Voices are laid end to end, so two voices never overlap.
  - A beat's picture starts where its voice starts, or `lead` seconds
    later for a J-cut.
  - Beat boundaries snap to frames. Line and word times keep millisecond
    precision for the audio.
  - Between two lines read back to back in the same take, a pause up to
    `max_gap` is kept. Any other join is a hard cut.
- **Pixel Lab.**
  - Scenes are rendered with `pixellab art export --format frames --fit W,H`,
    at the largest whole-pixel scale that fits.
  - For cues, pixelreel works on a copy of the document in
    `build/pixellab/`. It lengthens the scene's motion loop if the beat
    runs past it, then keys each item's state at the cue word's time. The
    owner's document is never written.
- **Video.** Each beat is rendered once, with the final codec settings,
  and cached by a hash of what it shows. Beats are joined with stream
  copy.
- **Mix.**
  - Voice, asset audio and music are mixed at 48 kHz.
  - The music ducks by `duck_db` under speech.
  - The master is brought to −14 LUFS (BS.1770-4, with `cozy-score`'s
    K-weighting), with true peak at or under −1 dBTP, measured at 4×
    oversampling with a lookahead limiter.
- **Captions** come from the script text on the aligned word times, so
  they have no transcription typos.

## Development

```bash
cargo fmt --check
```

```bash
cargo clippy --locked --all-targets -- -D warnings
```

```bash
cargo test --locked --release
```

`tests/pipeline.rs` runs every command on a generated project, with fake
`whisper-cli` and `pixellab` scripts (`tests/fakes/`) and a local fake
ElevenLabs. It needs `ffmpeg` with libx264 and libmp3lame. Set
`PIXELREEL_KEEP=1` to keep its output in the temp folder.
