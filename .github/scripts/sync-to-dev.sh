#!/usr/bin/env bash
# The git side of sync-to-dev.yml: copies stable's new commits onto dev.
#   pick    cherry-picks the commits after the `dev-synced` tag (or just this push's) onto dev
#           and pushes the result to the $BRANCH branch for the checks
#   bless   re-makes the golden files (src/regression/snapshots) the copies conflicted in
#   land    pushes $BRANCH to dev (on top of dev, if dev moved meanwhile), moves the tag and
#           starts dev's release build; with re-made goldens it opens a pull request instead
#   failed  reports that the copies failed the checks
# Needs GH_TOKEN, GH_REPO, RUN_URL and BRANCH, as the workflow sets them.
set -euo pipefail

mode=$1
title="Sync to dev needs a hand"
problems="$RUNNER_TEMP/problems.md"
: >"$problems"
note() { printf -- '- %s\n' "$*" >>"$problems"; }

# Opens the issue, or adds to the one still open, when something was left for a person.
report() {
    [ -s "$problems" ] || return 0
    body=$(printf 'Run: %s\n\n%s\n\nCopy these onto dev by hand (a pull request into dev), or close this if dev has them.\n' \
        "$RUN_URL" "$(cat "$problems")")
    n=$(gh issue list --state open --search "in:title \"$title\"" --json number,title \
        --jq "map(select(.title == \"$title\")) | .[0].number // empty")
    if [ -n "$n" ]; then
        gh issue comment "$n" --body "$body"
    else
        gh issue create --title "$title" --body "$body"
    fi
    echo "::warning::left for a person: see the issue \"$title\""
}

git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
# By full name: a tag called dev once shadowed the branch.
git fetch -q --no-tags origin +refs/heads/dev:refs/remotes/origin/dev
git fetch -q --no-tags origin +refs/tags/dev-synced:refs/tags/dev-synced 2>/dev/null || true

case $mode in
pick)
    head=$GITHUB_SHA
    if git rev-parse -q --verify refs/tags/dev-synced >/dev/null; then
        commits=$(git rev-list --reverse --first-parent "refs/tags/dev-synced..$head")
    else
        commits=$head
    fi
    if [ "$(printf '%s\n' "$commits" | grep -c .)" -gt 30 ]; then
        note "more than 30 commits since the \`dev-synced\` tag: it's probably in the wrong place. Nothing was copied; move the tag to the last stable commit dev has (\`git push -f origin <sha>:refs/tags/dev-synced\`) and re-run."
        report
        exit 1
    fi
    # A release of dev to stable has the tree dev had then.
    dev_trees=$(git log --format=%T -n 300 refs/remotes/origin/dev)

    git worktree add -q --detach "$RUNNER_TEMP/dev" refs/remotes/origin/dev
    cd "$RUNNER_TEMP/dev"
    list=""
    bless=false
    for c in $commits; do
        subject=$(git log -1 --format=%s "$c")
        short=$(git rev-parse --short "$c")
        if git log -1 --format=%B "$c" | grep -qx 'Dev-sync: skip'; then
            echo "left alone (Dev-sync: skip): $subject"
            continue
        fi
        if printf '%s\n' "$dev_trees" | grep -qx "$(git rev-parse "$c^{tree}")"; then
            echo "left alone (a release of dev): $subject"
            continue
        fi
        # Copied by hand already: its change undoes cleanly on dev.
        if git diff "$c^" "$c" | git apply --check -R 2>/dev/null; then
            echo "left alone (dev has it): $subject"
            continue
        fi
        goldens=""
        if ! git cherry-pick --no-commit "$c" >/dev/null 2>&1; then
            conflicted=$(git diff --name-only --diff-filter=U)
            # Golden files list every screen's texts, so they meet dev's own texts all the time.
            # Those keep dev's side here and are blessed again on Linux (the bless job); the
            # copies then go to dev through a pull request, for a person to read the diff.
            if printf '%s\n' "$conflicted" | grep -qvE '^src/regression/snapshots/[^/]+\.snap$'; then
                git reset -q --hard HEAD
                git cherry-pick --quit 2>/dev/null || true
                note "\`$short\` $subject: doesn't apply to dev (conflicts in $(printf '%s' "$conflicted" | tr '\n' ' '))"
                continue
            fi
            printf '%s\n' "$conflicted" | xargs git checkout -q --ours --
            printf '%s\n' "$conflicted" | xargs git add --
            goldens=$(printf '%s' "$conflicted" | tr '\n' ' ')
            bless=true
        fi
        if git diff --cached --quiet; then
            echo "left alone (dev has it): $subject"
            git reset -q --hard HEAD
            continue
        fi
        if git diff --cached --name-only | grep -q '^\.github/workflows/'; then
            git reset -q --hard HEAD
            note "\`$short\` $subject: changes workflows, which the Actions token may not push"
            continue
        fi
        { git log -1 --format=%B "$c"; printf '\n(cherry picked from commit %s)\n' "$c"; } >"$RUNNER_TEMP/msg"
        git commit -q --no-verify -F "$RUNNER_TEMP/msg" --author="$(git log -1 --format='%an <%ae>' "$c")"
        echo "copied: $subject"
        list="$list- \`$short\` $subject"$'\n'
        if [ -n "$goldens" ]; then
            list="$list  - goldens to bless again: $goldens"$'\n'
        fi
    done
    report

    if [ -n "$list" ]; then
        git push -q -f origin "HEAD:refs/heads/$BRANCH"
        echo "picked=true" >>"$GITHUB_OUTPUT"
        echo "bless=$bless" >>"$GITHUB_OUTPUT"
        { echo "list<<EOF"; printf '%s' "$list"; echo "EOF"; } >>"$GITHUB_OUTPUT"
    else
        # Nothing to check: dev already has everything up to here.
        git push -q -f origin "$head:refs/tags/dev-synced"
        echo "picked=false" >>"$GITHUB_OUTPUT"
    fi
    ;;
