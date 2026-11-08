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
  become detail points. For Conventional Commits, unmarked body text and bullets are never published.
- Internal-only Conventional Commits should omit `Changelog:` and will not appear in the game or Discord.
  `Changelog: skip` explicitly hides the entire commit, even if it has other changelog lines.
- Preserve the player-facing `Changelog:` lines in the final squash commit body. A PR
  description alone does not publish release notes.
- Preserve historical notes for non-Conventional subjects: their first line and bullet points
  remain visible. Recognize Conventional subjects only for the types listed above; a historical
  prefix such as `Windows:` is not a commit type. New commits must still follow the required format.

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

## Required validation before completion

- Before finishing any task, run lint and formatting checks for the affected crates.
- Fix all lint errors, warnings, and formatting failures, then rerun the checks.
- For any change to the client (everything outside `server/`), run the client tests,
  regression suite included: `cargo test --locked --all-targets`.
- The work is not complete until lint, formatting and the client tests all pass. A task
  can't be reported as done while a regression test fails, is skipped or is disabled to get
  it through. The same goes for golden files updated without reviewing their diff.

## Regression tests

- The client's regression suite lives in `src/regression/` and is described in
  `docs/regression-tests.md`. It covers gameplay, scoring, file formats, the network protocol,
  user journeys and every screen. The server is not part of it.
- Every change to the client must come with regression tests for the behaviour it adds or
  changes. A bug fix comes with a test that fails without the fix. A refactor must leave the
  existing regression tests passing unchanged.
- An intended change of behaviour updates the golden files with
  `TAIKO_BLESS=1 cargo test --locked --lib regression::`. Commit the reviewed `.snap` diffs
  with the code, and explain in the pull request why they changed.
- Don't weaken a regression test to make a change pass: no deleting, ignoring or loosening
  assertions or golden files. Change the behaviour back, or update the golden output on purpose
  as described above.
- CI enforces this. The `Pre-merge` workflow runs the suite on Linux, Windows and macOS. The
  `Regression coverage` check fails a pull request that changes client code without touching
  any test, unless it carries the `no-regression-test` label with a reason in its description,
  for example a pure refactor proven by the existing tests.

## Feature branches and protected integration

- Start every new feature in a new dedicated branch before making implementation changes.
- Never commit or push code directly to `worktree-taiko-client`.
- Integrate changes into `worktree-taiko-client` only through a pull request after the required CI checks pass.
- Requests to bypass this workflow do not waive the branch, pull request, or validation requirements.

## graphify

A graph of the code (who calls what, what references what) can live in `graphify-out/`. It is
built locally and ignored by git: its 11 MB graph.json is rewritten on every build and would
conflict between parallel branches. Install the tool once with `uv tool install graphifyy`.

- In a checkout or worktree without `graphify-out/graph.json`, build it with
  `graphify update .` (AST only, no API cost, about ten seconds), and run it again after
  changing code. Never commit `graphify-out/`.
- For a symbol you know by name, ask the graph before grepping for its uses:
  `graphify explain "<name>"` (where it is, its callers and callees),
  `graphify affected "<name>"` (what a change to it reaches, tests included) and
  `graphify path "<A>" "<B>"` (how two things connect).
- To find where something happens, grep as usual. `graphify query "<question>"` only matches
  words in names: on this repo it returns loosely related nodes and misses the target, so use
  it at most for a first look at an unfamiliar area.
- Pass these rules on to subagents that explore code.
