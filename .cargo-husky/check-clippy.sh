#!/bin/sh
set -eu

cd "$(git rev-parse --show-toplevel)"

# Editors launched from the desktop may not inherit the shell's Rust PATH.
if ! command -v cargo >/dev/null 2>&1; then
    cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"
    if [ -x "$cargo_bin/cargo" ]; then
        PATH="$cargo_bin:$PATH"
        export PATH
    else
        printf 'Commit blocked: Cargo was not found in PATH or %s. Install Rust or set CARGO_HOME for your editor.\n' "$cargo_bin" >&2
        exit 1
    fi
fi

for manifest in Cargo.toml server/Cargo.toml; do
    printf 'Checking %s with Clippy (warnings are errors)...\n' "$manifest"
    if ! cargo clippy --locked --manifest-path "$manifest" --all-targets -- -D warnings; then
        printf '\nCommit blocked: fix the Clippy diagnostics above and try again.\n' >&2
        exit 1
    fi
done
