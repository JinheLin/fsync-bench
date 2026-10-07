#!/usr/bin/env python3
"""Integration checks on an explicitly chosen filesystem; no latency assertions."""
import argparse
import csv
import json
import math
import os
import pathlib
import re
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent


def check(condition, message):
    if not condition:
        raise RuntimeError(message)


def run(cmd):
    return subprocess.check_output([str(p) for p in cmd], text=True, cwd=ROOT)


def rows(path):
    with path.open(newline="") as source:
        reader = csv.DictReader(source)
        result = list(reader)
        check(all(None not in r and None not in r.values() for r in result), "Malformed CSV")
        return result


def smoke(base, modes):
    with tempfile.TemporaryDirectory(prefix="verify-", dir=base) as tmp:
        tmp = pathlib.Path(tmp)
        data = tmp / "data"
        data.mkdir()
        sentinel = data / "existing-file"
        sentinel.write_text("preserve me")
        common = ["--dir", data, "--block-sizes", "4K,16K", "--iterations", "7",
                  "--warmup", "2", "--rounds", "4", "--file-size", "512K"]
        wal = tmp / "wal.csv"
        run([ROOT / "target/release/wal-bench", *common, "--layouts", "initialized,fallocate,append",
             "--io-modes", ",".join(modes), "--csv", wal])
        results = rows(wal)
        check(len(results) == 4 * 3 * 2 * len(modes) * 2, "Missing WAL cases")
        identities = {(r["round"], r["layout"], r["block_bytes"], r["io_mode"], r["sync_method"]) for r in results}
        check(len(identities) == len(results), "Duplicate WAL cases")
        combinations = [(mode, method) for mode in modes for method in ("fsync", "fdatasync")]
        for round_num in range(1, 5):
            for layout in ("initialized", "fallocate", "append"):
                for block in ("4096", "16384"):
                    group = [r for r in results if (r["round"], r["layout"], r["block_bytes"]) == (str(round_num), layout, block)]
                    offset = (round_num - 1) % len(combinations)
                    check([(r["io_mode"], r["sync_method"]) for r in group] == combinations[offset:] + combinations[:offset], "Case order did not rotate")
        for r in results:
            block = int(r["block_bytes"])
            check(int(r["writes"]) == int(r["sync_calls"]) == 7, "Every WAL write must sync")
            check(int(r["measured_bytes"]) == 7 * block, "Incorrect WAL byte count")
            check(int(r["segment_bytes"]) == (9 * block if r["layout"] == "append" else 512 * 1024), "Incorrect segment size")
            check(math.isclose(float(r["commits_s"]), 7 / float(r["elapsed_s"]), rel_tol=1e-5), "Incorrect WAL rate")
            check(math.isclose(float(r["commit_avg_us"]), float(r["write_avg_us"]) + float(r["sync_avg_us"]), abs_tol=2e-6), "Incorrect commit timing")
        buffered = tmp / "buffered.csv"
        run([ROOT / "target/release/fsync-bench", *common, "--workloads", "overwrite,fallocate,append", "--methods", "fsync,fdatasync,none", "--sync-every", "2", "--csv", buffered])
        original = rows(buffered)
        check(len(original) == 4 * 3 * 2 * 3, "Missing buffered cases")
        for r in original:
            check(int(r["writes"]) == 14, "Incorrect batch write count")
            check(int(r["sync_calls"]) == (0 if r["method"] == "none" else 7), "Incorrect batch sync count")
            check(int(r["measured_bytes"]) == 14 * int(r["block_bytes"]), "Incorrect batch byte count")
            check((r["sync_avg_us"] == "") == (r["method"] == "none"), "none must not claim sync latency")
        failed = subprocess.run([str(ROOT / "target/release/wal-bench"), "--methods", "none"], capture_output=True)
        check(failed.returncode != 0, "WAL accepted omitted sync")
        failed = subprocess.run([str(ROOT / "target/release/wal-bench"), *map(str, common), "--csv", str(wal)], capture_output=True)
        check(failed.returncode != 0 and len(rows(wal)) == len(results), "Existing CSV was overwritten")
        check(list(data.iterdir()) == [sentinel] and sentinel.read_text() == "preserve me", "Cleanup changed existing files or left scratch files")
        return {"wal_cases": len(results), "buffered_cases": len(original), "readback": "first and last measured records checked by WAL binary", "cleanup": "passed", "rotation": "passed"}


