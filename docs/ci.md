# Pre-merge checks

The `Pre-merge` GitHub Actions workflow runs on every pull request, on merge queue
groups, and on manual dispatch. For pull requests, it checks the proposed merge with the target branch.
New commits cancel the previous run for the same pull request.

- The game runs Clippy, all test targets, and a release build on Linux, Windows, and macOS.
- The server runs Clippy, all test targets, and a release build on Linux.
- Clippy treats every warning as an error (`-D warnings`). Both crates use their committed
  lockfiles (`--locked`).
- The existing `Release build` workflow also checks Android builds and desktop packaging
  on pull requests.

## Require checks before merging

After the first workflow run, add **Pre-merge checks** as a required status check in
the target branch's GitHub ruleset or branch protection settings. Enable **Require
branches to be up to date before merging** when using branch protection without a merge
queue. See [GitHub's required status check documentation](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks).

The final check runs even when a dependency fails or is skipped and succeeds only when
all desktop matrix jobs and the server job succeed. A workflow file alone does not enable
merge protection; the repository setting must require this status check.

Keep any existing required release checks to enforce Android builds and installer checks
as well. Before merging, rebase onto the latest target branch and use squash merge, as
specified in `AGENTS.md`.
