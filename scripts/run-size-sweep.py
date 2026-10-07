#!/usr/bin/env python3
"""Compare complete commits at several write sizes, using both benchmarks."""
import argparse
import collections
import csv
import datetime
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
COMBINATIONS = [(mode, method) for mode in ("buffered", "direct")
                for method in ("fsync", "fdatasync")]


def read_rows(path):
    with path.open(newline="") as source:
        return list(csv.DictReader(source))


def mean(rows, field):
    return sum(float(r[field]) * int(r["writes"]) for r in rows) / sum(int(r["writes"]) for r in rows)


def summarize(output, sizes, manifest):
    environment = json.loads((output / f"{sizes[0]}K/environment.json").read_text())
    aggregate = []
    for size in sizes:
        directory = output / f"{size}K"
        pooled = {}
        for line in (directory / "wal.txt").read_text().splitlines():
            values = line.split()
            if values and values[0] == "all":
                pooled[tuple(values[1:5])] = values
        for benchmark in ("buffered", "wal"):
            groups = collections.defaultdict(list)
            for row in read_rows(directory / f"{benchmark}.csv"):
                key = ((row["workload"], "buffered", row["method"]) if benchmark == "buffered"
                       else (row["layout"], row["io_mode"], row["sync_method"]))
                groups[key].append(row)
            for (layout, mode, method), rows in groups.items():
                writes = sum(int(r["writes"]) for r in rows)
                syncs = sum(int(r["sync_calls"]) for r in rows)
                if writes != syncs or len(rows) != manifest["rounds"]:
                    raise RuntimeError("Unexpected round or sync count")
                if any(int(r["block_bytes"]) != size * 1024 or int(r["writes"]) != manifest["iterations"] for r in rows):
                    raise RuntimeError("Unexpected record size or write count")
                elapsed = sum(float(r["elapsed_s"]) for r in rows)
                measured_bytes = sum(int(r["measured_bytes"]) for r in rows)
                write = mean(rows, "write_batch_avg_us" if benchmark == "buffered" else "write_avg_us")
                sync = mean(rows, "sync_avg_us")
                commit = write + sync if benchmark == "buffered" else mean(rows, "commit_avg_us")
                if abs(commit - write - sync) > 2e-6 or measured_bytes != writes * size * 1024:
                    raise RuntimeError("Inconsistent timing or byte count")
                quantiles = pooled[(layout, str(size * 1024), mode, method)] if benchmark == "wal" else None
                aggregate.append({
                    "benchmark": benchmark, "layout": layout, "block_kib": size,
                    "io_mode": mode, "sync_method": method, "rounds": len(rows),
                    "writes": writes, "sync_calls": syncs,
                    "segment_bytes": (manifest["iterations"] + manifest["warmup"]) * size * 1024,
                    "measured_bytes": measured_bytes, "elapsed_s": elapsed,
                    "commits_s": writes / elapsed, "mib_s": measured_bytes / elapsed / 2**20,
                    "write_avg_us": write, "sync_avg_us": sync, "commit_avg_us": commit,
                    "commit_p50_us": float(quantiles[9]) if quantiles else "",
                    "commit_p99_us": float(quantiles[11]) if quantiles else "",
                    "commit_max_us": float(quantiles[12]) if quantiles else "",
                })
    with (output / "aggregate.csv").open("w", newline="") as dest:
        writer = csv.DictWriter(dest, fieldnames=list(aggregate[0]), lineterminator="\n")
        writer.writeheader()
        writer.writerows(aggregate)
    lines = [
        "# Write-size sweep: complete write + sync latency", "",
        f"Linux {environment['kernel']}; {environment['filesystem']['fstype']}; "
        f"{environment['device_queue'].get('model', 'unknown device')}; {environment['rustc']}.", "",
        f"Sizes (KiB): {', '.join(map(str, sizes))}. {manifest['iterations']} measured writes "
        f"+ {manifest['warmup']} warmup writes per case per round; {manifest['rounds']} rounds. "
        "ONE explicit sync after EVERY write; release binaries; sequential, single writer.", "",
        "Fixed file capacity for each size is (iterations + warmup) × block bytes. "
        "The capacity is identical across methods at that size; all measured writes stay "
        "inside initialized/fallocated files without growing EOF or wrapping. Capacity "
        "varies between sizes, so this experiment represents correspondingly sized segments.", "",
        "Means use all measured commits; throughput uses total bytes/time. WAL p99 comes "
        "from pooled complete-commit samples, rounded to 0.01 us. No per-round percentile "
        "is averaged, and no write percentile is added to a sync percentile. Setup, "
        "warmup, final drain, readback and cleanup are excluded. Other processes' I/O "
        f"was not isolated. Device queue write_cache={environment['device_queue'].get('write_cache', 'unknown')!r}, "
        "as recorded in each environment.json.", "",
        "## WAL: buffered vs direct", "",
    ]
    index = {(r["benchmark"], r["layout"], r["block_kib"], r["io_mode"], r["sync_method"]): r for r in aggregate}
    headings = "| Write KiB | buffered fsync | buffered fdatasync | direct fsync | direct fdatasync |"
    for layout in ("append", "fallocate", "initialized"):
        lines.extend([f"### {layout}", ""])
        for field, description in (("commit_avg_us", "Mean write + sync (us)"),
                                   ("commit_p99_us", "Complete commit p99 (us)"),
                                   ("mib_s", "Throughput (MiB/s)")):
            lines.extend([description + ":", "", headings, "|---:|---:|---:|---:|---:|"])
            for size in sizes:
                values = [index[("wal", layout, size, mode, method)][field] for mode, method in COMBINATIONS]
                lines.append(f"| {size} | " + " | ".join(f"{value:.2f}" for value in values) + " |")
            lines.append("")
    lines.extend([
        "## Buffered micro-benchmark: the four original situations", "",
        "Mean write + sync (us), using fsync for append/fallocate. Both sync methods "
        "for every state are available in aggregate.csv; initialized means overwrite here.", "",
        "| Write KiB | append + fsync | fallocate first write + fsync | initialized + fsync | initialized + fdatasync |",
        "|---:|---:|---:|---:|---:|",
    ])
    for size in sizes:
        keys = [("append", "fsync"), ("fallocate", "fsync"), ("overwrite", "fsync"), ("overwrite", "fdatasync")]
        values = [index[("buffered", layout, size, "buffered", method)]["commit_avg_us"] for layout, method in keys]
        lines.append(f"| {size} | " + " | ".join(f"{value:.2f}" for value in values) + " |")
    lines.extend([
        "", "## Raw results and reproduction", "",
        "[aggregate.csv](aggregate.csv) contains means, rates and WAL pooled quantiles "
        "for all combinations. [manifest.json](manifest.json) records the sweep "
        "parameters and benchmark source commit. Each size's subdirectory contains "
        "the per-round CSV, stdout, environment, exact command templates and full report.", "",
    ])
    for size in sizes:
        lines.append(f"- [{size} KiB]({size}K/REPORT.md)")
    lines.append("")
    (output / "REPORT.md").write_text("\n".join(lines))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dir", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True, help="New result directory")
    parser.add_argument("--sizes-kib", default="32,64,128,256,512,1024")
    parser.add_argument("--iterations", type=int, default=2000)
    parser.add_argument("--warmup", type=int, default=100)
    parser.add_argument("--rounds", type=int, default=8)
    args = parser.parse_args()
    sizes = [int(s) for s in args.sizes_kib.split(",")]
    if not sizes or len(set(sizes)) != len(sizes) or any(s <= 0 or s % 4 for s in sizes):
        parser.error("Sizes must be distinct positive multiples of 4 KiB")
    if args.iterations <= 0 or args.rounds <= 0 or args.warmup < 0:
        parser.error("Iterations/rounds must be positive; warmup must be nonnegative")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    manifest = {
        "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "sizes_kib": sizes, "iterations": args.iterations, "warmup": args.warmup,
        "rounds": args.rounds, "fixed_segment_policy": "(iterations + warmup) * block_bytes",
        "benchmark_source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "command_template": "python3 scripts/run-size-sweep.py --dir <TARGET_DIR> --output <OUTPUT_DIR> "
                            f"--sizes-kib {args.sizes_kib} --iterations {args.iterations} --warmup {args.warmup} --rounds {args.rounds}",
    }
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    for size in sizes:
        capacity = (args.iterations + args.warmup) * size * 1024
        print(f"Starting {size} KiB; fixed file capacity {capacity} bytes", flush=True)
        subprocess.run([
            sys.executable, str(ROOT / "scripts/run-benchmarks.py"),
            "--dir", str(args.dir.resolve()), "--output", str(output / f"{size}K"),
            "--block-sizes", f"{size}K", "--iterations", str(args.iterations),
            "--warmup", str(args.warmup), "--rounds", str(args.rounds), "--file-size", str(capacity),
        ], check=True, cwd=ROOT)
    manifest["finished_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    summarize(output, sizes, manifest)
    print(f"Sweep complete: {output / 'REPORT.md'}", flush=True)


if __name__ == "__main__":
    main()
