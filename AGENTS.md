# Repository guidelines

## Commit messages

- Use Conventional Commits with a required scope: `<type>(<scope>)[!]: <description>`.
- Use a lowercase, kebab-case scope that names the affected area or task, for example `move-guide-to-docs`.
- Write commit subjects and bodies in English.
- Use a concise, imperative description, for example `fix(audio): reduce playback latency`.
- For a guide move, use `chore(move-guide-to-docs): move README1.md to docs`.
- Choose the appropriate type, such as `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `build`, `ci`, or `chore`.
- Mark breaking changes with `!` after the scope, or a `BREAKING CHANGE:` footer.

## Branch integration

- Before merging a branch, fetch the latest changes and rebase it onto the current target branch.
- Resolve rebase conflicts and rerun the relevant checks before merging.
- Use squash merge to integrate a branch as one commit; do not create merge commits.
- Write the squash commit message in English using Conventional Commits.
- If updating a rebased remote branch requires a force push, use `--force-with-lease`.
