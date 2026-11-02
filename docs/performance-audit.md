# Client performance audit

Audited base: `393e1d4` (`worktree-taiko-client`). This report records the original measurements;
subsequent fixes and their results are in [performance-fixes.md](performance-fixes.md).
The scope is the desktop client's render/input handoff,
CPU gameplay drawing, library UI, asset preparation, and audio control paths.

The largest demonstrated problem is **long work inside the same `State` mutex that keyboard
events require**. A fast GPU does not remove these pauses. Keyboard events are timestamped
when the window handler processes them, before taking that mutex. This accounts for that
particular lock wait in judging; it does not compensate earlier event-queue delays or make
a hitsound command reach the audio thread before the mutex is released.

## Reproduce

```sh
cargo test --profile perf-audit --locked --lib perf_audit:: -- --ignored --nocapture --test-threads=1
```

The [harness](../src/perf_audit.rs) calls the real client functions, without a window or audio
device. It generates its own maps, library entries, and PNGs in temporary directories, and
does not benchmark the player's music library. `perf-audit` enables optimized application
code, disables LTO, and uses 16 codegen units. Unoptimized test measurements are not used.

Results were collected on this macOS arm64 host with Rust 1.99.1. Each case has three warm-up
calls and 21 timed samples, except cold-per-entry hashes and preparation/contention probes,
which use 21 independent samples. For small operations a sample is a batch mean: 100 calls
for offset hints/expiry, 10 for drawing a playfield, 5 for drawing a library, and 1,000 for
cached hashes. **The reported p95 is a percentile of these samples, not a p95 of game frames.**
Skin/asset destruction is included in the skin/sounds measurements; map cloning is excluded
from game preparation. File hashes use warm filesystem caches. Numbers are CPU time and
are not measurements of GPU time, display latency, callback deadline misses, or physical
key-to-speaker latency. Synthetic stress cases are labeled below.

Full measured values: [performance-audit-results.txt](performance-audit-results.txt).
At 120 Hz one refresh is 8.33 ms; at 240 Hz it is 4.17 ms. Priorities reflect the affected
scenario, not a claim that every map or frame pays the listed cost.

## Findings worth fixing first

| ID | Scenario and evidence | Median / p95 CPU time | Priority |
| --- | --- | --- | --- |
| A1 | Rebuilding sounds at 48 kHz when changing a skin | 15.71 / 15.84 ms | P1: input/transition stalls |
| A2 | Loading one 512 px / 2048 px skin circle | 512 px: 5.43 ms median; 2048 px: 69.07 / 70.92 ms | P1: input/transition stalls |
| A3 | Search / title sort over 50,000 synthetic map entries | 10.36 / 13.39 ms medians; title-sort p95 17.55 ms | P2: large-library UI |
| A4 | One very slow future note in a 10,000-note map, with Multiverse | 0.017 ms normal vs 1.654 ms slow; slow p95 1.762 ms | P2: gameplay/gimmick maps |
| A5 | First offset lookup, despite an empty offset table: 1 MiB / 8 MiB map | 0.945 / 7.561 ms medians | P2: selection/first frame |
| A6 | Recalculating stars for a 10,000-note synthetic finished play | 3.55 / 3.64 ms | P2: result transition |
| A7 | Swapchain reconfiguration while holding the input mutex | Blocking path established by wgpu contract; duration not measured | P2: resize/mode transitions |
| A8 | Volume autorepeat persists config even when clamped/unchanged | Synchronous read/write path established; storage latency not measured | P2: input/UI I/O |

### A1: sounds are regenerated, including unrelated procedural sounds

