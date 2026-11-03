# Client performance fixes

This implements changes following the [baseline audit](performance-audit.md). The original
[reference results](performance-audit-results.txt) are preserved. The optimized rerun is in
[performance-fixes-results.txt](performance-fixes-results.txt).

## Measured changes

Same macOS arm64 host, optimized `perf-audit` profile, same original eight audit cases and
fixtures. Values below are medians of 21 samples, in microseconds per operation. Small cases
use batch means, as described in the original audit. They are not game-frame percentiles or
physical input latency measurements. A ninth case measures the new asynchronous skin handoff.

| Operation | Before (us) | After (us) | Interpretation |
| --- | ---: | ---: | --- |
| Default sound set at 48 kHz, warm | 15,710 | 0.250 | Reuses immutable samples, including medal sounds |
| 10,000 notes, one very slow future note | 201.746 | 5.054 | About 40x less CPU work; same 61 shapes |
| Same map with Multiverse | 1,653.888 | 41.708 | About 40x less CPU work; same 452 shapes |
| Search over 50,000 synthetic entries | 10,363 | 6,252 | Normalization and label formatting removed from queries |
| Title sort over 50,000 entries | 13,389 | 8,271 | Borrowed keys and one folder lookup per entry |
| First offset lookup, empty table, 8 MiB map | 7,561 | below timer resolution | No file read or hash when no offsets exist |
| Adaptive offset hint after 100,000 hits | about 19 | 0.967 | Only the latest 200 errors are transformed |

The old mutex probe around a sound rebuild went from 15,690 us median to 0.083 us. With the
cached operation it often finishes before the other thread attempts the lock. This means the
specific long critical section disappeared; it is not evidence of sub-microsecond input latency.

Controls still cost roughly the same: directly synthesizing medal sounds takes 13.50 ms,
loading a 2048 px skin image synchronously takes 67.68 ms, and directly calculating stars for
10,000 notes takes 3.48 ms. The fixes avoid repeating synthesis and move skin/submission
preparation away from the state mutex; they do not claim faster PNG decoding or star algorithms.

## What changed

- **Sounds (A1):** built-in sounds are cached by device sample rate. Each sound is immutable
  shared data. Custom don, kat, finish and combo-break replacements retain the old mixing
  rules; unrelated sounds reuse the cached samples. Cold initialization still has a cost.
- **Skins and uploads (A2):** one worker reads skins, decodes images, assembles the atlas and
  prepares sounds. Rapid requests coalesce and revision checks reject stale results. Ordinary
  map folders without images reuse the effective prepared skin. Explicit skin reloads bypass
  that cache so replacing a skin's files still works. The previous skin remains until ready.
  GPU atlas/background uploads happen on the render thread outside the shared state lock.
- **Library UI (A3):** entries prepare lowercase search/title keys when metadata is loaded;
  replacing an entry refreshes the keys. Title sorting borrows keys and builds group IDs,
  values and ordering in one pass rather than looking up each folder three times. Filtering
  and sorting still run synchronously; this is a reduction, not elimination of large-library
  pauses. The 50,000-entry title-sort case remains about one 120 Hz refresh.
- **Gameplay drawing (A4):** an index of 128-note blocks provides conservative local velocity
  bounds. A far-away slow note widens only its block's ordinary-note scan. Long rolls, swells,
  fake sliders and Liar retain the original conservative scan. Global bounds remain for special
  objects and barlines; no fixed time cap hides slow notes.
- **Offset lookup (A5):** an empty offset table returns immediately. Each frame resolves the
  map offset once and shares it between preview and play timing. Hashing with a nonempty
  table remains synchronous on its first use; changing offset readiness during a song would
  require a separate lifecycle change to preserve judging and preview timing.
- **Online results (A6):** the submission worker now calculates stars and MD5 before sending
  the result, with the replay's rate, mods, scroll speed, seed and Half rule. Replay persistence
  is still synchronous; its storage latency was not measured in the baseline.
- **Surface configuration (A7):** surface, resize and present-mode requests are taken under a
  short lock; GPU configuration runs after releasing it. Granted present mode is published
  afterward. GPU/backend duration itself was not measured.
- **Settings (A8):** audible volume changes apply immediately, persistence waits for 250 ms
  without further changes, and clamped turns schedule no write. Config snapshots go to one
  background writer which coalesces queued writes, preserves unknown keys and flushes after
  the final settings snapshot on normal shutdown. Serialization still occurs on the caller.
- **Offset hints:** transform only the final 200 hits, with the same adaptive/simple estimates.

The new 2048 px skin case measures request submission at 7.042 us median / 19.583 us p95 and
shared-asset handle copies at 0.125 us median / 0.334 us p95. Worker completion takes 69.15 ms
median / 73.83 ms p95, including polling at 1 ms intervals. Preparation has moved off-thread;
these numbers exclude native GPU upload and are not a measurement of the entire state lock.

## Validation and limits

Regression tests compare every shape's bytes against the original scan for all 76 individual
modifiers plus no mods, at five map times, including Multiverse, a very slow note, long rolls,
swells and fake objects. Existing gameplay tests additionally exercise judging and replay rules.
Sound tests check sample sharing, sample-rate separation, custom mixing and cache isolation.
Worker tests check stale-request rejection, plain-map reuse and explicit reloads. Settings tests
check the latest snapshot, unknown-key preservation, flush completion and clamped-volume saves.
The audit checks offset-estimate equivalence against a bounded history.

Checks run on macOS:

- `cargo test --profile perf-audit --locked --offline --all-targets -- --test-threads=1`:
  396 passed, 10 ignored (nine opt-in measurements and one existing ignored test).
- The nine opt-in optimized audit tests: all passed, with 68 recorded observations.
- `cargo clippy --all-targets --locked --offline -- -D warnings`: passed.
- `git diff --check`: passed.

Native GPU pacing, actual keyboard-to-sound latency, Linux/Windows backend behavior and audio
callback deadlines still require platform runs. No present-mode, vsync or audio-buffer policy
was changed. Whole-library row layout, pinned expiration scans, replay/status persistence,
nonempty-table first hashes and realtime audio queue bounds remain follow-up work; this change
does not claim to eliminate every source of latency.
