# Benchmark run

Started (UTC): 2026-10-07T14:03:15.473820+00:00. Linux 5.14.0-162.6.1.el9_1.x86_64, xfs, INTEL SSDPE2KX040T8.

Release build; block sizes 32K; 2000 measured writes + 100 warmup writes per case per round; 8 rounds; fixed segments 68812800; one sync per write.

Setup, warmup, initialization, final drain, readback and cleanup are excluded. Other processes' I/O was not isolated. Means are weighted by operation counts; throughput uses total bytes / total time. WAL p99 is from pooled samples in stdout, rounded to 0.01 us; per-round p99 values are never averaged.

## Buffered micro-benchmark

`overwrite` is fully written and synced before measurement. `fallocate` fixes EOF but leaves unwritten extents on XFS. `append` grows an initially empty file.

| Workload | Bytes/write | Sync | Mean write us | Mean sync us | Mean write+sync us | MiB/s |
|---|---:|---|---:|---:|---:|---:|
| append | 32768 | fsync | 28.03 | 47.24 | 75.27 | 414.78 |
| append | 32768 | fdatasync | 28.56 | 47.50 | 76.06 | 410.47 |
| fallocate | 32768 | fsync | 26.86 | 47.03 | 73.89 | 422.53 |
| fallocate | 32768 | fdatasync | 25.84 | 46.67 | 72.51 | 430.59 |
| overwrite | 32768 | fsync | 20.04 | 37.10 | 57.14 | 546.31 |
| overwrite | 32768 | fdatasync | 19.14 | 25.23 | 44.36 | 703.46 |

## WAL: buffered vs direct

Both modes use the same aligned record, offset and byte count; every write is immediately followed by the selected sync. Compare full commit latency, since direct I/O can move time from the sync call into the write call.

| Layout | Bytes/write | I/O | Sync | Mean write us | Mean sync us | Mean commit us | Commit p99 us | MiB/s |
|---|---:|---|---|---:|---:|---:|---:|---:|
| append | 32768 | buffered | fsync | 28.13 | 47.73 | 75.86 | 326.64 | 411.59 |
| append | 32768 | buffered | fdatasync | 27.80 | 47.36 | 75.17 | 312.17 | 415.39 |
| append | 32768 | direct | fsync | 33.40 | 26.24 | 59.64 | 120.05 | 523.44 |
| append | 32768 | direct | fdatasync | 33.66 | 27.08 | 60.74 | 125.21 | 513.92 |
| fallocate | 32768 | buffered | fsync | 28.14 | 47.17 | 75.31 | 334.51 | 414.59 |
| fallocate | 32768 | buffered | fdatasync | 25.63 | 47.05 | 72.69 | 309.35 | 429.53 |
| fallocate | 32768 | direct | fsync | 22.97 | 19.13 | 42.10 | 106.25 | 741.15 |
| fallocate | 32768 | direct | fdatasync | 23.06 | 19.27 | 42.33 | 107.11 | 737.18 |
| initialized | 32768 | buffered | fsync | 23.70 | 44.09 | 67.78 | 361.39 | 460.57 |
| initialized | 32768 | buffered | fdatasync | 22.26 | 25.99 | 48.26 | 247.61 | 646.79 |
| initialized | 32768 | direct | fsync | 23.44 | 4.19 | 27.63 | 100.77 | 1128.94 |
| initialized | 32768 | direct | fdatasync | 23.51 | 0.41 | 23.92 | 95.79 | 1303.46 |

## Interpretation

The four scenario descriptions in the README explain possible filesystem work. These latency differences do not isolate the cost of an individual inode field, extent-tree operation, journal transaction or block-layer flush.

The target queue reports write_cache='write through'. A short direct-I/O sync must be interpreted alongside write latency and device cache behavior. This benchmark checks syscall success and data readback, not persistence under power failure.

Raw per-round results: [buffered.csv](buffered.csv), [wal.csv](wal.csv). Pooled results: [wal.txt](wal.txt). Environment and exact command templates: [environment.json](environment.json).
