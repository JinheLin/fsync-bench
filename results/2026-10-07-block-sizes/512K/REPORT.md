# Benchmark run

Started (UTC): 2026-10-07T14:07:02.467224+00:00. Linux 5.14.0-162.6.1.el9_1.x86_64, xfs, INTEL SSDPE2KX040T8.

Release build; block sizes 512K; 2000 measured writes + 100 warmup writes per case per round; 8 rounds; fixed segments 1101004800; one sync per write.

Setup, warmup, initialization, final drain, readback and cleanup are excluded. Other processes' I/O was not isolated. Means are weighted by operation counts; throughput uses total bytes / total time. WAL p99 is from pooled samples in stdout, rounded to 0.01 us; per-round p99 values are never averaged.

## Buffered micro-benchmark

`overwrite` is fully written and synced before measurement. `fallocate` fixes EOF but leaves unwritten extents on XFS. `append` grows an initially empty file.

| Workload | Bytes/write | Sync | Mean write us | Mean sync us | Mean write+sync us | MiB/s |
|---|---:|---|---:|---:|---:|---:|
| append | 524288 | fsync | 335.26 | 358.06 | 693.31 | 721.01 |
| append | 524288 | fdatasync | 329.99 | 370.86 | 700.84 | 713.27 |
| fallocate | 524288 | fsync | 304.94 | 264.39 | 569.33 | 877.98 |
| fallocate | 524288 | fdatasync | 312.55 | 266.24 | 578.79 | 863.63 |
| overwrite | 524288 | fsync | 270.91 | 325.39 | 596.30 | 838.31 |
| overwrite | 524288 | fdatasync | 247.42 | 224.48 | 471.90 | 1059.26 |

## WAL: buffered vs direct

Both modes use the same aligned record, offset and byte count; every write is immediately followed by the selected sync. Compare full commit latency, since direct I/O can move time from the sync call into the write call.

| Layout | Bytes/write | I/O | Sync | Mean write us | Mean sync us | Mean commit us | Commit p99 us | MiB/s |
|---|---:|---|---|---:|---:|---:|---:|---:|
| append | 524288 | buffered | fsync | 316.63 | 315.86 | 632.48 | 2851.05 | 790.36 |
| append | 524288 | buffered | fdatasync | 318.93 | 295.69 | 614.62 | 2733.76 | 813.32 |
| append | 524288 | direct | fsync | 207.10 | 40.18 | 247.28 | 483.71 | 2021.31 |
| append | 524288 | direct | fdatasync | 206.43 | 41.68 | 248.10 | 464.33 | 2014.60 |
| fallocate | 524288 | buffered | fsync | 326.10 | 378.46 | 704.56 | 3175.73 | 709.52 |
| fallocate | 524288 | buffered | fdatasync | 331.04 | 363.25 | 694.29 | 3018.78 | 720.01 |
| fallocate | 524288 | direct | fsync | 203.98 | 31.77 | 235.75 | 568.04 | 2120.18 |
| fallocate | 524288 | direct | fdatasync | 201.74 | 30.86 | 232.61 | 522.05 | 2148.76 |
| initialized | 524288 | buffered | fsync | 321.89 | 335.89 | 657.77 | 3038.62 | 759.97 |
| initialized | 524288 | buffered | fdatasync | 304.83 | 224.05 | 528.88 | 870.66 | 945.14 |
| initialized | 524288 | direct | fsync | 214.12 | 51.71 | 265.83 | 2547.27 | 1880.31 |
| initialized | 524288 | direct | fdatasync | 221.34 | 0.52 | 221.86 | 531.54 | 2252.91 |

## Interpretation

The four scenario descriptions in the README explain possible filesystem work. These latency differences do not isolate the cost of an individual inode field, extent-tree operation, journal transaction or block-layer flush.

The target queue reports write_cache='write through'. A short direct-I/O sync must be interpreted alongside write latency and device cache behavior. This benchmark checks syscall success and data readback, not persistence under power failure.

Raw per-round results: [buffered.csv](buffered.csv), [wal.csv](wal.csv). Pooled results: [wal.txt](wal.txt). Environment and exact command templates: [environment.json](environment.json).
