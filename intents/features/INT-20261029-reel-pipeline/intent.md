# INT-20261029-reel-pipeline: Intent

| | |
|---|---|
| **Status** | Approved |
| **Type** | Feature |
| **Owner** | J. Law Cordova |
| **Created** | 2026-10-29 |
| **Approved** | 2026-10-29 |
| **Spec** | [spec.md](spec.md) |

## Problem

Making one video takes several tools, each with its own timing, and nothing
keeps them in sync. The workflow today:

1. Write a script and a storyboard: the general idea, plus a rough
   layout/rig for each pixellab scene.
2. Record the storyboard parts that are J. Law's talking head: no overlays,
   just J. Law in the middle of the frame, speaking. Some script parts are
   generated with ElevenLabs instead.
3. Adjust the timespans of the talking-head video and the audio.
4. Put the high-fidelity pixellab scenes into the storyboard slots meant for
   them.
5. Replace some storyboard slots with high-fidelity animations made outside
   pixellab, such as quote animations and logo intros and outros.
6. Generate music with `cozy-score` to match the edit so far.
7. Render everything.

Every step after recording is done by hand in an editor. Any change to a
timespan means re-checking the scenes, the animations and the music. The
music is the hardest part: `cozy-score` needs a plan of
`[video-type, timespan]` sections that has to be copied out of the edit by
hand.

The repository's goal is "tooling to generate video content where the
script, video and music are in sync". So far only the music has a tool.

## Intent

Build **`pixelreel`**, a Rust command-line tool in `cli/` that runs this
whole workflow from one project folder, with the script as the source of
truth.

- **One source of truth.** A Markdown storyboard holds the script and the
  beats. Each beat is tagged with one of `cozy-score`'s video types, so the
  music plan comes out of the edit and never has to be retyped.
- **Picture and voice are kept apart.** Each beat has a picture (talking
  head, pixellab scene, outside animation or placeholder card) and a voice
  (J. Law's recorded lines or ElevenLabs lines). The voice decides the
  beat's length. This makes L-cuts and J-cuts possible without special
  handling.
- **Timing comes from the words.** Recordings are aligned to the script
  word by word. Cut adjustments are stored as text. Pixellab scene events
  can fire on a marked spoken word.
- **Lock, then fill.** Everything is estimated until the cut is locked.
  After the lock, scenes, animations and music fit exact times.
- **YouTube-ready output.** The tool produces the final video, captions
  built from the script, a chapter list, and a check against YouTube's
  rules for chapters, end screens and loudness.

## Outcomes

- A storyboard gives an animatic and a music sketch before anything is
  recorded.
- Recording, ElevenLabs lines, cut adjustments, scenes, animations and
  music all come together into one rendered video with no manual
  timeline editing.
- Changing a line or a cut shows exactly what is now out of date, and
  rebuilds only that.
- Renders can be repeated exactly: raw media is never modified, and every
  decision lives in diffable text.

## Non-goals

- Replacing pixellab, ElevenLabs or the outside animation tools. `pixelreel`
  takes in what they produce.
- A graphical editor or timeline UI.
- Overlays on the talking head: titles, lower thirds, zoom punches.
- Vertical Shorts cut-downs. These may come in a later intent.
- Judging whether a video is good. As with `cozy-score`, a person still
  watches the preview before publishing.

## Success criteria

- The sample devlog in the spec, about 7:40 long, can be produced end to
  end with `pixelreel` commands only.
- `pixelreel check --youtube` passes on that render: chapters, end screen,
  −14 LUFS, captions.
- The music plan `pixelreel` generates is accepted by `cozy-score` with no
  edits.
- Re-running any command with no input changes rebuilds nothing.
