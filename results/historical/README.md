# Historical results

Archived measurements from the earlier benchmark versions used in the discussion. Absolute working-directory paths have been replaced by placeholders; numeric CSV results are preserved.

Both runs used this machine's Intel SSDPE2KX040T8 NVMe / XFS / Linux 5.14, Rust 1.92 release, 16 KiB blocks, 2,000 measured writes plus 100 warmup writes per case per round, 64 MiB fixed files, one explicit sync per write. Other processes' I/O was not isolated. Dates below are the original client dates in Asia/Shanghai.

- 2026-10-04: [buffered CSV](2026-10-04-buffered.csv), [stdout](2026-10-04-buffered.txt), [environment and command](2026-10-04-environment.txt). Six rounds, append/fallocate/overwrite × fsync/fdatasync. Mean **sync-call** latencies: append 38.95 / 38.85 us; fallocate 38.47 / 38.72 us; initialized overwrite 25.94 / 17.57 us. These produced the rounded 39 / 39 / 26 / 18 us comparison; they exclude preceding write time.
- 2026-10-04: independent [extent inspection](2026-10-04-extents.txt) and [initialization experiment](2026-10-04-initialization.txt). Initialization timings are single-run examples, not a stable ranking or a guarantee of lowest cost.
- 2026-10-06: [WAL CSV](2026-10-06-wal.csv) and [stdout with pooled percentiles](2026-10-06-wal.txt). Eight rounds; all three layouts and four I/O/sync combinations; alignment 4 KiB. Layout order was initialized/fallocate/append.

The [2026-10-07 rerun](../2026-10-07/REPORT.md) uses the reorganized source in this repository. It runs both binaries for eight rounds with layout order append/fallocate/initialized (buffered: append/fallocate/overwrite). Do not combine historical and rerun measurements into a single distribution.