bless)
    git fetch -q --no-tags origin "+refs/heads/$BRANCH:refs/remotes/origin/$BRANCH"
    git worktree add -q --detach "$RUNNER_TEMP/dev" "refs/remotes/origin/$BRANCH"
    cd "$RUNNER_TEMP/dev"
    # As docs/regression-tests.md blesses them; screen goldens are Linux-only, as this runner.
    # The checkout's target folder has the restored cache.
    TAIKO_BLESS=1 CARGO_TARGET_DIR="$GITHUB_WORKSPACE/target" cargo test --profile ci-test --locked --lib regression::
    git add -A src/regression/snapshots
    if git diff --cached --quiet; then
        echo "the goldens kept dev's side and still match"
        exit 0
    fi
    git diff --cached --stat
    git commit -q --no-verify -m "test(sync-to-dev): bless the goldens the copies conflicted in" \
        -m "Re-made on Linux by sync-to-dev.yml ($RUN_URL); the pull request is there to read the diff."
    git push -q origin "HEAD:refs/heads/$BRANCH"
    ;;
land)
    git fetch -q --no-tags origin "+refs/heads/$BRANCH:refs/remotes/origin/$BRANCH"
    git worktree add -q --detach "$RUNNER_TEMP/dev" "refs/remotes/origin/$BRANCH"
    cd "$RUNNER_TEMP/dev"
    if [ "${BLESS:-false}" = true ]; then
        # Re-made goldens are read by a person before they go in (AGENTS.md): a pull request,
        # its checks already passed in this run.
        body=$(printf 'Stable commits copied onto dev by %s. Their golden files conflicted with dev'"'"'s, so they kept dev'"'"'s side and were blessed again on Linux: **read the .snap diff before merging.** Pre-merge passed on Linux in that run; this pull request runs no checks of its own (the Actions token starts none).\n\n%s\n\nSquash-merge it soon: the next copies onto dev come without it.\n\nChangelog: skip\n' "$RUN_URL" "$LIST")
        if ! gh pr create --base dev --head "$BRANCH" --title "chore(sync-to-dev): bring stable's commits with re-made goldens into dev" --body "$body"; then
            note "copies with re-made goldens are on the branch \`$BRANCH\` (the Actions token may not open pull requests here): open one into dev and read the .snap diff"$'\n'"$LIST"
            report
        fi
        git push -q -f origin "$GITHUB_SHA:refs/tags/dev-synced"
        exit 0
    fi
    for attempt in 1 2 3; do
        # Dev moved while the checks ran: put the copies on top of it.
        if ! git merge-base --is-ancestor refs/remotes/origin/dev HEAD; then
            if ! git rebase -q refs/remotes/origin/dev; then
                git rebase --abort
                note "dev moved while the copies were checked, and they no longer apply on top of it: $RUN_URL"
                report
                exit 1
            fi
        fi
        if git push -q origin HEAD:refs/heads/dev; then
            git push -q -f origin "$GITHUB_SHA:refs/tags/dev-synced"
            git push -q origin --delete "$BRANCH" || true
            gh workflow run release.yml --ref dev
            exit 0
        fi
        git fetch -q --no-tags origin +refs/heads/dev:refs/remotes/origin/dev
    done
    note "couldn't push dev three times in a row: $RUN_URL"
    report
    exit 1
    ;;
failed)
    note "the copies onto dev failed Pre-merge on Linux (branch \`$BRANCH\`): $RUN_URL"$'\n'"$LIST"
    report
    exit 1
    ;;
esac
