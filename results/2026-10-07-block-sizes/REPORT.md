# Write-size sweep: complete write + sync latency

Linux 5.14.0-162.6.1.el9_1.x86_64; xfs; INTEL SSDPE2KX040T8; rustc 1.92.0 (ded5c06cf 2025-12-08).

Sizes (KiB): 32, 64, 128, 256, 512, 1024. 2000 measured writes + 100 warmup writes per case per round; 8 rounds. ONE explicit sync after EVERY write; release binaries; sequential, single writer.

Fixed file capacity for each size is (iterations + warmup) × block bytes. The capacity is identical across methods at that size; all measured writes stay inside initialized/fallocated files without growing EOF or wrapping. Capacity varies between sizes, so this experiment represents correspondingly sized segments.

Means use all measured commits; throughput uses total bytes/time. WAL p99 comes from pooled complete-commit samples, rounded to 0.01 us. No per-round percentile is averaged, and no write percentile is added to a sync percentile. Setup, warmup, final drain, readback and cleanup are excluded. Other processes' I/O was not isolated. Device queue write_cache='write through', as recorded in each environment.json.

## WAL: buffered vs direct

### append

Mean write + sync (us):

| Write KiB | buffered fsync | buffered fdatasync | direct fsync | direct fdatasync |
|---:|---:|---:|---:|---:|
| 32 | 75.86 | 75.17 | 59.64 | 60.74 |
| 64 | 109.42 | 112.32 | 76.28 | 74.65 |
| 128 | 176.92 | 176.66 | 98.05 | 98.44 |
| 256 | 314.78 | 311.58 | 149.00 | 155.06 |
| 512 | 632.48 | 614.62 | 247.28 | 248.10 |
| 1024 | 1192.75 | 1182.89 | 460.11 | 453.97 |

Complete commit p99 (us):

| Write KiB | buffered fsync | buffered fdatasync | direct fsync | direct fdatasync |
|---:|---:|---:|---:|---:|
| 32 | 326.64 | 312.17 | 120.05 | 125.21 |
| 64 | 341.50 | 368.05 | 145.64 | 136.36 |
| 128 | 427.11 | 419.00 | 161.03 | 163.11 |
| 256 | 573.31 | 563.91 | 269.73 | 286.99 |
| 512 | 2851.05 | 2733.76 | 483.71 | 464.33 |
| 1024 | 3131.70 | 3162.61 | 1010.21 | 966.82 |

Throughput (MiB/s):

| Write KiB | buffered fsync | buffered fdatasync | direct fsync | direct fdatasync |
|---:|---:|---:|---:|---:|
| 32 | 411.59 | 415.39 | 523.44 | 513.92 |
| 64 | 570.84 | 556.09 | 818.66 | 836.55 |
| 128 | 706.20 | 707.27 | 1274.09 | 1268.92 |
| 256 | 793.94 | 802.11 | 1676.98 | 1611.42 |
| 512 | 790.36 | 813.32 | 2021.31 | 2014.60 |
| 1024 | 838.29 | 845.26 | 2172.87 | 2202.23 |

### fallocate

Mean write + sync (us):

| Write KiB | buffered fsync | buffered fdatasync | direct fsync | direct fdatasync |
|---:|---:|---:|---:|---:|
| 32 | 75.31 | 72.69 | 42.10 | 42.33 |
| 64 | 110.51 | 109.79 | 56.11 | 55.94 |
| 128 | 178.84 | 182.46 | 79.28 | 79.90 |
| 256 | 317.14 | 315.19 | 132.04 | 128.51 |
| 512 | 704.56 | 694.29 | 235.75 | 232.61 |
| 1024 | 1210.01 | 1083.33 | 452.77 | 450.35 |

Complete commit p99 (us):

| Write KiB | buffered fsync | buffered fdatasync | direct fsync | direct fdatasync |
|---:|---:|---:|---:|---:|
| 32 | 334.51 | 309.35 | 106.25 | 107.11 |
| 64 | 356.79 | 359.20 | 120.28 | 120.31 |
| 128 | 426.81 | 431.47 | 141.53 | 145.13 |
| 256 | 609.14 | 574.55 | 269.26 | 256.02 |
| 512 | 3175.73 | 3018.78 | 568.04 | 522.05 |
| 1024 | 3185.37 | 1777.55 | 1032.08 | 1025.91 |

