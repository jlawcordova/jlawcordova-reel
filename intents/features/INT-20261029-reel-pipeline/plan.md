# Plan: pixelreel, the reel pipeline CLI (from intent.md 2026-10-29)

| | |
|---|---|
| **Status** | Written for implementation. The owner asked to proceed with implementing on 2026-10-29 |
| **Intent** | [intent.md](intent.md) |
| **Spec** | [spec.md](spec.md), including its 2026-10-29 amendment (scenes come from Pixel Lab) |

## Files that change

All new except where marked.

- `intents/features/INT-20261029-reel-pipeline/spec.md` (changed): the
  amendment that answers §13.
- `intents/features/INT-20261029-reel-pipeline/plan.md`: this plan.
- `.github/workflows/pixelreel.yml`: fmt, clippy and tests for `cli/`.
- `cli/Cargo.toml`, `cli/Cargo.lock`, `cli/rust-toolchain.toml`,
  `cli/.gitignore`, `cli/README.md`.
- `cli/src/main.rs`: clap commands, output and exit codes (spec §7.1).
- `cli/src/lib.rs` and these modules:

| Module | Spec |
|---|---|
| `time.rs`: parse and format times | §6.2 |
| `config.rs`: `pixelreel.yaml` and its defaults | §6.1 |
| `storyboard.rs`: parser and validation | §6.2 |
| `files.rs`: `cuts.yaml`, `picks.yaml`, `takes.yaml`, `assets/assets.yaml` | §6.3, §6.4, §6.7 |
| `project.rs`: project paths, loading, `init` | §5, §7.2 |
| `hash.rs`: sha256 and the media hash cache | §10 |
| `proc.rs`: running external programs and reporting them clearly | §4 |
| `media.rs`: `ffprobe` and `ffmpeg` helpers | §9 |
| `timeline.rs`: estimates, the locked timeline and `timeline.json` | §6.5, §7.3, §8 |
| `align.rs`: word normalisation, alignment and read detection | §7.7 |
| `takes.rs`: `take add` and the `whisper-cli` adapter | §7.7 |
| `vo.rs`: ElevenLabs through `curl`, the cache and picks | §7.5, §7.6 |
| `score.rs`: the plan, `cozy-score` renders and picks | §7.13–7.15 |
| `pixellab.rs`: the Pixel Lab adapter (list, show, copy, keys, export) | §6.6, §9.1 |
| `assets.rs`: `asset add` | §7.12 |
| `audio.rs`: WAV I/O, buses, ducking, loudness and true peak | §9.2 |
| `render.rs`: beat pictures, concatenation, mux, preview and final, `animatic`, `scene preview` | §7.4, §7.11, §7.16, §9.1 |
| `captions.rs`: `captions.srt` and `chapters.txt` | §7.16 |
| `status.rs` and `check.rs` | §7.17, §7.18 |

- `cli/tests/pipeline.rs`: the end-to-end integration test, using fake
  `whisper-cli`, `pixellab` and ElevenLabs.

## Order of work

