#!/usr/bin/env python3
"""Compares `perf_audit` runs of a pull request's base and head (docs/performance-audit.md).

Each log is the output of
  cargo test --profile perf-audit --locked --lib perf_audit:: -- --ignored --nocapture --test-threads=1
whose cases print `AUDIT <name>: p50_us=<x> p95_us=<y>` (timings) or `AUDIT <name>: <n>` (counts,
such as shapes drawn). Several logs per side are runs of the same build, taken in turns with the
other side's on the same machine. A case's value is the lowest of its runs' p50s: noise only ever
adds time, and on identical builds the lowest stays within 5 % where the median doesn't.

A case fails when head is more than THRESHOLD slower than base and also more than FLOOR_US
slower (tiny cases jitter by a large fraction). Cases that time threads waking, locks or the disk
are reported but never fail (NOISY). Cases only one side has are listed as new or gone. Exits 1
on a failure unless WAIVED=true (the pull request's `perf-regression-ok` label).
"""

import argparse
import os
import re
import statistics
import sys

TIMING = re.compile(r'AUDIT (\S+): p50_us=([0-9.]+) p95_us=([0-9.]+)')
COUNT = re.compile(r'AUDIT (\S+): ([0-9]+)\s*$')

# Timings of other threads, locks and the file system: too noisy on shared runners to gate on.
# Sorting 50,000 titles moves ~20 ms of allocations: on runners it read +12 % and +30 % (over
# five rounds) on code that doesn't touch it, and the same on a re-run; the 10,000 case gates the
# same sort.
NOISY = ('skin/async/worker-completion', 'mutex/', 'hash/first/', 'game/prepare/',
         'select/title-sort/50000')


def load(paths):
    timings, counts = {}, {}
    for path in paths:
        with open(path, encoding='utf8', errors='replace') as f:
            for line in f:
                m = TIMING.search(line)
                if m:
                    timings.setdefault(m[1], []).append((float(m[2]), float(m[3])))
                    continue
                m = COUNT.search(line)
                if m:
                    counts.setdefault(m[1], set()).add(int(m[2]))
    return timings, counts


def best(runs):
    return min(r[0] for r in runs)


def median(runs, i):
    return statistics.median(r[i] for r in runs)


def fmt(us):
    return f'{us:,.1f}' if us >= 10 else f'{us:.3f}'


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--base', nargs='+', required=True)
    ap.add_argument('--head', nargs='+', required=True)
    ap.add_argument('--threshold', type=float, default=float(os.environ.get('THRESHOLD', '0.10')))
    ap.add_argument('--floor-us', type=float, default=float(os.environ.get('FLOOR_US', '2')))
    args = ap.parse_args()

    base, base_counts = load(args.base)
    head, head_counts = load(args.head)
    rows, failures, notes = [], [], []
    for name in sorted(set(base) | set(head)):
        if name not in base:
            rows.append((name, '', fmt(best(head[name])), '', 'new'))
            continue
        if name not in head:
            rows.append((name, fmt(best(base[name])), '', '', 'gone'))
            continue
        b, h = best(base[name]), best(head[name])
        change = (h - b) / b if b > 0 else 0.0
        verdict = ''
        if change > args.threshold and h - b > args.floor_us:
            if name.startswith(NOISY):
                verdict = 'slower (noisy, not gated)'
            else:
                verdict = '**SLOWER**'
                failures.append(name)
        elif change < -args.threshold and b - h > args.floor_us:
            verdict = 'faster'
        rows.append((
            name,
            f'{fmt(b)} (p95 {fmt(median(base[name], 1))})',
            f'{fmt(h)} (p95 {fmt(median(head[name], 1))})',
            f'{change:+.1%}',
            verdict,
        ))
    for name in sorted(set(base_counts) & set(head_counts)):
        if base_counts[name] != head_counts[name]:
            notes.append(f'- `{name}`: {sorted(base_counts[name])} -> {sorted(head_counts[name])}')

    out = [
        '## Perf gate',
        '',
        f'Lowest p50 of {len(args.base)} base and {len(args.head)} head runs (p95: median), '
        f'in microseconds. '
        f'Fails at more than {args.threshold:.0%} and {args.floor_us:g} us slower.',
        '',
        '| case | base | head | change | |',
        '|---|---:|---:|---:|---|',
    ]
    out += [f'| `{n}` | {b} | {h} | {c} | {v} |' for n, b, h, c, v in rows]
    if notes:
        out += ['', 'Counts that changed (shapes drawn and the like):', ''] + notes
    waived = os.environ.get('WAIVED', '').lower() == 'true'
    if failures:
        out += ['', f'**{len(failures)} case(s) slower:** ' + ', '.join(f'`{f}`' for f in failures)]
        if waived:
            out += ['', 'Waived by the `perf-regression-ok` label.']
    elif not rows:
        out += ['', 'No cases in common (the base has no perf audit yet).']
    text = '\n'.join(out) + '\n'
    print(text)
    summary = os.environ.get('GITHUB_STEP_SUMMARY')
    if summary:
        with open(summary, 'a', encoding='utf8') as f:
            f.write(text)
    return 1 if failures and not waived else 0


if __name__ == '__main__':
    sys.exit(main())
