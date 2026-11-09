#!/bin/sh
# Fails when the changes between two commits touch client code but no test.
# Usage: check-regression-tests.sh <base> <head>; WAIVED=true skips the requirement.
# Run it locally as: sh .github/scripts/check-regression-tests.sh origin/dev HEAD
set -eu

base=$1
head=$2
range="$(git merge-base "$base" "$head")..$head"
changed=$(git diff --name-only "$range")

# Client code: the game's sources and what is built into it. The server, docs, CI and the
# platform packaging (android/, windows/, macos/) are not.
client=$(printf '%s\n' "$changed" |
    grep -E '^(src/|crates/|assets/|vendor/|build\.rs$|Cargo\.toml$)' |
    grep -vE '^src/regression/|_tests?\.rs$' || true)

if [ -z "$client" ]; then
    echo "No client code changed: nothing to check."
    exit 0
fi

# Tests: the regression suite (code or golden files), test modules, or new #[test]s anywhere
# in the client and its crates.
tests=$(printf '%s\n' "$changed" | grep -E '^src/regression/|_tests?\.rs$' || true)
added=$(git diff --unified=0 "$range" -- src crates | grep -cE '^\+.*#\[(test|tokio::test)\]' || true)

echo "Client files changed:"
printf '  %s\n' $client
if [ -n "$tests" ] || [ "${added:-0}" -gt 0 ]; then
    echo "Tests changed with them:"
    [ -n "$tests" ] && printf '  %s\n' $tests
    [ "${added:-0}" -gt 0 ] && echo "  $added new #[test] function(s) in src/ or crates/"
    exit 0
fi

if [ "${WAIVED:-false}" = true ]; then
    echo "No tests changed, but the pull request is labelled no-regression-test."
    echo "Its description must say why no test is needed."
    exit 0
fi

echo
echo "::error::Client code changed without any regression test."
echo "Add or update tests for the change (docs/regression-tests.md): src/regression/, a test"
echo "module, or golden files blessed with TAIKO_BLESS=1. For a pure refactor that the existing"
echo "tests already prove, add the 'no-regression-test' label and explain it in the description."
exit 1