Throughput (MiB/s):

| Write KiB | buffered fsync | buffered fdatasync | direct fsync | direct fdatasync |
|---:|---:|---:|---:|---:|
| 32 | 414.59 | 429.53 | 741.15 | 737.18 |
| 64 | 565.19 | 568.90 | 1112.52 | 1116.02 |
| 128 | 698.64 | 684.75 | 1575.43 | 1563.19 |
| 256 | 788.02 | 792.92 | 1892.36 | 1944.25 |
| 512 | 709.52 | 720.01 | 2120.18 | 2148.76 |
| 1024 | 826.33 | 922.95 | 2208.14 | 2219.97 |

### initialized

Mean write + sync (us):

| Write KiB | buffered fsync | buffered fdatasync | direct fsync | direct fdatasync |
|---:|---:|---:|---:|---:|
| 32 | 67.78 | 48.26 | 27.63 | 23.92 |
| 64 | 98.61 | 82.54 | 39.57 | 35.38 |
| 128 | 186.16 | 151.20 | 67.12 | 59.94 |
| 256 | 346.84 | 276.02 | 143.48 | 116.19 |
| 512 | 657.77 | 528.88 | 265.83 | 221.86 |
| 1024 | 1153.41 | 1027.68 | 523.02 | 430.34 |

Complete commit p99 (us):

| Write KiB | buffered fsync | buffered fdatasync | direct fsync | direct fdatasync |
|---:|---:|---:|---:|---:|
| 32 | 361.39 | 247.61 | 100.77 | 95.79 |
| 64 | 381.35 | 277.10 | 105.32 | 97.32 |
| 128 | 2516.67 | 419.66 | 154.16 | 138.28 |
| 256 | 2763.40 | 522.92 | 1323.03 | 302.02 |
| 512 | 3038.62 | 870.66 | 2547.27 | 531.54 |
| 1024 | 3077.03 | 1546.64 | 2950.89 | 992.51 |

Throughput (MiB/s):

| Write KiB | buffered fsync | buffered fdatasync | direct fsync | direct fdatasync |
|---:|---:|---:|---:|---:|
| 32 | 460.57 | 646.79 | 1128.94 | 1303.46 |
| 64 | 633.36 | 756.60 | 1577.40 | 1763.56 |
| 128 | 671.20 | 826.32 | 1860.71 | 2083.53 |
| 256 | 720.58 | 905.40 | 1741.51 | 2150.56 |
| 512 | 759.97 | 945.14 | 1880.31 | 2252.91 |
| 1024 | 866.89 | 972.93 | 1911.60 | 2323.24 |

## Buffered micro-benchmark: the four original situations

Mean write + sync (us), using fsync for append/fallocate. Both sync methods for every state are available in aggregate.csv; initialized means overwrite here.

| Write KiB | append + fsync | fallocate first write + fsync | initialized + fsync | initialized + fdatasync |
|---:|---:|---:|---:|---:|
| 32 | 75.27 | 73.89 | 57.14 | 44.36 |
| 64 | 112.29 | 114.72 | 99.20 | 73.35 |
| 128 | 184.73 | 187.55 | 159.72 | 130.72 |
| 256 | 319.14 | 308.73 | 312.50 | 248.46 |
| 512 | 693.31 | 569.33 | 596.30 | 471.90 |
| 1024 | 1193.26 | 1197.80 | 1041.95 | 925.37 |

## Raw results and reproduction

[aggregate.csv](aggregate.csv) contains means, rates and WAL pooled quantiles for all combinations. [manifest.json](manifest.json) records the sweep parameters and benchmark source commit. Each size's subdirectory contains the per-round CSV, stdout, environment, exact command templates and full report.

- [32 KiB](32K/REPORT.md)
- [64 KiB](64K/REPORT.md)
- [128 KiB](128K/REPORT.md)
- [256 KiB](256K/REPORT.md)
- [512 KiB](512K/REPORT.md)
- [1024 KiB](1024K/REPORT.md)
