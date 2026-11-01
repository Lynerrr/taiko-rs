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