[`load_skin`](../src/lib.rs#L3582) calls `Engine::set_hitsounds`, which synchronously calls
[`hitsound_set`](../src/audio.rs#L808). Even a custom don/kat change generates the medal sounds,
screech, and chime again. The five deterministic medal sounds alone measured **13.36 ms**.
The same rebuild is performed when following a map skin in `build_frame`.

The render thread calls `build_frame` while holding `State`; the window thread needs that
same mutex before calling `State::key`, and the audio command is sent only inside the latter
path. A contention probe holding the actual `State` mutex around this exact sound rebuild
measured **15.69 ms median / 16.13 ms p95** waiting to acquire it from another thread. This
probe isolates mutex blocking; it is not an OS input-latency benchmark.

**Fix direction:** cache immutable built-in/procedural sounds per sample rate; prepare custom
sounds off the state lock and hand over an `Arc` when ready. Keep skin-part changes from
regenerating unrelated audio. Verify bitwise/sample equivalence and cache invalidation on
sample-rate changes before shipping.

### A2: PNG decode and atlas assembly happen under the same mutex

[`follow_map_skin` and `load_skin`](../src/lib.rs#L3582) run from `build_frame`. `skin::load`
reads images and assembles the atlas synchronously. A single generated circle gives 5.43 ms
at 512 px and 69.07 ms at 2048 px, even with warm caches. This is a CPU measurement, before
the extra GPU upload. The 2048 px case is a large-skin stress case, not a typical skin claim.

`renderer.set_sprites` and `renderer.set_background` also execute while the `State` guard is
held. Their native GPU/upload costs were not measured here and must not be added to the
CPU numbers as if observed. A1 and A2 can occur on the same skin transition; their individual
percentiles are not an end-to-end transition percentile.

**Fix direction:** prepare decoded assets/atlases in a worker, retain the old skin until ready,
and move GPU uploads outside the `State` critical section. Asset versioning must prevent an
old worker result from overwriting a newer selection.

### A3: searching and sorting block the event/render owner

[`Select::refilter`](../src/ui.rs#L381) scans the complete library, creates labels for matching,
and sorts/rebuilds groups. [`sort_visible`](../src/ui.rs#L493) also constructs lowercase title
keys. At 10,000 entries the search/title-sort medians are 2.02/2.49 ms; at 50,000 they are
10.36/13.39 ms. These generated libraries contain one difficulty per song. The search is a
matching text query, not an empty-query shortcut.

These operations run on search/sort actions and relevant status updates, not every gameplay
frame. `poll_set_status` is already guarded by `Scene::Select`; it is **not** an established
periodic mid-song stall. Nevertheless a keystroke that performs the full search consumes a
visible refresh on a large library and blocks other events behind the same lock.

**Fix direction:** cache normalized keys and separate filtering from unchanged sorting;
version/coalesce background queries for large libraries while keeping the old visible list.

### A4: one global minimum velocity expands every field's scan

[`Game::draw_field`](../src/game.rs#L2707) derives lookahead from the minimum velocity anywhere
in the map. One note at velocity `1e-5` near the end of a 10,000-note, 100 ms-spaced map widens
the scan of mostly ordinary offscreen notes. Plain drawing grows from **1.94 to 201.75 us**,
while emitted shape count changes only from **59 to 61**. Multiverse grows from **16.80 to
1,653.89 us**, with **436 to 452** shapes. The extra CPU work is principally scanning/culling,
not a proportional increase in visible output. This intentionally long/slow synthetic map
demonstrates scaling; normal maps do not necessarily incur it.

**Fix direction:** index notes by velocity/section or compute conservative per-section bounds,
sharing candidate discovery across fields where applicable. Preserve slow future notes,
negative/fake sliders, long rolls, warps and per-field reach: a fixed lookahead cap would be
a correctness regression. Existing drawing tests and new image/visibility comparisons are
required for an implementation.

### A5: an empty offset table still triggers synchronous lazy file hashing

[`refresh_play_offset`](../src/lib.rs#L4204) runs at the start of every frame. Its `map_offset`
evaluates `e.hash()` even when `map_offsets` is empty. [`Entry::hash`](../src/library.rs#L36)
reads and hashes the entire file on the first lookup, then caches it in `OnceLock`.

The benchmark calls the **actual `refresh_play_offset` with an empty table**: 66 us at 64 KiB,
0.945 ms at 1 MiB, and 7.56 ms at 8 MiB. Large files are stress cases, and these are warm-cache
numbers, not cold-storage latency. Later lookups are cached, so this is a first-selection
stall rather than a per-frame full read.

**Fix direction:** early-return for an empty offset map, compute hashes in loading workers,
and cache the selected map's resolved offset until the selection/settings change.

### A6: online submission preparation repeats star work on the result transition

[`Scene::Play` completion](../src/lib.rs#L8612) calls `stars::star_rating_played` synchronously
while constructing an online submission, before the submission worker starts. That function
reads/parses/converts the map and computes difficulty. The measured CPU time is 0.70 ms for
2,000 notes and 3.55 ms for 10,000. This path is conditional on a recorded, eligible online
play; it is not continuous input latency during a song.

Replay serialization/disk writes via `scores::save` also happen in the transition. Their
storage-dependent latency was not measured. **Fix direction:** prepare the complete submission
and replay persistence off the state lock; reuse correctly keyed difficulty results where
possible (rate, mods, seed, Half rule and scroll speed all matter).

### A7: swapchain reconfiguration may wait for the GPU while input is locked out

At the start of [`render_loop`](../src/lib.rs#L10163), the `State` guard remains alive during
`renderer.resize`, `set_surface` and `set_present`. They reach `Surface::configure`, whose
[wgpu 30 contract](https://docs.rs/wgpu/30.0.1/wgpu/struct.Surface.html#method.configure) says
reconfiguration waits for GPU idle. This is a demonstrated blocking dependency; its duration
on this Mac, Windows and Linux was not measured. Normal image acquisition is correctly outside
the lock already.

**Fix direction:** snapshot requests under a short lock, reconfigure/upload on the render
thread without that lock, then publish the resulting state in a short critical section.

### A8: volume autorepeat writes config even when the value cannot change

[`turn_volume`](../src/lib.rs#L4658) receives a `changed` flag from the volume overlay but
calls `cfg.save()` regardless. Thus holding Alt+Up/Down, or scrolling against a limit, still
does synchronous directory checks, config reads, unknown-key preservation and a full config
rewrite. These occur on the window-event thread with `State` held. Both subsequent key
event dispatch and the render owner can wait behind that I/O. This is a source-proven
redundant operation, not a measured disk stall on this SSD.

**Fix direction:** avoid persistence for unchanged values, update audible volume immediately,
and debounce/coalesce persistence outside the input lock. Flush pending changes on shutdown.

## Measured inefficiencies that are lower priority

- **Whole-library row layout each frame:** `Select::draw` materializes all logical rows before
  clipping to the viewport. With roughly the same 439 output shapes, 10,000/50,000 entries
  take 29/148 us. Cache cumulative row heights and virtualize layout if large-library menu
  CPU use matters. This is much smaller than A1/A2, not evidence of a 9 ms gameplay bottleneck.
- **Offset hints transform the entire hit history:** `offset_suggestion` allocates `corrected`
  for all hits, then uses only the last 40–200. At 10,000 hits the adaptive mode is a few us;
  at 100,000 it is 19 us, versus about 1 us with just the final 200. The harness verifies equal
  estimates in both modes for its fixtures. A bounded suffix/cache is straightforward, but
  100,000 hits is a stress case: do not present the speedup ratio as a major normal-game gain.
- **An unfinished long roll pins expiration scanning:** comparing the same midpoint twice,
  a synthetic map-wide overlapping roll makes `set_time` revisit already-judged notes.
  At 10,000 notes the expiry path costs 4.75 us versus a near-empty loop without the roll.
  `update` shares that expiration routine. This is real unnecessary work but small on this
  host; preserve overlapping-roll judging semantics before changing the cursor scheme.
- **Game construction on the render owner:** `Game::with_mods` takes 0.93 ms at 10,000 normal
  notes (map cloning excluded). This does not establish a severe ordinary-map bottleneck;
  complex mod preprocessing deserves separate stress measurements before prioritizing it.

## Code risks requiring additional measurements

- `Config::save` synchronously reads/preserves/writes the config; replay, pp backfill and dirty
  status-cache saves also perform storage work under `State`. This creates storage-dependent
  input stalls, especially on slow/custom data folders, but no filesystem tail latency was
  measured here. Coalesce snapshots and persist off-lock; do not drop writes on exit.
- `Mixer::render` drains an unbounded command queue before filling output, and returns old
  assets through a standard unbounded `mpsc` channel. It has no fixed upper bound on command
  work, and channel allocation/deallocation is not a realtime guarantee. This is an audio
  robustness risk, **not a measured underrun**. Realtime callback work should be bounded and
  allocation-free; see the [callback constraints](https://files.portaudio.com/docs/v19-doxydocs/writing_a_callback.html)
  for the same scheduling principle. The client uses CPAL, not PortAudio. Its audio stamp
  uses `try_lock`, so there is no evidence here of the callback blocking on that mutex.
- The frame's `now` is sampled before export work, the frame-state lock and uploads. Expensive
  work after that sample makes visuals stale; record clock-sample-to-submit and lock-wait
  percentiles separately. The present profiler's `build` bucket includes more than pure
  scene construction and does not report the first lock's wait separately.
- Map analysis workers pause between maps, rather than interrupting a map already being
  analyzed. Background CPU contention is possible, but has not been measured in this audit.

## What this audit does not diagnose as a bug

- An 8.33 ms acquisition wait at 120 Hz with vsync is pacing, not an 8.33 ms CPU render cost.
  `immediate` permits tearing. A second in-flight vsync frame is a policy to test on devices,
  not proof of a fixed extra frame of latency on every backend.
- The default 256-frame audio buffer has a 5.80 ms interval at 44.1 kHz. Queueing relative to
  the callback adds phase-dependent delay; reducing the buffer trades that against underruns.
- The uncapped renderer can consume a GPU, and the FPS limiter deliberately spins its final
  500 us. These policies have costs but need latency/power comparisons, not blind removal.
- Checksums for selected-set updates, live pp, and screenshot PNG encoding already have
  worker paths. `setstatus::Cache::save` already has a dirty guard. They must not be reported
  as unconditional per-frame full-file rewrites or synchronous network calls.

## Recommended order and validation

1. Cache/provision sound assets and move skin preparation outside the input lock (A1/A2).
2. Remove first-selection hashing with an empty offset table; precompute needed hashes (A5).
3. Snapshot around GPU reconfiguration and move submission/persistence work out of the lock
   (A6/A7/A8), with versioned completion and shutdown handling.
4. Bound section/velocity scans (A4) and background/coalesce large-library queries (A3).
5. Consider the smaller per-frame allocation reductions only after the above are measured.

For each implementation compare an identical scenario before/after, with output/judging
equivalence tests and allocation/lock instrumentation. Add frame-time, input-lock wait,
clock-sample age and audio-underrun counters; average FPS alone misses these stalls. Native
Metal/Vulkan/DX12 runs, actual keyboard-to-sound measurements, mobile thermals, and driver
behavior remain outside this headless CPU audit. This is not an exhaustive proof that every
client performance problem has been found.

## Validation of this branch

- Eight opt-in audit tests passed in the optimized profile, producing 65 reference observations.
- Ordinary client tests: 391 passed; nine ignored (eight opt-in audit cases plus one existing
  ignored test). The audit cases do not add runtime thresholds to ordinary CI tests.
- Client Clippy with `--all-targets -- -D warnings` passed on macOS. This is not a claim of
  native Linux/Windows lint or performance verification.
