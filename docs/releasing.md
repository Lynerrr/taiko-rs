# Releasing and version numbers

The game's version is the `version` of the `taiko-rs` package in `Cargo.toml`, numbered
`MAJOR.MINOR.PATCH`. Players see it in the settings' Updates row, the update badge, the
download toast and the "What's new" window. Crash reports keep the commit beside it.

| Build | Called (`update::version_label`) |
| --- | --- |
| stable (`worktree-taiko-client`) | `1.0.0` |
| dev | `1.1.0-dev (abc1234)`: a test build of the next version, several per version |
| local | "local build" (no updates) |

The updater still decides by commit (a different commit on the channel = an update), so switching
from dev back to stable installs stable's build even though its version is lower. It reads what
to call the build from the release notes' first line, `Version: …`, which `release.yml` writes.
A release published before that line existed is called by its short commit.

## Releases on GitHub

- `latest` and `dev-build` are rolling: recreated on every push to stable and dev, named after
  the version. Installers (`install.ps1`, `install.sh`) and the updater download from them.
- Each stable version also gets a release of its own, `v1.0.0`, made by the first stable build of
  that version and never touched again (`--latest=false`: `latest` keeps GitHub's Latest badge).

## Releasing dev to stable

1. Dev already carries the version being released (dev builds show `X.Y.0-dev`).
2. A `release/X.Y.0` branch: `origin/dev` with stable merged in with `-s ours` (its tree is dev's),
   after checking stable's own commits since the last release are on dev. Pull request into
   `worktree-taiko-client`, squash-merged, with the players' `Changelog:` lines.
3. Right after, on dev: bump `version` to the next minor (`X.(Y+1).0`) in `Cargo.toml` and
   `Cargo.lock`, so dev's builds are the next version's test builds.

## A fix straight to stable

A `release/*` cherry-pick pull request into stable bumps the patch (`X.Y.1`) in the same pull
request, as a commit of its own with a line `Dev-sync: skip` in its message: dev carries a
higher version already, and the sync to dev (docs/ci.md) must not copy the bump over it. The fix
itself goes to dev as usual.
