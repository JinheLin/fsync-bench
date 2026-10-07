#!/usr/bin/env python3
"""Run both release binaries sequentially and produce a portable result bundle."""
import argparse
import collections
import csv
import datetime
import json
import pathlib
import platform
import shlex
import subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent


def command(args):
    return subprocess.check_output(args, text=True, cwd=ROOT)


def environment(target):
    fs = json.loads(command([
        "findmnt", "-J", "-T", str(target), "-o", "SOURCE,FSTYPE,OPTIONS"
    ]))["filesystems"][0]
    device = pathlib.Path(fs["source"].split("[")[0]).name
    queue = pathlib.Path("/sys/class/block") / device / "queue"
    device_info = {}
    for field in ("write_cache", "logical_block_size", "physical_block_size", "rotational", "fua"):
        path = queue / field
        if path.exists():
            device_info[field] = path.read_text().strip()
    model = pathlib.Path("/sys/class/block") / device / "device/model"
    if model.exists():
        device_info["model"] = model.read_text().strip()
    cpu = next((line.split(":", 1)[1].strip()
                for line in pathlib.Path("/proc/cpuinfo").read_text().splitlines()
                if line.startswith("model name")), "unknown")
    return {
        "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "kernel": platform.release(), "architecture": platform.machine(),
        "rustc": command(["rustc", "--version"]).strip(),
        "cpu": cpu, "filesystem": fs, "device_queue": device_info,
        "build": "cargo build --release --offline (proxy environment unset)",
        "other_io_isolated": False,
        "path_policy": "Target and result paths are replaced by placeholders in saved output."
    }


def grouped_rows(path, keys):
    groups = collections.defaultdict(list)
    with path.open(newline="") as source:
        for row in csv.DictReader(source):
            groups[tuple(row[k] for k in keys)].append(row)
    return groups


def weighted_mean(rows, value, weight="writes"):
    return sum(float(r[value]) * int(r[weight]) for r in rows) / sum(int(r[weight]) for r in rows)


