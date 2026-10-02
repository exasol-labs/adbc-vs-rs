#!/usr/bin/env python3
"""Combines the newest per-source benchmark CSVs into one Markdown table.

Reads the newest ``results-<source>-<stamp>.csv`` per source in BENCH_DIR
(the first argument, default ``dist/bench``), takes the median rows/sec of the
non-warm-up samples per (tier, adapter) exactly as ``report.rs`` does, and
writes ``summary-<UTC-stamp>.md`` beside them, echoing it to stdout.

Run after the bench binaries, never by them: each binary only writes its own
source's files, so no process has to coordinate with another. A source with
no CSV yet is left out with a note on stderr; one with no JDBC baseline
(Trino) shows ``n/a`` in the JDBC and ratio columns. Standard library only.
"""

import csv
import datetime
import glob
import os
import statistics
import sys

SOURCES = ("mysql", "postgres", "trino")


def newest_csv(bench_dir, source):
    # The stamp is an ISO-8601 UTC timestamp, so lexical order is time order.
    paths = sorted(glob.glob(os.path.join(bench_dir, f"results-{source}-*.csv")))
    return paths[-1] if paths else None


def medians_by_tier(path):
    """Returns {tier: {adapter: median rows/sec}}, tiers in first-seen order."""
    groups = {}
    with open(path, newline="") as handle:
        for row in csv.DictReader(handle):
            if row["warm_up"] == "true":
                continue
            groups.setdefault(row["tier"], {}).setdefault(row["adapter"], []).append(
                float(row["rows_per_sec"])
            )
    return {
        tier: {adapter: statistics.median(values) for adapter, values in by_adapter.items()}
        for tier, by_adapter in groups.items()
    }


def render(inputs):
    lines = [
        "# adbc-vs Benchmark Summary",
        "",
        "Median rows/sec over the measured (non-warm-up) runs of each input:",
        "",
    ]
    lines += [f"- `{os.path.basename(path)}`" for _, path in inputs]
    lines += [
        "",
        "| Source | Tier | ADBC rows/sec | JDBC rows/sec | adbc ÷ jdbc |",
        "|---|---|---|---|---|",
    ]
    for source, path in inputs:
        for tier, by_adapter in medians_by_tier(path).items():
            adbc = by_adapter.get("adbc")
            jdbc = by_adapter.get("jdbc")
            ratio = f"{adbc / jdbc:.3f}" if adbc and jdbc else "n/a"
            adbc_text = f"{adbc:.0f}" if adbc else "n/a"
            jdbc_text = f"{jdbc:.0f}" if jdbc else "n/a"
            lines.append(f"| {source} | {tier} | {adbc_text} | {jdbc_text} | {ratio} |")
    return "\n".join(lines) + "\n"


def main(argv):
    bench_dir = argv[1] if len(argv) > 1 else "dist/bench"
    inputs = []
    for source in SOURCES:
        path = newest_csv(bench_dir, source)
        if path is None:
            print(f"no results-{source}-*.csv in {bench_dir}; {source} left out", file=sys.stderr)
        else:
            inputs.append((source, path))
    if not inputs:
        sys.exit(f"no results-<source>-*.csv in {bench_dir}; run a bench-<source> target first")

    text = render(inputs)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H-%M-%SZ")
    out_path = os.path.join(bench_dir, f"summary-{stamp}.md")
    with open(out_path, "w") as handle:
        handle.write(text)
    sys.stdout.write(text)
    print(f"wrote {out_path}", file=sys.stderr)


if __name__ == "__main__":
    main(sys.argv)
