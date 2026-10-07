# fsync-bench

Rust micro-benchmarks for Linux `fsync`, `fdatasync` and `fallocate`, including a single-writer WAL comparison of buffered I/O and `O_DIRECT`. No third-party Rust dependencies. Linux x86_64, Rust 1.77+; MIT licensed.

[中文说明](README.zh-CN.md) · [WAL details](WAL-BENCH.md) · [Latest results](results/2026-10-07/REPORT.md) · [Historical results](results/historical/README.md)

## Build and reproduce

Build offline without proxy settings:

```sh
env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY \
    -u http_proxy -u https_proxy -u all_proxy \
    -u CARGO_HTTP_PROXY cargo build --release --offline

# Choose a directory on the disk/filesystem you want to measure.
# The output directory must be new. Requires Python 3.9+ and findmnt.
python3 scripts/run-benchmarks.py \
    --dir /path/on/test/disk --output results/my-run
```

The script runs both binaries sequentially with 16 KiB blocks, 2,000 measured writes plus 100 warmup writes per case per round, 8 rounds, 64 MiB fixed segments, and one sync per write. It saves per-round CSV, pooled WAL statistics, environment, commands and a report. Parameters can be overridden; use `--help`. Build in release mode for performance measurements.

Each case creates an independent file in a newly created child of `--dir`. Setup, initialization, directory syncs, warmup, final drain, readback and deletion are outside the measured interval. Existing files are preserved; output CSV files are never overwritten. Forced termination can leave the benchmark's temporary directory behind.

## The four situations

These describe possible work in the filesystem, rather than a trace proving that each listed operation happens on every commit:

| Situation | File state / potential work |
|---|---|
| ① Append + allocation | Initially empty file; sequential writes grow EOF and may allocate blocks and change extent mappings, inode size and timestamps. |
| ② Fallocate + first write | `fallocate(mode=0)` sets EOF and reserves blocks before measurement. On XFS, first writes convert unwritten extents and persist necessary mapping metadata; timestamps also change. |
| ③ Initialized + overwrite + fsync | Whole fixed-size file actually written and synced first; overwrites avoid EOF growth and first-write conversion. `fsync` also requests file metadata synchronization. |
| ④ Initialized + overwrite + fdatasync | Same initialized file state; `fdatasync` can skip metadata not needed to retrieve the data, such as timestamps, but must still persist any necessary metadata. |

`fallocate` reserves actual space on this XFS filesystem. An unwritten extent means reads return zeros until initialization; it does not mean no physical blocks were allocated. The [extent probe](results/2026-10-07/extents.txt) shows the first physical mapping remains unchanged while its flag changes. Fully writing the segment in 1 MiB zero-filled chunks and syncing it removes unwritten flags before timing. `fsync` alone does not initialize these extents; `FALLOC_FL_ZERO_RANGE` commonly creates unwritten extents too.

Semantics: [fsync(2)](https://man7.org/linux/man-pages/man2/fsync.2.html), [fallocate(2)](https://man7.org/linux/man-pages/man2/fallocate.2.html), [Linux iomap mappings](https://docs.kernel.org/filesystems/iomap/design.html).

## Measured results

Buffered micro-benchmark, 16 KiB, one sync per write, Intel SSDPE2KX040T8 NVMe / XFS / Linux 5.14. Values below are **mean sync-call latency**, excluding the preceding write:

| File state | Sync | Earlier run, 2026-10-04 (us) | Rerun, 2026-10-07 (us) |
|---|---|---:|---:|
| Append | fsync / fdatasync | 38.95 / 38.85 | 38.27 / 38.26 |
| Fallocate, first write | fsync / fdatasync | 38.47 / 38.72 | 38.68 / 38.52 |
| Initialized overwrite | fsync | 25.94 | 24.15 |
| Initialized overwrite | fdatasync | 17.57 | 17.33 |

The earlier approximate 39 / 39 / 26 / 18 us pattern is reproduced within run-to-run variation. The historical run used 6 rounds; the rerun uses 8. Initialized overwrites separate the two sync methods more clearly here. Differences do not measure the isolated cost of an inode field, an extent-tree operation or a journal transaction. No block-level profiling or power-failure test is performed.

For WAL comparisons, use **write + sync commit latency** and commits/s: direct I/O can shift waiting into `pwrite`. On the rerun, initialized buffered fsync / fdatasync mean commits were 36.13 / 31.61 us, compared with 23.33 / 16.50 us for direct. Ordinary append mean commits were about 55–57 us in both modes. All 12 combinations, per-round data and pooled p99 values are in the [full report](results/2026-10-07/REPORT.md).

These are observations on this machine, with other processes' I/O not isolated. Its device queue reports `write through`; the 0.41 us initialized direct `fdatasync` average includes an actual sync call after every write, verified separately with strace. Interpret it alongside the 16.10 us mean write time and [Linux write-cache handling](https://docs.kernel.org/block/writeback_cache_control.html), rather than treating it as a general storage guarantee.

## Binaries

```sh
# Buffered experiments, including optional group commit.
./target/release/fsync-bench --dir /path/on/test/disk \
    --workloads append,fallocate,overwrite --block-sizes 16K \
    --iterations 2000 --warmup 100 --rounds 8 --file-size 64M \
    --sync-every 1 --methods fsync,fdatasync --csv buffered.csv

# WAL: every write followed immediately by fsync or fdatasync.
./target/release/wal-bench --dir /path/on/test/disk \
    --layouts append,fallocate,initialized --block-sizes 16K \
    --iterations 2000 --warmup 100 --rounds 8 --file-size 64M \
    --io-modes buffered,direct --methods fsync,fdatasync --csv wal.csv

./target/release/fsync-bench --help
./target/release/wal-bench --help
```

`fsync-bench` has `--sync-every N` for group commit, and an optional `--methods none` baseline without per-batch durability. Its `overwrite` workload writes the entire file during preparation and wraps if necessary. `wal-bench` has no batching or unsynced mode, and never wraps. Its `initialized` preparation uses fallocate followed by full zero writes, a sync, and file-specific `POSIX_FADV_DONTNEED` advice. `fallocate` layouts in both binaries reject configurations whose warmup plus measured writes exceed the fixed length.

## Verification and code layout

```sh
# Run Rust checks with proxy variables unset, as above.
cargo test --all-targets --offline
cargo clippy --all-targets --offline -- -D warnings

# Integration checks on your chosen target; optionally verify syscalls and XFS extents.
# strace, fallocate and filefrag are required for the optional checks.
python3 scripts/verify.py --dir /path/on/test/disk --strace --extents
```

The local rerun passed 8 Rust tests, 96 WAL integration cases, 72 buffered integration cases, four syscall checks and the XFS extent probe. Integration checks cover counts, rotation, rates, record readback, fixed EOF, CSV preservation and cleanup. CI checks stable Rust and the declared minimum Rust version; CI uses buffered integration checks without making performance claims about its runner.

- `src/main.rs`: buffered workloads and group commit.
- `src/bin/wal-bench.rs`: aligned WAL writes, explicit sync, timing and readback.
- `src/linux.rs`: shared Linux allocation/sync/cache-advice and aligned memory helpers.
- `src/cli.rs`, `src/stats.rs`: checked arguments and nearest-rank statistics.
- `scripts/`: reproducible runs, reports and integration verification.

Throughput uses total measured bytes/time, including sampling overhead. Mean write, sync and commit timings measure their respective intervals; WAL pooled percentiles are computed from all raw samples, never by averaging per-round percentiles. CSV rows preserve per-round statistics, not individual latency samples.
