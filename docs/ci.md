# Pre-merge checks

The `Pre-merge` GitHub Actions workflow runs on every pull request, on merge queue
groups, and on manual dispatch. For pull requests, it checks the proposed merge with the target branch.
New commits cancel the previous run for the same pull request.

- The game builds its application and test binaries, runs Clippy, and executes the tests
  on Linux, Windows, and macOS.
- The server runs the same checks on Linux.
- Android builds an APK with a debug signing key, without release secrets.
- Clippy treats every warning as an error (`-D warnings`). Both crates use their committed
  lockfiles (`--locked`).

## Build reuse

Each desktop/server job uses one runner, target directory, and `ci-test` profile for all
three steps. `cargo build --all-targets` compiles the application and test binaries once.
Clippy then analyzes the source, and `cargo test --all-targets` executes the already-built
test binaries. Clippy still needs its own compiler analysis; it cannot lint an executable.

The CI profile keeps debug assertions and unwinding enabled, omits debug symbols and
incremental data, and optimizes dependencies. Using the same panic strategy for application
and test targets avoids building dependencies again for tests.

The `Release build` workflow runs on pushes to the main/dev branches and manual dispatch,
not on pull requests. Production release optimization, desktop packaging, and installer
checks happen there, so PRs do not repeat the same tests/builds in two workflows.

## Perf gate

The `Perf gate` workflow (`.github/workflows/perf-gate.yml`) runs on pull requests that touch
the client. It builds the pull request's base and head with the `perf-audit` profile, runs the
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
all desktop matrix jobs, the server job, and the Android build succeed. A workflow file alone does not enable
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
