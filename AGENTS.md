# Repository guidelines

## Commit messages

- Use Conventional Commits with a required scope: `<type>(<scope>)[!]: <description>`.
- Use a lowercase, kebab-case scope that names the affected area or task, for example `move-guide-to-docs`.
- Write commit subjects and bodies in English.
- Use a concise, imperative description, for example `fix(audio): reduce playback latency`.
- For a guide move, use `chore(move-guide-to-docs): move README1.md to docs`.
- Choose the appropriate type, such as `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `build`, `ci`, or `chore`.
- Mark breaking changes with `!` after the scope, or a `BREAKING CHANGE:` footer.

## Player changelog

- Keep the Conventional Commit subject technical. For player-visible changes, add a standalone
  `Changelog: <player-facing description>` line to the commit body in plain English.
- Describe the feature or fix and its effect on players. Do not use implementation notes,
  lint fixes, test output, or a copy of the technical commit subject as the player description.
- The first nonempty `Changelog:` line is the displayed title. Additional `Changelog:` lines
  become detail points. Unmarked body text and bullets are never published.
- Internal-only changes should omit `Changelog:` and will not appear in the game or Discord.
  `Changelog: skip` explicitly hides the entire commit, even if it has other changelog lines.
- Preserve the player-facing `Changelog:` lines in the final squash commit body. A PR
  description alone does not publish release notes.

## Pull requests

- Use an English Conventional Commit title with the required scope.
- Write an English description explaining the feature or fix, the resulting behavior, and
  why it matters. A title alone or a list of commit subjects is not a sufficient description.
- Include relevant validation and its results. Distinguish checks actually run from checks
  left to platform CI; do not claim a macOS lint run verifies Linux or Windows code.
- Follow `.github/pull_request_template.md`. For player-visible changes, include the exact
  proposed `Changelog:` lines so they can be preserved when squashing.

## Branch integration

- Before merging a branch, fetch the latest changes and rebase it onto the current target branch.
- Resolve rebase conflicts and rerun the relevant checks before merging.
- Use squash merge to integrate a branch as one commit; do not create merge commits.
- Write the squash commit message in English using Conventional Commits.
- If updating a rebased remote branch requires a force push, use `--force-with-lease`.