def trace_checks(base, modes):
    summaries = []
    with tempfile.TemporaryDirectory(prefix="trace-", dir=base) as tmp:
        tmp = pathlib.Path(tmp)
        for mode in modes:
            for method in ("fsync", "fdatasync"):
                trace = tmp / "trace.txt"
                run(["strace", "-qq", "-yy", "-s", "32", "-o", trace, "-e", "trace=openat,pwrite64,fsync,fdatasync,fallocate,fadvise64",
                     ROOT / "target/release/wal-bench", "--dir", tmp / "data", "--layouts", "append", "--block-sizes", "4K",
                     "--iterations", "7", "--warmup", "2", "--rounds", "1", "--io-modes", mode, "--methods", method])
                lines = trace.read_text().splitlines()
                opened = [line for line in lines if "openat(" in line and "segment.wal\"" in line and "O_RDWR" in line and "O_CREAT" not in line]
                check(len(opened) == 1 and ("O_DIRECT" in opened[0]) == (mode == "direct"), "Unexpected I/O open flags")
                writes = [(i, line) for i, line in enumerate(lines) if line.startswith("pwrite64(") and "segment.wal>" in line]
                check(len(writes) == 9, "Unexpected pwrite count (measurement + warmup)")
                for n, (i, line) in enumerate(writes):
                    check(re.search(r", 4096, " + str(n * 4096) + r"\)\s*= 4096$", line) is not None, "Short or nonsequential write")
                    check(lines[i+1].startswith(method + "(") and "segment.wal>" in lines[i+1] and lines[i+1].endswith("= 0"), "Write not immediately followed by selected sync")
                check(any("POSIX_FADV_DONTNEED" in line for line in lines), "Missing preparation cache advice")
                summaries.append({"io_mode": mode, "sync": method, "writes": 9, "write_then_sync": "passed", "open_flags": "passed"})
    return summaries


def extent_probe(base):
    fs = json.loads(run(["findmnt", "-J", "-T", base, "-o", "FSTYPE"]))["filesystems"][0]["fstype"]
    check(fs == "xfs", "--extents verifies XFS-specific observations; choose an XFS directory")
    with tempfile.TemporaryDirectory(prefix="extents-", dir=base) as tmp:
        path = pathlib.Path(tmp) / "probe.dat"
        fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_EXCL, 0o600)
        try:
            run(["fallocate", "-l", "64M", path])
            os.fsync(fd)
            before = run(["filefrag", "-v", path])
            check(os.pwrite(fd, bytes([17]) * 16384, 0) == 16384, "Short probe write")
            os.fdatasync(fd)
            after = run(["filefrag", "-v", path])
            zeros = bytes(1024 * 1024)
            for offset in range(0, 64 * 1024 * 1024, len(zeros)):
                check(os.pwrite(fd, zeros, offset) == len(zeros), "Short initialization write")
            os.fsync(fd)
            initialized = run(["filefrag", "-v", path])
            check(os.fstat(fd).st_size == 64 * 1024 * 1024, "Probe grew EOF")
        finally:
            os.close(fd)
        # Horizontal whitespace only: a row with empty flags must not consume the next row.
        pattern = r"^[ \t]*\d+:[ \t]+(\d+)\.\.[ \t]*(\d+):[ \t]*(\d+)\.\.[ \t]*(\d+):[ \t]*\d+:[ \t]*(.*)$"
        parse = lambda text: [m.groups() for m in re.finditer(pattern, text, re.MULTILINE)]
        b, a, done = map(parse, (before, after, initialized))
        check(b and a and done, "No parsed extent rows")
        check("unwritten" in b[0][-1] and "unwritten" not in a[0][-1], "Missing unwritten-to-written conversion")
        check(b[0][0] == a[0][0] == "0" and b[0][2] == a[0][2], "First physical mapping changed")
        check(any("unwritten" in row[-1] for row in a), "Unwritten tail missing")
        check(all("unwritten" not in row[-1] for row in done), "Full initialization left unwritten extents")
        report = "Independent XFS probe; outside all benchmark timings.\n64 MiB fallocate(mode=0) + fsync; then 16 KiB pwrite + fdatasync; then full 1 MiB zero writes + fsync.\nEOF remains 64 MiB. First physical mapping unchanged; full initialization removes unwritten flags.\n\n"
        for title, text in (("Preallocated", before), ("First write synced", after), ("Fully initialized", initialized)):
            report += title + ":\n" + text.replace(tmp, "<PROBE_DIR>") + "\n"
        return "\n".join(line.rstrip() for line in report.rstrip().splitlines()) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dir", type=pathlib.Path, required=True)
    parser.add_argument("--io-modes", choices=("buffered", "buffered,direct"), default="buffered,direct")
    parser.add_argument("--strace", action="store_true")
    parser.add_argument("--extents", action="store_true", help="Verify XFS unwritten extents with fallocate and filefrag")
    parser.add_argument("--output", type=pathlib.Path, help="New directory for JSON verification and optional extent report")
    args = parser.parse_args()
    base = args.dir.resolve()
    base.mkdir(parents=True, exist_ok=True)
    if args.output:
        args.output.mkdir(parents=True, exist_ok=False)
    result = {"smoke": smoke(base, args.io_modes.split(","))}
    if args.strace:
        result["syscalls"] = trace_checks(base, args.io_modes.split(","))
    if args.extents:
        report = extent_probe(base)
        result["extents"] = "passed: allocated physical blocks, first-write conversion, full initialization, fixed EOF"
        if args.output:
            (args.output / "extents.txt").write_text(report)
    serialized = json.dumps(result, indent=2) + "\n"
    if args.output:
        (args.output / "verification.json").write_text(serialized)
    print(serialized, end="")


if __name__ == "__main__":
    main()
