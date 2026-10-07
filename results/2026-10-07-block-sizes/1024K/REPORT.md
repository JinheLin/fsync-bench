# Benchmark run

Started (UTC): 2026-10-07T14:10:32.895476+00:00. Linux 5.14.0-162.6.1.el9_1.x86_64, xfs, INTEL SSDPE2KX040T8.

Release build; block sizes 1024K; 2000 measured writes + 100 warmup writes per case per round; 8 rounds; fixed segments 2202009600; one sync per write.

Setup, warmup, initialization, final drain, readback and cleanup are excluded. Other processes' I/O was not isolated. Means are weighted by operation counts; throughput uses total bytes / total time. WAL p99 is from pooled samples in stdout, rounded to 0.01 us; per-round p99 values are never averaged.

## Buffered micro-benchmark

`overwrite` is fully written and synced before measurement. `fallocate` fixes EOF but leaves unwritten extents on XFS. `append` grows an initially empty file.

| Workload | Bytes/write | Sync | Mean write us | Mean sync us | Mean write+sync us | MiB/s |
|---|---:|---|---:|---:|---:|---:|
| append | 1048576 | fsync | 614.81 | 578.45 | 1193.26 | 837.90 |
| append | 1048576 | fdatasync | 619.91 | 567.21 | 1187.12 | 842.23 |
| fallocate | 1048576 | fsync | 615.63 | 582.16 | 1197.80 | 834.72 |
| fallocate | 1048576 | fdatasync | 607.47 | 598.31 | 1205.78 | 829.20 |
| overwrite | 1048576 | fsync | 485.39 | 556.55 | 1041.95 | 959.58 |
| overwrite | 1048576 | fdatasync | 485.95 | 439.42 | 925.37 | 1080.45 |

## WAL: buffered vs direct

Both modes use the same aligned record, offset and byte count; every write is immediately followed by the selected sync. Compare full commit latency, since direct I/O can move time from the sync call into the write call.

| Layout | Bytes/write | I/O | Sync | Mean write us | Mean sync us | Mean commit us | Commit p99 us | MiB/s |
|---|---:|---|---|---:|---:|---:|---:|---:|
| append | 1048576 | buffered | fsync | 606.41 | 586.34 | 1192.75 | 3131.70 | 838.29 |
| append | 1048576 | buffered | fdatasync | 608.42 | 574.47 | 1182.89 | 3162.61 | 845.26 |
| append | 1048576 | direct | fsync | 418.78 | 41.33 | 460.11 | 1010.21 | 2172.87 |
| append | 1048576 | direct | fdatasync | 412.37 | 41.60 | 453.97 | 966.82 | 2202.23 |
| fallocate | 1048576 | buffered | fsync | 620.31 | 589.70 | 1210.01 | 3185.37 | 826.33 |
| fallocate | 1048576 | buffered | fdatasync | 596.00 | 487.33 | 1083.33 | 1777.55 | 922.95 |
| fallocate | 1048576 | direct | fsync | 420.32 | 32.45 | 452.77 | 1032.08 | 2208.14 |
| fallocate | 1048576 | direct | fdatasync | 417.05 | 33.30 | 450.35 | 1025.91 | 2219.97 |
| initialized | 1048576 | buffered | fsync | 601.08 | 552.32 | 1153.41 | 3077.03 | 866.89 |
| initialized | 1048576 | buffered | fdatasync | 591.01 | 436.67 | 1027.68 | 1546.64 | 972.93 |
| initialized | 1048576 | direct | fsync | 418.12 | 104.90 | 523.02 | 2950.89 | 1911.60 |
| initialized | 1048576 | direct | fdatasync | 429.74 | 0.60 | 430.34 | 992.51 | 2323.24 |

## Interpretation

The four scenario descriptions in the README explain possible filesystem work. These latency differences do not isolate the cost of an individual inode field, extent-tree operation, journal transaction or block-layer flush.

The target queue reports write_cache='write through'. A short direct-I/O sync must be interpreted alongside write latency and device cache behavior. This benchmark checks syscall success and data readback, not persistence under power failure.

Raw per-round results: [buffered.csv](buffered.csv), [wal.csv](wal.csv). Pooled results: [wal.txt](wal.txt). Environment and exact command templates: [environment.json](environment.json).
