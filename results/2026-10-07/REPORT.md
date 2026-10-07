# Benchmark run

Started (UTC): 2026-10-07T10:35:21.634835+00:00. Linux 5.14.0-162.6.1.el9_1.x86_64, xfs, INTEL SSDPE2KX040T8.

Release build; block sizes 16K; 2000 measured writes + 100 warmup writes per case per round; 8 rounds; fixed segments 64M; one sync per write.

Setup, warmup, initialization, final drain, readback and cleanup are excluded. Other processes' I/O was not isolated. Means are weighted by operation counts; throughput uses total bytes / total time. WAL p99 is from pooled samples in stdout, rounded to 0.01 us; per-round p99 values are never averaged.

## Buffered micro-benchmark

`overwrite` is fully written and synced before measurement. `fallocate` fixes EOF but leaves unwritten extents on XFS. `append` grows an initially empty file.

| Workload | Bytes/write | Sync | Mean write us | Mean sync us | Mean write+sync us | MiB/s |
|---|---:|---|---:|---:|---:|---:|
| append | 16384 | fsync | 17.37 | 38.27 | 55.64 | 280.43 |
| append | 16384 | fdatasync | 17.88 | 38.26 | 56.14 | 277.93 |
| fallocate | 16384 | fsync | 17.09 | 38.68 | 55.77 | 279.78 |
| fallocate | 16384 | fdatasync | 16.65 | 38.52 | 55.17 | 282.84 |
| overwrite | 16384 | fsync | 12.39 | 24.15 | 36.54 | 426.88 |
| overwrite | 16384 | fdatasync | 11.41 | 17.33 | 28.74 | 542.37 |

## WAL: buffered vs direct

Both modes use the same aligned record, offset and byte count; every write is immediately followed by the selected sync. Compare full commit latency, since direct I/O can move time from the sync call into the write call.

| Layout | Bytes/write | I/O | Sync | Mean write us | Mean sync us | Mean commit us | Commit p99 us | MiB/s |
|---|---:|---|---|---:|---:|---:|---:|---:|
| append | 16384 | buffered | fsync | 17.35 | 39.78 | 57.13 | 294.77 | 273.09 |
| append | 16384 | buffered | fdatasync | 16.48 | 38.92 | 55.40 | 276.89 | 281.63 |
| append | 16384 | direct | fsync | 28.52 | 28.00 | 56.52 | 115.73 | 276.11 |
| append | 16384 | direct | fdatasync | 28.31 | 29.02 | 57.33 | 123.52 | 272.11 |
| fallocate | 16384 | buffered | fsync | 17.54 | 39.15 | 56.69 | 283.98 | 275.21 |
| fallocate | 16384 | buffered | fdatasync | 16.85 | 38.91 | 55.76 | 288.51 | 279.80 |
| fallocate | 16384 | direct | fsync | 16.88 | 18.32 | 35.20 | 95.70 | 443.00 |
| fallocate | 16384 | direct | fdatasync | 16.83 | 18.17 | 35.00 | 95.27 | 445.53 |
| initialized | 16384 | buffered | fsync | 12.54 | 23.60 | 36.13 | 236.68 | 431.46 |
| initialized | 16384 | buffered | fdatasync | 13.47 | 18.14 | 31.61 | 162.64 | 493.20 |
| initialized | 16384 | direct | fsync | 16.14 | 7.19 | 23.33 | 90.26 | 667.90 |
| initialized | 16384 | direct | fdatasync | 16.10 | 0.41 | 16.50 | 85.73 | 943.30 |

## Interpretation

The four scenario descriptions in the README explain possible filesystem work. These latency differences do not isolate the cost of an individual inode field, extent-tree operation, journal transaction or block-layer flush.

The target queue reports write_cache='write through'. A short direct-I/O sync must be interpreted alongside write latency and device cache behavior. This benchmark checks syscall success and data readback, not persistence under power failure.

Raw per-round results: [buffered.csv](buffered.csv), [wal.csv](wal.csv). Pooled results: [wal.txt](wal.txt). Environment and exact command templates: [environment.json](environment.json).
