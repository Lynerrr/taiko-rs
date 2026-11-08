## Summary

<!-- Describe the feature or fix, why it is needed, and the resulting behavior. -->

## Player impact

<!-- Explain what players will notice. For internal-only work, state that here. -->
<!-- For player-visible changes, add standalone Changelog: lines in plain English below.
     Copy those lines into the final squash commit body. Omit them for internal-only work. -->

## Regression tests

<!-- Client changes need regression tests (see docs/regression-tests.md). Name the tests added
     or changed, and explain every golden file (.snap) that changed and why. For a change that
     needs no new test (a pure refactor), say why and add the `no-regression-test` label. -->

- [ ] `cargo test --locked --all-targets` passes (regression suite included)
- [ ] New or changed behaviour is covered by tests in `src/regression/` (or module tests)
- [ ] Every changed `.snap` golden file was reviewed and is explained above

## Validation

<!-- List checks actually run and their results. Note any platform checks left to CI. -->
