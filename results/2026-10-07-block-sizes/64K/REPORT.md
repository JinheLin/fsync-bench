# Benchmark run

Started (UTC): 2026-10-07T14:03:38.252566+00:00. Linux 5.14.0-162.6.1.el9_1.x86_64, xfs, INTEL SSDPE2KX040T8.

Release build; block sizes 64K; 2000 measured writes + 100 warmup writes per case per round; 8 rounds; fixed segments 137625600; one sync per write.

Setup, warmup, initialization, final drain, readback and cleanup are excluded. Other processes' I/O was not isolated. Means are weighted by operation counts; throughput uses total bytes / total time. WAL p99 is from pooled samples in stdout, rounded to 0.01 us; per-round p99 values are never averaged.

## Buffered micro-benchmark

`overwrite` is fully written and synced before measurement. `fallocate` fixes EOF but leaves unwritten extents on XFS. `append` grows an initially empty file.

| Workload | Bytes/write | Sync | Mean write us | Mean sync us | Mean write+sync us | MiB/s |
|---|---:|---|---:|---:|---:|---:|
| append | 65536 | fsync | 49.53 | 62.77 | 112.29 | 556.10 |
| append | 65536 | fdatasync | 48.38 | 62.73 | 111.11 | 562.05 |
| fallocate | 65536 | fsync | 51.81 | 62.91 | 114.72 | 544.38 |
| fallocate | 65536 | fdatasync | 47.43 | 62.63 | 110.07 | 567.41 |
| overwrite | 65536 | fsync | 39.16 | 60.04 | 99.20 | 629.56 |
| overwrite | 65536 | fdatasync | 34.31 | 39.04 | 73.35 | 851.27 |

## WAL: buffered vs direct

Both modes use the same aligned record, offset and byte count; every write is immediately followed by the selected sync. Compare full commit latency, since direct I/O can move time from the sync call into the write call.

| Layout | Bytes/write | I/O | Sync | Mean write us | Mean sync us | Mean commit us | Commit p99 us | MiB/s |
|---|---:|---|---|---:|---:|---:|---:|---:|
| append | 65536 | buffered | fsync | 47.40 | 62.02 | 109.42 | 341.50 | 570.84 |
| append | 65536 | buffered | fdatasync | 49.13 | 63.19 | 112.32 | 368.05 | 556.09 |
| append | 65536 | direct | fsync | 45.57 | 30.71 | 76.28 | 145.64 | 818.66 |
| append | 65536 | direct | fdatasync | 45.56 | 29.09 | 74.65 | 136.36 | 836.55 |
| fallocate | 65536 | buffered | fsync | 48.10 | 62.41 | 110.51 | 356.79 | 565.19 |
| fallocate | 65536 | buffered | fdatasync | 47.07 | 62.73 | 109.79 | 359.20 | 568.90 |
| fallocate | 65536 | direct | fsync | 34.60 | 21.51 | 56.11 | 120.28 | 1112.52 |
| fallocate | 65536 | direct | fdatasync | 34.61 | 21.33 | 55.94 | 120.31 | 1116.02 |
| initialized | 65536 | buffered | fsync | 44.08 | 54.53 | 98.61 | 381.35 | 633.36 |
| initialized | 65536 | buffered | fdatasync | 44.03 | 38.51 | 82.54 | 277.10 | 756.60 |
| initialized | 65536 | direct | fsync | 35.07 | 4.49 | 39.57 | 105.32 | 1577.40 |
| initialized | 65536 | direct | fdatasync | 34.94 | 0.44 | 35.38 | 97.32 | 1763.56 |

## Interpretation

The four scenario descriptions in the README explain possible filesystem work. These latency differences do not isolate the cost of an individual inode field, extent-tree operation, journal transaction or block-layer flush.

The target queue reports write_cache='write through'. A short direct-I/O sync must be interpreted alongside write latency and device cache behavior. This benchmark checks syscall success and data readback, not persistence under power failure.

Raw per-round results: [buffered.csv](buffered.csv), [wal.csv](wal.csv). Pooled results: [wal.txt](wal.txt). Environment and exact command templates: [environment.json](environment.json).
