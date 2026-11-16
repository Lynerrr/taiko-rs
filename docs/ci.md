# Pre-merge checks

The `Pre-merge` GitHub Actions workflow runs on every pull request, on merge queue
groups, and on manual dispatch. For pull requests, it checks the proposed merge with the target branch.
New commits cancel the previous run for the same pull request.

- The game builds its application and test binaries, runs Clippy, and executes the tests
  on Linux, Windows, and macOS.
- Android builds an APK with a debug signing key, without release secrets.
- Clippy treats every warning as an error (`-D warnings`). Builds use the committed
  lockfile (`--locked`).

## Build reuse

Each desktop job uses one runner, target directory, and `ci-test` profile for all
three steps. `cargo build --all-targets` compiles the application and test binaries once.
Clippy then analyzes the source, and `cargo test --all-targets` executes the already-built
test binaries. Clippy still needs its own compiler analysis; it cannot lint an executable.

The client is a Cargo workspace: the game (`.`) and the crates split out of it
(`crates/taiko-logic`, the rules with no window, GPU, sound or network; `crates/taiko-proto`,
what the game and the server both speak). All are default members, so these plain commands
build, lint and test all of them. Commands that need a single package, such as the APK's `cargo rustc`, name the game with `-p taiko-rs`. The
patched `vendor/` crates are excluded from the workspace.

The CI profile keeps debug assertions and unwinding enabled, omits debug symbols and
incremental data, and optimizes dependencies. Using the same panic strategy for application
and test targets avoids building dependencies again for tests.

The `Release build` workflow runs on pushes to the main/dev branches and manual dispatch,
not on pull requests. Production release optimization, desktop packaging, and installer
checks happen there, so PRs do not repeat the same tests/builds in two workflows. It runs no
tests: Pre-merge ran them on every platform before the merge.

## Build caches

A cache saved by a pull request is only visible to that pull request, so before 2026-11-16 every
pull request built all dependencies from nothing (`No cache found.` in each job), and their
caches crowded the repository's 10 GB out. Now:

- Pre-merge and the Perf gate also run on pushes to stable (`worktree-taiko-client`), and only
  those runs save caches (`save-if`). The Perf gate's push run only builds the head.
- Pull requests read stable's caches, pull requests into `dev` too: the default branch's caches
  are open to every branch.
- `Release build` saves its caches from stable only (`prefix-key: v1-rust`); dev's release
  builds read them.

## Sync to dev

Bug fixes land on stable first, and dev has to carry them: a release of dev to stable takes dev's
tree, so a fix only stable has would leave stable again. `.github/workflows/sync-to-dev.yml`
runs after each push to stable and copies the stable commits since the `dev-synced` tag onto dev
(`.github/scripts/sync-to-dev.sh`): it cherry-picks them, runs Pre-merge on Linux on the result
(each commit already passed all platforms on stable), pushes dev, moves the tag and starts dev's
release build. The Actions token's pushes start no workflows, hence the dispatch, and no pull
request: its checks would never run.

- Left alone: commits dev already has (a release of dev, a fix copied by hand) and commits with
  a line `Dev-sync: skip` in their message.
- Golden files (`src/regression/snapshots`) list every screen's texts and so often conflict with
  dev's own. A commit whose only conflicts are there keeps dev's side, the goldens are blessed
  again on Linux, and after the checks the copies go to dev through a pull request (one branch
  `sync/dev-<run>` per run) for a person to read the .snap diff, since a blessed golden needs a
  review. Merge it soon: the next copies onto dev come without it.
- Left for a person, in the issue "Sync to dev needs a hand": a commit that doesn't apply to dev,
  one that changes `.github/workflows/` (the Actions token may not push workflows), and copies
  that fail the checks. Dev stays as it was; copy those through a pull request into dev.

## Perf gate

The `Perf gate` workflow (`.github/workflows/perf-gate.yml`) runs on pull requests that touch
the client. It builds the pull request's base and head with the `perf-audit` profile (at the
same time: with one codegen unit each build keeps one core busy), runs the
[performance audit](performance-audit.md) three times for each, in turns on the same runner, and
compares them with `.github/scripts/perf-gate.py`:

- A case's value is the lowest p50 of its three runs: noise only ever adds time. On identical
  builds this stays within 5 %, where a single run or the median of three can be 10–20 % off.
- When a case looks slower, two more rounds run (head first this time) and the five runs of each
  side decide. A runner's noise can last several rounds: `frame/select` once read +24 % and
  then +1 % on a re-run of the same code. A pull request that slows nothing pays no extra time.
- A case fails when head is more than 10 % **and** more than 2 µs slower than base. Cases that
  time other threads, locks or the disk (`mutex/`, `skin/async/worker-completion`,
  `hash/first/`, `game/prepare/`) are reported but never fail, and so is
  `select/title-sort/50000`: its ~20 ms of allocations swing 10–30 % on runners, and
  `select/title-sort/10000` gates the same sort.
- Cases only the head has are listed as new; changed counts (shapes drawn) are listed too.
- The table lands in the job summary, and the raw logs in the `perf-logs` artifact.

A slowdown that is meant (explained in the pull request) is accepted with the
`perf-regression-ok` label; re-run the job after adding it. To try the comparison locally, save
the audit's output from each build and run
`python3 .github/scripts/perf-gate.py --base base-*.log --head head-*.log`.

## Require checks before merging

After the first workflow run, add **Pre-merge checks** as a required status check in
the target branch's GitHub ruleset or branch protection settings. Enable **Require
branches to be up to date before merging** when using branch protection without a merge
queue. See [GitHub's required status check documentation](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks).

The final check runs even when a dependency fails or is skipped and succeeds only when
all desktop matrix jobs and the Android build succeed. A workflow file alone does not enable
merge protection; the repository setting must require this status check.

If branch protection previously required checks from `Release build` on pull requests,
replace those required checks with **Pre-merge checks**; release checks no longer run on PRs.
Before merging, rebase onto the latest target branch and use squash merge, as specified in
`AGENTS.md`.

## Reading a Windows crash report

A release build's backtrace on Windows has no names (`<unknown>`), so a crash report also ends
in `frames: taiko-rs.exe+0x1a2b3c ...`: each return address as its module and offset. The
release run builds the exe with line tables in `taiko_rs.pdb` (not shipped) and keeps both as
the `windows-pdb` artifact for 90 days. To turn the frames into files and lines:

```sh
run=$(gh run list --workflow release.yml --commit <build> --json databaseId --jq '.[0].databaseId')
gh run download "$run" -n windows-pdb -D pdb
# The offsets of the taiko-rs.exe frames, as 0x... each; the exe's image base is 0x140000000.
for off in 0x1a2b3c 0x0f00d; do printf '0x%x\n' $((0x140000000 + off)); done |
  llvm-symbolizer --obj=pdb/taiko-rs.exe --pretty-print
```
