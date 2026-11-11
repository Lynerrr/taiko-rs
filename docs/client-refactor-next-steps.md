# Client refactor: where it stands and the next steps

This continues `docs/client-architecture.md` (branch `docs/client-architecture`): that document
says where the client should end up and lists the steps (its section 6). This one records what
is done, what the review of steps 5 and 6 taught, and the order to do the rest in. Where the two
disagree about order, this one is newer; the target layout (its sections 2 to 5) is unchanged.

Numbers below are from `dev` at 817037b with #387 and `refactor/lib-screens` on top.

## 1. Where it stands

| Step | What | State |
|---|---|---|
| 0 | Regression suite, perf gate | done (#350, #362, #370, #377) |
| 1 | `taiko-proto` | done: `net`, `online`, `tls`, `lang`, `changelog_entry` in `crates/taiko-proto`; the server has no `#[path]` includes left |
| 1b | SQLite store | not started |
| 2 | Wrong-way dependencies | done (#364, #365) |
| 3 | Drawing globals to a frame's theme and look | done for the playfield (#366); 75 `static`/`thread_local!` left in `src/` |
| 4 | Split `game.rs` into the play and its drawing | done (#367 to #380) |
| 5 | `taiko-logic` | leaf rules, calibrate, mods, Defense, maps, tko's model, genres, font: done (#381 to #385). The play, stars, pp: #387. Scores and replays (#408), search and map numbers (#409), `--recalc-pp` as `crates/taiko-recalc`; the crate reads no files and keeps no statics (#418, #420, #421). The client keeps thin wrappers over it (`scores.rs`, `mapinfo.rs`, `stars.rs`, `pp.rs`, `beatmap.rs`) |
| 6 | `lib.rs` free functions out; `app/` | `app/` done (#386, #389); select, results and play helpers: `refactor/lib-screens`. Left: 40 free functions and one `impl State` of 361 methods in a 17,700-line `lib.rs` |
| 7 to 15 | | not started |

What holds `State` together today, and so what the next steps have to cut:

- 237 fields in one struct, one `impl` block.
- 28 `*_key` handlers and 28 `open_*` methods: input and navigation for every screen.
- 22 `poll_*` methods over 12 `Option<Receiver<..>>` fields: every background job.
- About 600 places that name `Scene::` or `self.scene`.

## 2. What the review of steps 5 and 6 taught

Each of these was a finding; each becomes a rule in section 4.

1. **A test waited for the wrong thing.** The pointer tests looked for the preview's seek graph
   on the first frame after `ready`, which waits for text, and the graph (drawn after a
   background read) has none. It failed once on Windows CI (#388). Any test that depends on a
   background job has to wait for that job, and the harness cannot ask "is anything still
   loading?" because every job is a hand-written receiver. Step 9 (`Job<T>`) fixes the cause.
2. **Words crept into the logic crate.** `lang` and `tr!` moved into `taiko-logic` for three
   functions of `multiverse.rs`, against the plan's 4.1, and brought a process-wide language and
   a `testing` switch with them. Fixed in #387: the crate keeps keys, the client words them, as
   #383 and #385 already did.
3. **A fix rode along with a move.** `Mod::bit`'s lookup table (a third off the dense playfield's
   draw time) landed inside the PR that moved the sim across a crate line, so the perf gate
   could not show what the move alone costs. Bits were verified identical, but the two changes
   cannot be told apart in the numbers.
4. **Moves opened too much.** About 200 items of the sim became `pub` so the client's drawing
   and tests could keep reaching into `Game`; the app shell's pieces became `pub(crate)` where
   `pub(super)` does. A crate's `pub` is its API: the server can now write `game.cursor`.
5. **Guards that read source text are weak.** `contains("fn run(")` passes on a comment and
   fails on an unrelated function. The guards that held were the compiler's: Cargo keeping
   `wgpu` out of the crate, a `const` assertion on the mod table, privacy.
6. **Docs lagged the move.** `docs/README.md` still pointed at `src/game.rs` for the play.
7. **`use crate::*` hides what a module needs.** Every new file under `app/` and `screens/`
   starts with it (under `#[allow(unused_imports)]`), so rule 5 of the architecture ("screens
   don't import other screens") cannot be checked, and a move cannot show its dependencies.

What went well and should stay: one concern per PR, squash-merged after green CI on all
platforms; moves that really were verbatim (checked line by line, see 4.1); test counts that
add up across every step (1256 before step 5, 1266 now, nothing lost); no golden file changed.

## 3. The order from here

The architecture's table has 7 (screens) before 8 (overlays) and 9 (jobs). The numbers in
section 1 say otherwise: what ties a screen to `State` is its background jobs and its share of
the input chain, so moving a screen first means moving it twice. The order below does the
cheap, enabling steps first. Step numbers are the architecture's; letters are the PRs.

Two tracks stay independent, as before: **L** (logic crate, touches `crates/` and `src/game*`)
and **S** (shell and screens, touches `src/lib.rs`). One open PR per track at a time: both
tracks' PRs conflict with themselves, not with each other.

### Track L: finish the logic crate

| PR | What | Done when |
|---|---|---|
| L1 | Merge #387 (the play, stars, pp) after #388 makes Windows green | CI green on all platforms; perf gate not slower |
| L2 | **Close the sim's API.** The client's drawing reads the play through a read-only view (`sim::View<'_>` or accessor methods: time, notes in range, judgements, scroll, lane geometry); fields of `Game`, `NoteState`, `Scroll`, `LaneGeo`, `Tower`, `Crumb` go back to private. Tests that poke fields move into the crate; tests that draw stay in the client and use the view | `grep -c "pub " crates/taiko-logic/src/sim.rs` back near the count before #387; the server cannot write a play's state; perf gate neutral (accessors `#[inline]`) |
| L3 | `scores.rs` format (replays, score rows) to `logic::scores`; file reading and writing stay in the client | a replay parses and re-plays with `cargo test -p taiko-logic` alone |
| L4 | `search.rs`, `mapinfo.rs` to `logic::library`; `recalc.rs` to a small bin over the crate | the query language tests run in the crate |
| L5 | Crate hygiene: no `std::fs` outside `from_bytes`/`to_bytes` wrappers (rule 2; `beatmap::load`, `stars::load` take bytes, the client reads the file); the process-wide settings (`od_override`, `tact`, `tko::runtime::CALM`, `multiverse::CURRENT`) become arguments of the play | `grep -rn "std::fs" crates/taiko-logic/src` only in tests; no `static` with interior mutability in the crate |

L2 comes before L3 and L4 on purpose: every later move into the crate is cheaper once the rule
"the crate's `pub` is designed, not inherited" has an example to follow.

### Track S: the shell, then the screens

| PR | What | Done when |
|---|---|---|
| S1 | Merge #389 and `refactor/lib-screens` (with `refactor/lib-screens-review` on top) | |
| S2 | The remaining 40 free functions of `lib.rs` to the module they serve (texts and badges to their screens; `open_url`, `file_uri`, `profile_url` to a `services::system` module; `find_map`, `map_info`, `load_plays` next to the library) | `grep -cE "^(pub(\(crate\))? )?fn " src/lib.rs` is 0 |
| S3 | **Step 9 first: `Job<T>`.** One type for "work on another thread, result polled on the frame" with `poll()`, `is_idle()` and cancel-on-drop; the 12 receivers and 22 `poll_*` methods move to it one family at a time (preview map and music, boards, downloads, results, replays, scans). The regression harness's `settle` asks every job `is_idle()` instead of comparing text | no `Option<Receiver<` field in `State`; `settle` has no sleep-and-compare loop; the pointer tests' own wait (#388) is deleted |
| S4 | **Step 8 next: overlays and the input stack.** Social, volume, toast, quit, guide, idle and crash overlays get one trait (`key`, `pointer`, `draw`, `modal`) and a stack that input walks top-down; their fields leave `State` | the 28 `*_key` handlers no longer start with "if an overlay is open"; overlays do not name `Scene` |
| S5 | Step 7a: `Screen` trait, router, `Scene::Other(Box<dyn Screen>)`, and the first screen (`CalibrateDone`) | one screen lives fully outside `lib.rs`, with its fields |
| S6.. | Step 7b: one screen per PR, simplest first: `Download`, `Settings`, `Profile`/`Medals`, `VideoEdit`, `Editor*`, `Watch`/`Spectate`, `Results`, multiplayer, `Select`, `Play` | per screen: its `Scene` arm, its `*_key`, `open_*` and `draw_*` methods and its `State` fields are gone from `lib.rs` |

Track S progress: S1, S2 (#395, #396) and S3 (#397, #401, #403) are done. S4 is done as
`src/app/overlays.rs` (#410, #411, #413, #414 and the PR that added this note): the overlays
are a `Layer` enum, not trait objects, because what a layer does mostly acts on `State` (the
guide calibrates, a profile opens medals, the clipboard types into the screen). Keys, the wheel,
the left button and drawing each walk their list of layers; a layer says whether it took the
input, and none names `Scene`. The guide's fields are one `guide::Tour`; the other overlays
already were one field each (`social`, `volume`, `idle`, `unlock`, `profile`, `medals`, ...).

S5 is `src/screens/screen.rs` and `src/screens/calibrate_done.rs`: a screen outside lib.rs is a
`Screen` trait object behind `Scene::Other`. While it takes a key it is taken out of
`State::scene`, so it gets the whole `&mut State`, and answers `Next::Stay` (put it back) or
`Next::Left` (it has set the scene that follows). Each screen moved in S6 is added to `MOVED` in
`screen.rs`'s `moved_screens_stay_out_of_lib` test.

S6 starts with `VideoEdit` (`src/screens/video_edit.rs`), not `Download`: the download tab is
song select's list and preview showing remote maps, named next to `Select` in about 25 places,
while the video editor was named in 14. It also replaced S5's stand-in while a screen handles
input: the stand-in was `Scene::Select`, so code the screen calls (global keys, which check for
song select's search) could see the wrong screen. A `Shadow` now holds the screen's place and
answers every query of the `Screen` trait as the screen would, so each query a moved screen adds
to the trait is added to `Shadow` as well. A screen leaves by putting another scene in its place.
The video export stays `State::export`: the render thread draws it and it ends on its own.

Done so far: `VideoEdit` (#422), `LayoutEdit` (#424), `HudEdit` (#425), `EditorHome` (#426).

#### S6, the remaining screens

The four screens above had their own state and few readers. The rest are read by the code
around them, and a `Shadow` can answer only what it copied when the screen was taken out:
flags and small `Copy` values, never a borrow into the screen. So the rest move under these
rules, decided before the code:

1. **Questions, not variants.** Code outside a screen never matches on what is inside it. It
   asks the trait, and the answer is a flag or a small value the `Shadow` copies: `map_entry()
   -> Option<usize>` (the map it is about), `over_lobby()` (opened over the lobby, so the lobby's
   map counts), `over_menu()` (opened from a top-bar screen). These replace the matches on
   `Scene::Preview { back }` and `Scene::Settings { prev }` that look into the scene behind.
2. **One group, not a flag per site.** About 25 places list `Select | Preview | Download`
   together: the song list's pointer and scrubbing, the preview panel, the map's stars, the
   score link. A screen says it belongs to song select with `song_select()`; those places ask
   `State::in_song_select()` (true for `Scene::Select` and for such a screen). A place whose list
   differs from the group keeps its own question rather than bending the group.
3. **What a frame was given, it is still given.** Some arms used values `build_frame` worked
   out before polling media keys and other input (the preview target, the size label, whether
   the player hosts). Working them out again inside the screen could see a different song in
   that frame, so `Screen::draw` gets them in a `Frame`, computed where they were.
4. **The game is not inside a screen.** `Play`, `MpPlay`, `Calibrate`, `Watch` and `Spectate`
   hold a `Play` that the input, the audio clock, presence, replays and pp all read while the
   screen is up. The play moves to one `State::play` slot first (its own PR, no screen moved),
   and the screens then hold only what is theirs (the entry, `back`, the watched player). After
   that they move like the others.
5. **Order:** `Preview`, `Download`, then the multiplayer screens together (`MpMenu`, `Lobby`,
   `MpWait`, `MpResults`, which the multiplayer state machine creates from many places),
   `Results`, the `State::play` slot, `Settings` (`live_play_ms` and `live_map` read the paused
   play inside its `prev`, so it needs the slot), the play screens, `Editor`, and `Select`
   last. Tests that build or match a moved scene use its `scene()` constructor and
   `Scene::screen_mut::<T>()`; their assertions stay the same.

After S3 and S4 a screen's move in S6 is its own state, its drawing and its input, and nothing
else: no job plumbing and no overlay checks travel with it.

### Later steps, unchanged in order

10 (settings registry), 11 (`taiko-engine`), 12 (threading), 13 and 14 (mod and effect
registries, need L2), 15 (`ui_kit` on taffy, needs S6). Step 1 (`taiko-proto`) and 1b (SQLite)
are independent of both tracks and can start any time; step 1 is what finally removes the
server's `#[path]` includes, so it is worth starting before L3 adds more shared types.

### Sizing

A PR that moves code stays under about 1,500 moved lines and touches one concern: #387 (3,000
lines, a move plus a perf fix plus a feature flag) was reviewable only with a script. Expect
S6 to be ten to twelve PRs; `lib.rs` should lose 1,000 to 2,500 lines with each.

## 4. Rules for every refactor PR

### 4.1 A move is only a move

- Nothing but the path, the visibility and the imports changes. Verify it mechanically before
  asking for review: diff the removed lines against the added ones after stripping `pub(...)`
  and `crate::`/`super::` prefixes; what is left over must be explainable line by line in the
  PR description. (`git diff -M --color-moved=dimmed-zebra` shows the same by eye.)
- A bug or a slow spot found on the way gets its own PR, merged **first**, so the move is
  measured against a base that already has it.
- Count `#[test]` before and after, per crate, and put both numbers in the description.

### 4.2 Visibility

- Start private; open to `pub(super)`, then `pub(crate)`, only as far as a caller needs.
- In `taiko-logic`, `pub` means "part of the crate's API": fields stay private, and a test that
  needs to poke one lives in the crate.
- No `use crate::*` in new modules: name what the module uses. (Existing ones are cleaned up as
  each file is next touched.)

### 4.3 The logic crate

- No words: the crate returns keys, enums and numbers; the client turns them into text
  (`tr!` lives in `taiko_proto::lang`).
- No window, GPU, sound or network (Cargo enforces it), no file access outside explicit byte
  wrappers, no new process-wide state.
- The `testing` feature is for test helpers only and is enabled only from dev-dependencies. It
  must never change what shipped code does.

### 4.4 Guards

- Prefer a guard the compiler runs: privacy, a `const { assert!(..) }`, a missing dependency.
- A source-text guard (`include_str!` + `contains`) is a last resort; it names a whole item
  (`"pub(super) fn render_loop("`), not a fragment a comment could match.

### 4.5 Tests and background work

- A test never assumes a background job has finished: it waits for the job (after S3:
  `Job::is_idle`), with a timeout that fails with a message saying what never arrived.
- Golden files change only with an explanation in the PR; a refactor step that changes one is
  wrong.

### 4.6 Performance

- The perf gate (`.github/workflows/perf-gate.yml`) must be neutral for a move. A move that is
  faster is suspicious for the same reason as one that is slower: something else changed.
- The gate builds the `perf-audit` profile (no LTO, 16 codegen units); releases use thin LTO
  with one unit. A cross-crate slowdown seen only in `perf-audit` is real for the gate and for
  `android-dev`, but say in the PR which profile the numbers are from.
- `#[inline]` goes on small functions the per-frame drawing calls across a crate line, not on
  everything that moved.

### 4.7 Platforms and docs

- Code under `#[cfg(windows)]`, `#[cfg(target_os = "android")]` or macOS-only is verified by
  that platform's CI job and by nothing else: say so in the description, and do not merge on a
  red or missing platform job.
- The PR that moves a file updates every path that names it: `docs/README.md`'s layout table,
  `docs/ci.md`, comments in `server/` and `assets/`, `include_str!` paths in guards
  (`grep -rn "src/<old name>"` over the repository, not only `src/`).

## 5. Open questions

- **L2's shape.** A borrowed view struct keeps the drawing code close to what it is today
  (`view.notes()`, `view.scroll()`); accessor methods on `Game` are less code but leave the API
  as wide as the drawing's needs. Decide with the first 200 lines of `field.rs` ported both ways
  and the perf gate's numbers for each.
- **Where the sim's tests live.** `src/game/tests.rs` (about 3,900 lines) tests the play and its
  drawing together. Splitting it is most of L2's work; the split line is "does the test build a
  `Canvas`?".
- **`#[inline]` versus LTO in `perf-audit`.** If the gate's profile used thin LTO the marks
  would be unnecessary, at the price of a slower gate. Measure the gate's wall time both ways
  before adding more marks.
