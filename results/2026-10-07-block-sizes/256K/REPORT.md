# Benchmark run

Started (UTC): 2026-10-07T14:05:14.171857+00:00. Linux 5.14.0-162.6.1.el9_1.x86_64, xfs, INTEL SSDPE2KX040T8.

Release build; block sizes 256K; 2000 measured writes + 100 warmup writes per case per round; 8 rounds; fixed segments 550502400; one sync per write.

Setup, warmup, initialization, final drain, readback and cleanup are excluded. Other processes' I/O was not isolated. Means are weighted by operation counts; throughput uses total bytes / total time. WAL p99 is from pooled samples in stdout, rounded to 0.01 us; per-round p99 values are never averaged.

## Buffered micro-benchmark

`overwrite` is fully written and synced before measurement. `fallocate` fixes EOF but leaves unwritten extents on XFS. `append` grows an initially empty file.

| Workload | Bytes/write | Sync | Mean write us | Mean sync us | Mean write+sync us | MiB/s |
|---|---:|---|---:|---:|---:|---:|
| append | 262144 | fsync | 168.40 | 150.74 | 319.14 | 783.06 |
| append | 262144 | fdatasync | 167.40 | 152.83 | 320.22 | 780.41 |
| fallocate | 262144 | fsync | 159.27 | 149.46 | 308.73 | 809.46 |
| fallocate | 262144 | fdatasync | 162.80 | 150.69 | 313.49 | 797.18 |
| overwrite | 262144 | fsync | 140.74 | 171.75 | 312.50 | 799.72 |
| overwrite | 262144 | fdatasync | 132.44 | 116.01 | 248.46 | 1005.82 |

## WAL: buffered vs direct

Both modes use the same aligned record, offset and byte count; every write is immediately followed by the selected sync. Compare full commit latency, since direct I/O can move time from the sync call into the write call.

| Layout | Bytes/write | I/O | Sync | Mean write us | Mean sync us | Mean commit us | Commit p99 us | MiB/s |
|---|---:|---|---|---:|---:|---:|---:|---:|
| append | 262144 | buffered | fsync | 164.18 | 150.60 | 314.78 | 573.31 | 793.94 |
| append | 262144 | buffered | fdatasync | 161.69 | 149.88 | 311.58 | 563.91 | 802.11 |
| append | 262144 | direct | fsync | 112.22 | 36.78 | 149.00 | 269.73 | 1676.98 |
| append | 262144 | direct | fdatasync | 116.95 | 38.11 | 155.06 | 286.99 | 1611.42 |
| fallocate | 262144 | buffered | fsync | 165.57 | 151.57 | 317.14 | 609.14 | 788.02 |
| fallocate | 262144 | buffered | fdatasync | 162.21 | 152.97 | 315.19 | 574.55 | 792.92 |
| fallocate | 262144 | direct | fsync | 103.53 | 28.51 | 132.04 | 269.26 | 1892.36 |
| fallocate | 262144 | direct | fdatasync | 101.40 | 27.11 | 128.51 | 256.02 | 1944.25 |
| initialized | 262144 | buffered | fsync | 167.81 | 179.02 | 346.84 | 2763.40 | 720.58 |
| initialized | 262144 | buffered | fdatasync | 159.32 | 116.69 | 276.02 | 522.92 | 905.40 |
| initialized | 262144 | direct | fsync | 114.19 | 29.29 | 143.48 | 1323.03 | 1741.51 |
| initialized | 262144 | direct | fdatasync | 115.71 | 0.48 | 116.19 | 302.02 | 2150.56 |

## Interpretation

The four scenario descriptions in the README explain possible filesystem work. These latency differences do not isolate the cost of an individual inode field, extent-tree operation, journal transaction or block-layer flush.

The target queue reports write_cache='write through'. A short direct-I/O sync must be interpreted alongside write latency and device cache behavior. This benchmark checks syscall success and data readback, not persistence under power failure.

Raw per-round results: [buffered.csv](buffered.csv), [wal.csv](wal.csv). Pooled results: [wal.txt](wal.txt). Environment and exact command templates: [environment.json](environment.json).