There is one slice, on the session branch `claude/sweet-faraday-kd6odz`,
which already carries the intent and spec in
[jlawcordova/jlawcordova-reel#2](https://github.com/jlawcordova/jlawcordova-reel/pull/2).
One runner does the work in one pass, and the session is held to that
branch, so there's no separate main-session branch. The owner merges the
PR into `main`.

```
main ── claude/sweet-faraday-kd6odz (PR #2: intent, spec, plan, cli/) ──▶ main (owner merges)
```

1. Write the spec amendment and this plan.
2. **M1:** the crate scaffold, `time`, `config`, `storyboard`, `files`,
   `project`, the estimated timeline, `init`, `board`, `score plan`,
   `score render` and `score pick`, plus the CI workflow. Unit tests for
   the parser (every error at once), times, estimates and the plan merge,
   with Appendix A's golden plan.
3. **M2:** `vo gen` and `vo pick` (`curl`, the cache, words from the
   character alignment); `take add` (ffmpeg extraction, whisper JSON,
   alignment, the report); `cut show`, `tighten`, `set`, `take`, `lead`,
   `lock` and `unlock`; `status`. Unit tests for alignment and read
   detection, joins, lead, frame rounding, and a timeline built from
   fixtures.
4. **M3:** `asset add`, the Pixel Lab adapter, `scene list`,
   `scene preview` and `animatic`. Unit tests for the Pixel Lab arguments
   for each scene setting, and for cue keys and the loop length.
5. **M4:** audio buses, ducking and loudness; beat pictures; concatenation
   and mux; `render --preview` and `--final`; captions and chapters;
   `check --youtube`. Unit tests for caption splitting, chapters, each
   YouTube check, the duck envelope and true peak.
6. The end-to-end test (`tests/pipeline.rs`) and `cli/README.md`.

## Workflow and reviews

- **Who runs it:** the main session runs every step, in one context, on
  `claude/sweet-faraday-kd6odz`. No subagents: the steps share types (the
  storyboard, the timeline) and build on each other.
- **Owner reviews:** the PR. The owner listens to and watches a real
  render, which no test can judge (intent, non-goals).
- **Independent review:** none planned. The owner can ask for one on the
  PR.

## Risks

| Risk | Handling |
|---|---|
| The real `pixellab`, `whisper-cli` and ElevenLabs aren't available in the build session (building Pixel Lab from source isn't allowed here, and there's no API key) | Each adapter is a thin wrapper around documented commands: `art export`, `scene show`, `document copy`, `scene motion set`, `scene key set`, whisper's `-oj` JSON, and the with-timestamps endpoint. Fakes cover them in tests, and the PR says plainly that the real programs weren't run |
| Riskiest step: the timeline (joins, lead, frame rounding), which everything downstream reads | Unit tests per rule, and a golden `timeline.json` hash in the end-to-end test |
| Audio clicks or level jumps at edits | 10 ms fades at every segment edge; the duck is a smoothed envelope over known voice spans, not a detector |
| `ffmpeg` filter differences between versions | Use only long-standing filters (`trim`, `setpts`, `scale`, `pad`, `tpad`, `overlay`, `drawtext`, `concat`); the end-to-end test runs on CI's `ffmpeg` |
| No fonts for `drawtext` on a machine | Pass a font file found with `fc-match`; warn and leave text out if none is found |
| `cozy-score`'s pinned dependency versions | Pin the same exact versions in `cli/Cargo.toml` |
| Other options considered and not chosen | `ureq` or `reqwest` for HTTP: rejected, because they bring TLS crates and `curl` is everywhere. FFV1 intermediates: rejected for their size at 4K; beats encode once with final settings and concatenate with stream copy. Our own compositor: rejected now that Pixel Lab exports frames |

## Proof

Run from `cli/`:

```bash
cargo fmt --check
```

```bash
cargo clippy --locked --all-targets -- -D warnings
```

```bash
cargo test --locked --release
```

The tests include the end-to-end run: a fixture project with generated
takes, a fake whisper, a fake Pixel Lab and a local fake ElevenLabs. It
goes through `init`, `board`, `vo gen`, `take add`, `cut lock`,
`score render`, `score pick`, `render --preview`, `render --final` and
`check --youtube`. It asserts the duration, frame count, chapters,
captions and loudness (−14 ± 0.5 LU).

## Departures

- **The fakes are committed scripts.** `cli/tests/fakes/whisper-cli` and
  `cli/tests/fakes/pixellab` are files in the repo, and the test points
  `pixelreel.yaml` at them. The plan had the tests write them.
- **The master loop iterates.** The master adjusts gain and limiting in a
  loop, and tightens the limiter's sample ceiling until the true peak
  fits. A single pass came in up to 1 LU under target on peaky speech.
- **`check --json`** was added, matching `board`, `status` and `cut show`.