def summary(output, info):
    params = info["parameters"]
    lines = [
        "# Benchmark run", "",
        f"Started (UTC): {info['started_utc']}. Linux {info['kernel']}, "
        f"{info['filesystem']['fstype']}, {info['device_queue'].get('model', 'unknown device')}.", "",
        f"Release build; block sizes {params['block_sizes']}; {params['iterations']} measured "
        f"writes + {params['warmup']} warmup writes per case per round; "
        f"{params['rounds']} rounds; fixed segments {params['file_size']}; one sync per write.", "",
        "Setup, warmup, initialization, final drain, readback and cleanup are excluded. "
        "Other processes' I/O was not isolated. Means are weighted by operation counts; "
        "throughput uses total bytes / total time. WAL p99 is from pooled samples in stdout, "
        "rounded to 0.01 us; per-round p99 values are never averaged.", "",
        "## Buffered micro-benchmark", "",
        "`overwrite` is fully written and synced before measurement. `fallocate` fixes EOF "
        "but leaves unwritten extents on XFS. `append` grows an initially empty file.", "",
        "| Workload | Bytes/write | Sync | Mean write us | Mean sync us | Mean write+sync us | MiB/s |",
        "|---|---:|---|---:|---:|---:|---:|",
    ]
    for (workload, block, method), rows in grouped_rows(output / "buffered.csv", ["workload", "block_bytes", "method"]).items():
        write = weighted_mean(rows, "write_batch_avg_us")
        sync = weighted_mean(rows, "sync_avg_us")
        rate = sum(int(r["measured_bytes"]) for r in rows) / sum(float(r["elapsed_s"]) for r in rows) / 2**20
        lines.append(f"| {workload} | {block} | {method} | {write:.2f} | {sync:.2f} | {write+sync:.2f} | {rate:.2f} |")
    lines.extend([
        "", "## WAL: buffered vs direct", "",
        "Both modes use the same aligned record, offset and byte count; every write is "
        "immediately followed by the selected sync. Compare full commit latency, since "
        "direct I/O can move time from the sync call into the write call.", "",
        "| Layout | Bytes/write | I/O | Sync | Mean write us | Mean sync us | Mean commit us | Commit p99 us | MiB/s |",
        "|---|---:|---|---|---:|---:|---:|---:|---:|",
    ])
    pooled_p99 = {}
    for line in (output / "wal.txt").read_text().splitlines():
        values = line.split()
        if values and values[0] == "all":
            pooled_p99[tuple(values[1:5])] = float(values[11])
    for key, rows in grouped_rows(output / "wal.csv", ["layout", "block_bytes", "io_mode", "sync_method"]).items():
        layout, block, mode, method = key
        write = weighted_mean(rows, "write_avg_us")
        sync = weighted_mean(rows, "sync_avg_us")
        commit = weighted_mean(rows, "commit_avg_us")
        rate = sum(int(r["measured_bytes"]) for r in rows) / sum(float(r["elapsed_s"]) for r in rows) / 2**20
        lines.append(f"| {layout} | {block} | {mode} | {method} | {write:.2f} | {sync:.2f} | {commit:.2f} | {pooled_p99[key]:.2f} | {rate:.2f} |")
    lines.extend([
        "", "## Interpretation", "",
        "The four scenario descriptions in the README explain possible filesystem work. "
        "These latency differences do not isolate the cost of an individual inode field, "
        "extent-tree operation, journal transaction or block-layer flush.", "",
        f"The target queue reports write_cache={info['device_queue'].get('write_cache', 'unknown')!r}. "
        "A short direct-I/O sync must be interpreted alongside write latency and device "
        "cache behavior. This benchmark checks syscall success and data readback, "
        "not persistence under power failure.", "",
        "Raw per-round results: [buffered.csv](buffered.csv), [wal.csv](wal.csv). "
        "Pooled results: [wal.txt](wal.txt). Environment and exact command templates: "
        "[environment.json](environment.json).", "",
    ])
    (output / "REPORT.md").write_text("\n".join(lines))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dir", type=pathlib.Path, required=True, help="Directory on the disk being measured")
    parser.add_argument("--output", type=pathlib.Path, required=True, help="New result directory (will not overwrite)")
    parser.add_argument("--block-sizes", default="16K")
    parser.add_argument("--iterations", type=int, default=2000)
    parser.add_argument("--warmup", type=int, default=100)
    parser.add_argument("--rounds", type=int, default=8)
    parser.add_argument("--file-size", default="64M")
    args = parser.parse_args()
    target = args.dir.resolve()
    output = args.output.resolve()
    for binary in ("fsync-bench", "wal-bench"):
        if not (ROOT / "target/release" / binary).is_file():
            parser.error("Build both release binaries with cargo build --release --offline first.")
    output.mkdir(parents=True, exist_ok=False)
    target.mkdir(parents=True, exist_ok=True)
    info = environment(target)
    info["parameters"] = {k: getattr(args, k) for k in ("block_sizes", "iterations", "warmup", "rounds", "file_size")}
    shared = ["--dir", str(target), "--block-sizes", args.block_sizes,
              "--iterations", str(args.iterations), "--warmup", str(args.warmup),
              "--rounds", str(args.rounds), "--file-size", args.file_size]
    info["commands"] = []
    for binary, name, extra in (
        ("fsync-bench", "buffered", ["--workloads", "append,fallocate,overwrite", "--sync-every", "1"]),
        ("wal-bench", "wal", ["--layouts", "append,fallocate,initialized", "--io-modes", "buffered,direct", "--alignment", "4K"]),
    ):
        cmd = [str(ROOT / "target/release" / binary), *shared, *extra,
               "--methods", "fsync,fdatasync", "--csv", str(output / (name + ".csv"))]
        public_cmd = [p.replace(str(target), "<TARGET_DIR>").replace(str(output), "<OUTPUT_DIR>")
                      .replace(str(ROOT), ".") for p in cmd]
        info["commands"].append(shlex.join(public_cmd))
        print(f"Running {binary}...", flush=True)
        stdout = command(cmd)
        stdout = stdout.replace(str(target), "<TARGET_DIR>").replace(str(output), "<OUTPUT_DIR>").replace(str(ROOT), "<REPOSITORY>")
        (output / (name + ".txt")).write_text(stdout)
    info["finished_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    (output / "environment.json").write_text(json.dumps(info, indent=2) + "\n")
    summary(output, info)
    print(f"Saved CSV, stdout, environment and report in {output}")


if __name__ == "__main__":
    main()
