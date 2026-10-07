# WAL benchmark details

`wal-bench` simulates one sequential WAL writer. Every `pwrite` is immediately followed by exactly one explicit `fsync` or `fdatasync`; the next record begins after the sync succeeds. Defaults test buffered/direct × fsync/fdatasync. There is no group commit, skipped sync, `O_SYNC` or `O_DSYNC` mode.

Buffered means Linux page cache I/O, without a userspace `BufWriter` around the data file. Direct opens with `O_DIRECT`. Both use the same aligned memory, bytes and offsets.

## File preparation

| Layout | Before timing | During timing |
|---|---|---|
| `initialized` (default) | `fallocate(mode=0)` → write zeros throughout segment in 1 MiB chunks → `fsync` | Sequential overwrite of initialized extents, fixed EOF. |
| `fallocate` | `fallocate(mode=0)` → `fsync` | First writes at new offsets, fixed EOF; unwritten conversion on XFS. |
| `append` | Create empty file → `fsync` | Sequential EOF growth; allocation and size metadata may be needed. |

Preparation issues `posix_fadvise(POSIX_FADV_DONTNEED)` for that file's clean pages after syncing; this is advisory, not a global cache drop. The preparation descriptor closes before the measured descriptor opens. Directory syncs, preparation, warmup, extra drains, readback and deletion are excluded from performance metrics.

Offsets never wrap: `(warmup + iterations) × block_bytes` must fit in a fixed segment. The standard reproduction uses 2,100 × 16 KiB = 32.8125 MiB inside a 64 MiB segment. Every case starts with an independently prepared file. Four combination orders rotate each round; 4 or 8 rounds balance positions when selecting all four. Layout and block-size order is fixed, so long-term environmental drift can still affect comparisons.

## Alignment and records

`--alignment` defaults to 4 KiB and must be a power of two, at least 16. Block sizes must be multiples of alignment. Buffer address, size and offset are aligned; the benchmark performs no implicit padding. Actual direct-I/O requirements depend on the filesystem, device and kernel. Use `--alignment 512` or another suitable value when appropriate. Errors terminate the case; the program does not request a buffered fallback. Some filesystems can ignore `O_DIRECT`, so validate behavior on the target rather than inferring it from a successful open alone.

Each record contains a little-endian sequence (first 8 bytes), end LSN (next 8 bytes) and a deterministic pre-generated payload. Only the header changes in the timed loop; payload generation and sample-buffer allocation are outside timing. Both modes retry `EINTR`; a short WAL write fails the case, preventing an unaligned retry or a partial record being counted as committed.

After syncing and closing the writing descriptor, a fresh buffered descriptor reads the first and last measured records, checks sequence/LSN/full payload and checks final EOF. The program does not implement CRCs, recovery, concurrency, transaction execution or segment rotation; it measures the write-and-sync path with aligned fixed-size records.

## Parameters

```sh
./target/release/wal-bench --help
./target/release/wal-bench --dir /path/on/test/disk \
    --block-sizes 16K --iterations 2000 --warmup 100 --rounds 8 \
    --file-size 64M --layouts initialized,fallocate,append \
    --io-modes buffered,direct --methods fsync,fdatasync \
    --alignment 4K --csv wal-results.csv
```

Sizes accept binary `K/KiB`, `M/MiB`, `G/GiB` suffixes or plain byte counts. Options support `--key=value`. Defaults are 4K/16K/64K, 500 measured and 50 warmup records, 4 rounds, initialized layout and all four combinations. CSV paths must be new. Temporary files are removed on normal exit or ordinary error returns; forced termination can leave `wal-bench-<pid>-<timestamp>` directories.

## Metrics

- Mean write: header update and `pwrite`, including retries if any.
- Mean sync: selected sync operation, with an adjacent timestamp on either side.
- Commit: write + sync, ending when sync succeeds.
- MiB/s / commits/s: total measured bytes / records divided by total measured loop time, including sampling overhead.
- Commit p50/p95/p99/max: nearest-rank statistics; the final `all` rows pool every raw sample from all rounds.

CSV contains 27 fields including measured write/sync counts, bytes, alignment, segment size, warmup, elapsed time, throughput and per-round write/sync/commit statistics. It does not contain individual raw latency samples. Do not average per-round percentiles to obtain pooled percentiles; use the program's combined stdout statistics.

Compare full commit latency and throughput. Waiting can shift from sync into direct writes. A direct write alone does not replace an explicit durability boundary. Device cache behavior matters: the local NVMe queue reports `write through`, helping explain a very short sync for initialized direct writes. No power-failure persistence test or isolated kernel-cost attribution is attempted.

See [latest full results](results/2026-10-07/REPORT.md), [verified system calls and integration checks](results/2026-10-07/verification.json), and [historical data](results/historical/README.md). Primary references: [open(2)](https://man7.org/linux/man-pages/man2/open.2.html), [fsync(2)](https://man7.org/linux/man-pages/man2/fsync.2.html), [kernel cache control](https://docs.kernel.org/block/writeback_cache_control.html).
