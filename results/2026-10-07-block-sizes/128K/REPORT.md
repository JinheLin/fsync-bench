# Benchmark run

Started (UTC): 2026-10-07T14:04:14.001854+00:00. Linux 5.14.0-162.6.1.el9_1.x86_64, xfs, INTEL SSDPE2KX040T8.

Release build; block sizes 128K; 2000 measured writes + 100 warmup writes per case per round; 8 rounds; fixed segments 275251200; one sync per write.

Setup, warmup, initialization, final drain, readback and cleanup are excluded. Other processes' I/O was not isolated. Means are weighted by operation counts; throughput uses total bytes / total time. WAL p99 is from pooled samples in stdout, rounded to 0.01 us; per-round p99 values are never averaged.

## Buffered micro-benchmark

`overwrite` is fully written and synced before measurement. `fallocate` fixes EOF but leaves unwritten extents on XFS. `append` grows an initially empty file.

| Workload | Bytes/write | Sync | Mean write us | Mean sync us | Mean write+sync us | MiB/s |
|---|---:|---|---:|---:|---:|---:|
| append | 131072 | fsync | 90.98 | 93.75 | 184.73 | 676.27 |
| append | 131072 | fdatasync | 90.00 | 93.22 | 183.22 | 681.85 |
| fallocate | 131072 | fsync | 91.76 | 95.79 | 187.55 | 666.09 |
| fallocate | 131072 | fdatasync | 86.27 | 94.58 | 180.84 | 690.80 |
| overwrite | 131072 | fsync | 69.53 | 90.19 | 159.72 | 782.23 |
| overwrite | 131072 | fdatasync | 67.26 | 63.46 | 130.72 | 955.64 |

## WAL: buffered vs direct

Both modes use the same aligned record, offset and byte count; every write is immediately followed by the selected sync. Compare full commit latency, since direct I/O can move time from the sync call into the write call.

| Layout | Bytes/write | I/O | Sync | Mean write us | Mean sync us | Mean commit us | Commit p99 us | MiB/s |
|---|---:|---|---|---:|---:|---:|---:|---:|
| append | 131072 | buffered | fsync | 84.68 | 92.24 | 176.92 | 427.11 | 706.20 |
| append | 131072 | buffered | fdatasync | 84.40 | 92.26 | 176.66 | 419.00 | 707.27 |
| append | 131072 | direct | fsync | 66.33 | 31.72 | 98.05 | 161.03 | 1274.09 |
| append | 131072 | direct | fdatasync | 66.41 | 32.04 | 98.44 | 163.11 | 1268.92 |
| fallocate | 131072 | buffered | fsync | 86.19 | 92.65 | 178.84 | 426.81 | 698.64 |
| fallocate | 131072 | buffered | fdatasync | 89.05 | 93.41 | 182.46 | 431.47 | 684.75 |
| fallocate | 131072 | direct | fsync | 55.68 | 23.61 | 79.28 | 141.53 | 1575.43 |
| fallocate | 131072 | direct | fdatasync | 56.02 | 23.88 | 79.90 | 145.13 | 1563.19 |
| initialized | 131072 | buffered | fsync | 83.97 | 102.19 | 186.16 | 2516.67 | 671.20 |
| initialized | 131072 | buffered | fdatasync | 86.92 | 64.28 | 151.20 | 419.66 | 826.32 |
| initialized | 131072 | direct | fsync | 59.29 | 7.83 | 67.12 | 154.16 | 1860.71 |
| initialized | 131072 | direct | fdatasync | 59.49 | 0.45 | 59.94 | 138.28 | 2083.53 |

## Interpretation

The four scenario descriptions in the README explain possible filesystem work. These latency differences do not isolate the cost of an individual inode field, extent-tree operation, journal transaction or block-layer flush.

The target queue reports write_cache='write through'. A short direct-I/O sync must be interpreted alongside write latency and device cache behavior. This benchmark checks syscall success and data readback, not persistence under power failure.

Raw per-round results: [buffered.csv](buffered.csv), [wal.csv](wal.csv). Pooled results: [wal.txt](wal.txt). Environment and exact command templates: [environment.json](environment.json).
