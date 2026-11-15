# Client refactor: running it in parallel

`docs/client-refactor-next-steps.md` says **what** is left and in which order. This file says
**how** to run it with two or three Claude Code sessions at once, so the work goes faster
without the sessions stepping on each other. Where the two disagree about a step, the
next-steps document wins; where either disagrees with `AGENTS.md`, `AGENTS.md` wins.

State at the time of writing: `dev` at f5c0b39. Steps 0, 2, 4, 5 (up to #387) and 6 (`app/`,
`screens/`, #391) are merged. Next: **L2** and **S2**.

## 1. Lanes

The refactor runs in independent lanes. Each lane is one Claude Code session in its own git
worktree, working through its steps strictly in order.

| Lane | Steps, in order | Who | Worktree | Branch prefix |
|---|---|---|---|---|
| **L** logic crate | L2 → L3 → L4 → L5 | session 1 (account 1) | `../taiko-L` | `refactor/l<n>-` |
| **S** shell and screens | S2 → S3 → S4 → S5 → S6a, S6b, ... | session 2 (account 2) | `../taiko-S` | `refactor/s<n>-` |
| **P** protocol (optional) | step 1 `taiko-proto`, then 1b SQLite | session 3, if a third account is free | `../taiko-P` | `refactor/p<n>-` |

Rules that keep the lanes apart:

- **One open PR per lane.** The open PR is the lane's lock: open it as a draft right after the
  first commit, so everyone can see which step is taken. Nobody starts a step from another lane.
- A lane only touches its own files: L touches `crates/taiko-logic/`, `src/game*` and what
  moves into the crate; S touches `src/lib.rs`, `src/app/`, `src/screens/`, `src/regression/`'s
  harness; P touches the new `crates/taiko-proto/`, `server/` and the files it moves.
- Shared files (`Cargo.toml`, `Cargo.lock`, `docs/README.md`, `use` and `mod` lines in
  `src/lib.rs`) may be touched by any lane, but only as far as its step needs. Whoever merges
  second rebases and resolves (section 5).

## 2. Which model runs which step

Switch with `/model opus` or `/model sonnet` inside the session before starting the step.
Opus decides; Sonnet moves. Every Sonnet step ends with an Opus review before the merge
(step 6 of the loop in section 4).

| Step | What | Model | Why |
|---|---|---|---|
| L2 | Close the sim's API: read-only view or accessors; split `src/game/tests.rs` | **Opus** | a design choice (next-steps §5) and perf |
| L3 | `scores.rs` format to `logic::scores` | Sonnet | a move |
| L4 | `search.rs`, `mapinfo.rs` to `logic::library`; `recalc.rs` to a bin | Sonnet | a move |
| L5 | No `std::fs` in the crate; process-wide settings become arguments | **Opus** | changes how the play is called |
| S2 | The 40 free functions left in `lib.rs` to their modules | Sonnet | a move; split into 2 to 3 PRs |
| S3 | `Job<T>` and the 12 receivers; `settle` asks `is_idle()` | **Opus** | threading and the test harness |
| S4 | Overlay trait and the input stack | **Opus** | design |
| S5 | `Screen` trait, router, `CalibrateDone` | **Opus** | the pattern every S6 PR copies |
| S6 | `Download`, `Settings`, `Profile`/`Medals`, `VideoEdit` | Sonnet | follows S5's pattern |
| S6 | `Editor*`, `Watch`/`Spectate`, `Results` | Sonnet, Opus if the first try fights the borrow checker | |
| S6 | multiplayer, `Select`, `Play` | **Opus** | the largest state and the hot paths |
| 1 | `taiko-proto`: the server stops `#[path]`-including client files | Sonnet, Opus review | touches the live server |
| 1b | SQLite store | **Opus** | a new storage format |

## 3. One-time setup (on the machine that runs the sessions)

Two Claude accounts side by side: the second one gets its own config folder and shares the
first one's instructions and memory.

```sh
# ~/.bashrc
alias claude2='CLAUDE_CONFIG_DIR=$HOME/.claude-acc2 claude'

mkdir -p ~/.claude-acc2
ln -s ~/.claude/CLAUDE.md ~/.claude-acc2/CLAUDE.md   # if you have one
ln -s ~/.claude/projects  ~/.claude-acc2/projects    # shared memory
```

Run `claude2` once and `/login` with the second account. From then on `claude` is account 1
and `claude2` is account 2.

One worktree per lane, next to the main checkout:

```sh
cd taiko-rs
git fetch origin
git worktree add ../taiko-L origin/dev
git worktree add ../taiko-S origin/dev
```

Each worktree has its own `target/`, so two builds run at once. Keep them from starving each
other: put `export CARGO_BUILD_JOBS=8` (half the cores) in the shell that starts each session.
`gh auth status` must show a token with the `repo` scope; a PR that changes
`.github/workflows/` also needs `gh auth refresh -h github.com -s workflow`.

Start a lane:

```sh
cd ../taiko-L && claude     # lane L, account 1
cd ../taiko-S && claude2    # lane S, account 2
```

and paste the lane's prompt from section 6.

## 4. The loop for every step

1. `git fetch origin && git switch -c refactor/<lane><n>-<topic> origin/dev`. Never branch
   from another refactor branch.
2. Read the step's row in `docs/client-refactor-next-steps.md` §3 and the rules in its §4.
3. Count the tests before touching anything, per crate:
   `cargo test --locked --all-targets -- --list 2>/dev/null | grep -c ': test$'` and the same
   with `-p taiko-logic`. Keep the numbers for the PR.
4. Do the step. A bug or a slow spot found on the way is **not** fixed in the move: note it,
   and fix it in its own PR, merged first (next-steps §4.1).
5. Check:
   - `cargo fmt --all --check`
   - clippy for both manifests (the pre-commit hook runs it on `git commit`):
     `cargo clippy --locked --all-targets -- -D warnings` and the same with
     `--manifest-path server/Cargo.toml`
   - `cargo test --locked --all-targets`; for lane P also `cargo test --locked --manifest-path server/Cargo.toml`
   - a move is only a move: `git diff -M --color-moved=dimmed-zebra origin/dev`, and anything
     that is not path, visibility or imports is explained in the PR
   - test counts after equal test counts before (or the difference is explained)
6. Sonnet steps only: `/model opus`, then `/code-review high` on the branch. Fix what it finds,
   then repeat step 5.
7. Push and open the PR against `dev`: Conventional Commit title with a scope, the template
   filled in, test counts before and after, the move check's result. A pure move gets the
   `no-regression-test` label with "pure refactor, proven by the existing suite" in the
   description.
8. Wait for the PR's checks on all platforms: `gh pr checks --watch`. Don't wait for `dev`'s
   Release build after the merge; one check of it at the end of a series is enough. While the
   checks run, read ahead for the next step, but don't start committing it.
9. Merge: `git fetch origin && git rebase origin/dev`; if the rebase changed anything, rerun
   step 5 and `git push --force-with-lease`, then wait for the checks again. Then
   `gh pr merge --squash` with a Conventional Commit message.
10. Back to 1 with the lane's next step.

Small steps can share a PR (three or four pieces of S2, for example), as long as the PR stays
under about 1,500 moved lines and one concern.

## 5. When lanes collide

- **Rebase conflict in a shared file:** keep both sides (both lanes' `mod` lines, both
  `Cargo.toml` entries). For `Cargo.lock`, take `dev`'s version and run `cargo build` once to
  add the lane's own entries back, then rerun all of step 5.
- **L3 or L4 and S2 both need `src/lib.rs`:** L changes only the `use` lines that name the moved
  module; everything else in `lib.rs` belongs to S.
- **A step needs something another lane has not merged yet** (S6 for `Play` needs L2's view, for
  example): stop that lane at the step before it, and say so in the lane's PR or in chat.

## 6. Session prompts

Paste one into each session after `cd` into its worktree.

**Lane L**

> You run lane L of the client refactor. Read `docs/refactor-runbook.md` and
> `docs/client-refactor-next-steps.md` first. Do steps L2, L3, L4, L5 in order, one PR each,
> following the loop in the runbook's section 4 exactly. Use the model from the runbook's
> section 2 for each step and tell me when to switch with `/model`. For L2's open question
> (view or accessors), port the first 200 lines of `field.rs` both ways, compare with the perf
> gate, pick one and write the numbers and the reason in the PR. Stop and ask me only for the
> cases in the runbook's section 7.

**Lane S**

> You run lane S of the client refactor. Read `docs/refactor-runbook.md` and
> `docs/client-refactor-next-steps.md` first. Do S2, S3, S4, S5 and then S6 one screen per PR,
> in the order the next-steps document gives, following the loop in the runbook's section 4
> exactly. Use the model from the runbook's section 2 for each step and tell me when to switch
> with `/model`. Stop and ask me only for the cases in the runbook's section 7.

**Lane P** (optional)

> You run lane P of the client refactor. Read `docs/refactor-runbook.md` and
> `docs/client-refactor-next-steps.md` first. Do step 1 (`taiko-proto`): a crate that both the
> client and `server/` depend on, so the server no longer includes client files by `#[path]`.
> One PR, following the loop in the runbook's section 4 exactly, with the server's tests and
> clippy too. Stop and ask me only for the cases in the runbook's section 7.

## 7. When a session stops and asks a human

- A golden `.snap` file would change. A refactor that changes one is wrong.
- The perf gate is clearly slower or clearly faster on a move.
- A platform job fails twice in a row for a reason that isn't obviously the step's code.
- A rebase conflict touches logic, not just `mod`/`use` lines or `Cargo.toml` entries.
- The step needs another lane's unmerged work (section 5).
- The step turns out much bigger than its row (over about 2,500 moved lines): propose a split
  before going on.
