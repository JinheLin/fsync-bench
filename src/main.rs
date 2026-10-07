#[cfg(not(target_os = "linux"))]
compile_error!("This benchmark targets Linux fsync/fdatasync semantics.");

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use fsync_bench::{
    parse_size, parse_unique, payload, preallocate, Result, ScratchDir, Stats, SyncMethod,
};

const HELP: &str = "\
fsync-bench: single-threaded, buffered Linux file I/O (pwrite + sync)

Usage: fsync-bench [OPTIONS]
  --dir PATH                Target filesystem directory [./bench-data]
  --block-sizes LIST        Bytes per write, e.g. 4K,16K,64K [4K,16K,64K]
  --iterations N            Measured batches per case per round [200]
  --warmup N                Unmeasured batches before each case [20]
  --rounds N                Repetitions, rotating method order [3]
  --sync-every N            Writes per batch, then ONE sync [1]
  --file-size SIZE          Overwrite/fallocate file size [64M]
  --workloads LIST          overwrite,append,fallocate [overwrite,append]
  --methods LIST            fsync,fdatasync,none [fsync,fdatasync]
  --csv PATH                Save per-round numeric results; must be a new file
  -h, --help                Show this help

Sizes accept bytes or binary K/KiB, M/MiB, G/GiB suffixes.
none measures writes WITHOUT a durability boundary; its final drain is excluded.
File preparation, warmup, final drain and deletion are excluded from throughput.
Each case uses its own temporary file, removed on normal exit.
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    Fsync,
    Fdatasync,
    None,
}

impl Method {
    fn name(self) -> &'static str {
        match self {
            Self::Fsync => "fsync",
            Self::Fdatasync => "fdatasync",
            Self::None => "none",
        }
    }

    fn sync(self, file: &File) -> io::Result<()> {
        match self {
            Self::Fsync => SyncMethod::Fsync.sync(file),
            Self::Fdatasync => SyncMethod::Fdatasync.sync(file),
            Self::None => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Workload {
    Overwrite,
    Append,
    Fallocate,
}

impl Workload {
    fn name(self) -> &'static str {
        match self {
            Self::Overwrite => "overwrite",
            Self::Append => "append",
            Self::Fallocate => "fallocate",
        }
    }
}

#[derive(Debug)]
struct Config {
    dir: PathBuf,
    block_sizes: Vec<usize>,
    iterations: usize,
    warmup: usize,
    rounds: usize,
    sync_every: usize,
    file_size: u64,
    workloads: Vec<Workload>,
    methods: Vec<Method>,
    csv: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            dir: PathBuf::from("./bench-data"),
            block_sizes: vec![4096, 16384, 65536],
            iterations: 200,
            warmup: 20,
            rounds: 3,
            sync_every: 1,
            file_size: 64 * 1024 * 1024,
            workloads: vec![Workload::Overwrite, Workload::Append],
            methods: vec![Method::Fsync, Method::Fdatasync],
            csv: None,
        }
    }
}

impl Config {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();
        let mut args = args;
        while let Some(arg) = args.next() {
            let (key, inline) = arg
                .split_once('=')
                .map_or((arg.as_str(), None), |(k, v)| (k, Some(v)));
            let value = match inline {
                Some(value) => value.to_owned(),
                None => args
                    .next()
                    .ok_or_else(|| format!("missing value for {key}"))?,
            };
            match key {
                "--dir" => config.dir = PathBuf::from(value),
                "--block-sizes" => {
                    config.block_sizes =
                        parse_unique(&value, |s| Ok(usize::try_from(parse_size(s)?)?))?
                }
                "--iterations" => config.iterations = value.parse()?,
                "--warmup" => config.warmup = value.parse()?,
                "--rounds" => config.rounds = value.parse()?,
                "--sync-every" => config.sync_every = value.parse()?,
                "--file-size" => config.file_size = parse_size(&value)?,
                "--workloads" => {
                    config.workloads = parse_unique(&value, |s| match s {
                        "overwrite" => Ok(Workload::Overwrite),
                        "append" => Ok(Workload::Append),
                        "fallocate" => Ok(Workload::Fallocate),
                        _ => Err(format!("unknown workload: {s}").into()),
                    })?
                }
                "--methods" => {
                    config.methods = parse_unique(&value, |s| match s {
                        "fsync" => Ok(Method::Fsync),
                        "fdatasync" => Ok(Method::Fdatasync),
                        "none" => Ok(Method::None),
                        _ => Err(format!("unknown method: {s}").into()),
                    })?
                }
                "--csv" => config.csv = Some(PathBuf::from(value)),
                _ => return Err(format!("unknown option: {key}").into()),
            }
        }
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.iterations == 0 || self.rounds == 0 || self.sync_every == 0 {
            return Err("iterations, rounds and sync-every must be positive".into());
        }
        if self.file_size == 0 || self.file_size > i64::MAX as u64 {
            return Err("file-size must be positive and fit a Linux file offset".into());
        }
        let batches = self
            .iterations
            .checked_add(self.warmup)
            .ok_or("batch count overflow")?;
        let writes = batches
            .checked_mul(self.sync_every)
            .ok_or("write count overflow")?;
        for &block in &self.block_sizes {
            if block < 8 {
                return Err("block sizes must be at least 8 bytes (per-write sequence tag)".into());
            }
            if self.workloads.contains(&Workload::Overwrite) && self.file_size < block as u64 {
                return Err("file-size must be at least the largest overwrite block".into());
            }
            let bytes = (writes as u64)
                .checked_mul(block as u64)
                .ok_or("total write bytes overflow")?;
            if bytes > i64::MAX as u64 {
                return Err("total write bytes exceed a Linux file offset".into());
            }
            if self.workloads.contains(&Workload::Fallocate) && bytes > self.file_size {
                return Err(format!(
                    "fallocate needs file-size >= {bytes} bytes for block {block}, including warmup; writes must stay within the preallocated file"
                ).into());
            }
        }
        Ok(())
    }
}

fn prepare(file: &File, workload: Workload, file_size: u64) -> io::Result<()> {
    if workload == Workload::Fallocate {
        // Allocate without writing data: first writes can convert unwritten extents.
        preallocate(file, file_size)?;
    }
    if workload == Workload::Overwrite {
        // set_len alone creates holes; fallocate alone can leave unwritten extents.
        // Write every byte and sync before measuring overwrites of initialized extents.
        let data = payload(1024 * 1024);
        let mut offset = 0;
        while offset < file_size {
            let len = (file_size - offset).min(data.len() as u64) as usize;
            file.write_all_at(&data[..len], offset)?;
            offset += len as u64;
        }
    }
    Method::Fsync.sync(file)
}

fn write_batch(
    file: &File,
    data: &mut [u8],
    workload: Workload,
    file_size: u64,
    writes: usize,
    position: &mut u64,
    sequence: &mut u64,
) -> io::Result<()> {
    for _ in 0..writes {
        if workload == Workload::Overwrite && *position > file_size - data.len() as u64 {
            *position = 0;
        }
        data[..8].copy_from_slice(&sequence.to_le_bytes());
        file.write_all_at(data, *position)?;
        *position += data.len() as u64;
        *sequence += 1;
    }
    Ok(())
}

#[derive(Debug)]
struct Measurement {
    workload: Workload,
    method: Method,
    block: usize,
    round: usize,
    bytes: u64,
    elapsed_s: f64,
    write_us: Vec<f64>,
    sync_us: Vec<f64>,
    batch_us: Vec<f64>,
    drain_ms: f64,
}

fn run_case(
    config: &Config,
    dir: &Path,
    workload: Workload,
    method: Method,
    block: usize,
    round: usize,
) -> Result<Measurement> {
    let path = dir.join("case.dat");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)?;
    prepare(&file, workload, config.file_size)?;
    // Namespace durability is not part of the measured operation.
    Method::Fsync.sync(&File::open(dir)?)?;
    let mut data = payload(block);
    let mut position = 0;
    let mut sequence = 1;
    for _ in 0..config.warmup {
        write_batch(
            &file,
            &mut data,
            workload,
            config.file_size,
            config.sync_every,
            &mut position,
            &mut sequence,
        )?;
        method.sync(&file)?;
    }
    // Also drain the none baseline's warmup before taking samples.
    Method::Fsync.sync(&file)?;

    let mut sample = Measurement {
        workload,
        method,
        block,
        round,
        bytes: config.iterations as u64 * config.sync_every as u64 * block as u64,
        elapsed_s: 0.0,
        write_us: Vec::with_capacity(config.iterations),
        sync_us: Vec::with_capacity(if method == Method::None {
            0
        } else {
            config.iterations
        }),
        batch_us: Vec::with_capacity(config.iterations),
        drain_ms: 0.0,
    };
    let start = Instant::now();
    for _ in 0..config.iterations {
        let batch_start = Instant::now();
        write_batch(
            &file,
            &mut data,
            workload,
            config.file_size,
            config.sync_every,
            &mut position,
            &mut sequence,
        )?;
        let write_end = Instant::now();
        let batch_end = if method == Method::None {
            write_end
        } else {
            method.sync(&file)?;
            Instant::now()
        };
        sample
            .write_us
            .push((write_end - batch_start).as_secs_f64() * 1e6);
        if method != Method::None {
            sample
                .sync_us
                .push((batch_end - write_end).as_secs_f64() * 1e6);
        }
        sample
            .batch_us
            .push((batch_end - batch_start).as_secs_f64() * 1e6);
    }
    sample.elapsed_s = start.elapsed().as_secs_f64();

    let drain_start = Instant::now();
    Method::Fsync.sync(&file)?;
    sample.drain_ms = drain_start.elapsed().as_secs_f64() * 1e3;
    let expected_size = match workload {
        Workload::Overwrite | Workload::Fallocate => config.file_size,
        Workload::Append => {
            (config.iterations + config.warmup) as u64 * config.sync_every as u64 * block as u64
        }
    };
    if file.metadata()?.len() != expected_size {
        return Err("unexpected file size after benchmark".into());
    }
    drop(file);
    fs::remove_file(path)?;
    // Finish directory deletion work before preparing the next case.
    Method::Fsync.sync(&File::open(dir)?)?;
    Ok(sample)
}

impl Measurement {
    fn mib_s(&self) -> f64 {
        self.bytes as f64 / (1024.0 * 1024.0) / self.elapsed_s
    }

    fn batches_s(&self) -> f64 {
        self.batch_us.len() as f64 / self.elapsed_s
    }

    fn print(&self) {
        let write = Stats::new(&self.write_us);
        let sync = Stats::new(&self.sync_us);
        let batch = Stats::new(&self.batch_us);
        let sync_values = if self.method == Method::None {
            format!("{:>10} {:>10} {:>10} {:>10}", "-", "-", "-", "-")
        } else {
            format!(
                "{:>10.2} {:>10.2} {:>10.2} {:>10.2}",
                sync.mean, sync.p50, sync.p95, sync.p99
            )
        };
        println!(
            "{:>5} {:<9} {:>7} {:<9} {:>10.2} {:>11.1} {:>12.2} {} {:>12.2} {:>9.2}",
            self.round,
            self.workload.name(),
            self.block,
            self.method.name(),
            self.mib_s(),
            self.batches_s(),
            write.mean,
            sync_values,
            batch.p99,
            self.drain_ms
        );
    }

    fn write_csv(&self, out: &mut impl Write, config: &Config) -> io::Result<()> {
        let write = Stats::new(&self.write_us);
        let sync = Stats::new(&self.sync_us);
        let batch = Stats::new(&self.batch_us);
        let sync_count = self.sync_us.len();
        let sync_values = if sync_count == 0 {
            ",,,".to_owned()
        } else {
            format!(
                "{:.6},{:.6},{:.6},{:.6}",
                sync.mean, sync.p50, sync.p95, sync.p99
            )
        };
        writeln!(
            out,
            "{},{},{},{},{},{},{},{},{},{},{:.9},{:.6},{:.6},{:.6},{},{:.6},{:.6}",
            self.round,
            self.workload.name(),
            self.block,
            self.method.name(),
            config.sync_every,
            config.iterations,
            config.warmup,
            self.batch_us.len() * config.sync_every,
            sync_count,
            self.bytes,
            self.elapsed_s,
            self.mib_s(),
            self.batches_s(),
            write.mean,
            sync_values,
            batch.p99,
            self.drain_ms
        )
    }
}

fn print_comparison(config: &Config, samples: &[Measurement]) {
    println!("\nAll rounds combined (total measured bytes / total measured time):");
    println!("workload    block    fsync MiB/s  fdatasync MiB/s  fdatasync/fsync  sync p99 us (fsync / fdatasync)");
    for &workload in &config.workloads {
        for &block in &config.block_sizes {
            let aggregate = |method| {
                let cases: Vec<_> = samples
                    .iter()
                    .filter(|s| s.workload == workload && s.block == block && s.method == method)
                    .collect();
                if cases.is_empty() {
                    return None;
                }
                let bytes: f64 = cases.iter().map(|s| s.bytes as f64).sum();
                let seconds: f64 = cases.iter().map(|s| s.elapsed_s).sum();
                let latencies: Vec<_> = cases
                    .iter()
                    .flat_map(|s| s.sync_us.iter().copied())
                    .collect();
                Some((
                    bytes / (1024.0 * 1024.0) / seconds,
                    Stats::new(&latencies).p99,
                ))
            };
            if let (Some((fsync, fsync_p99)), Some((fdatasync, fdatasync_p99))) =
                (aggregate(Method::Fsync), aggregate(Method::Fdatasync))
            {
                println!(
                    "{:<9} {:>7} {:>14.2} {:>16.2} {:>15.3}x {:>14.2} / {:.2}",
                    workload.name(),
                    block,
                    fsync,
                    fdatasync,
                    fdatasync / fsync,
                    fsync_p99,
                    fdatasync_p99
                );
            }
        }
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{HELP}");
        return Ok(());
    }
    let config = Config::parse(args.into_iter())?;
    // Reserve the output before doing I/O; never silently replace an existing result.
    let mut csv = config
        .csv
        .as_ref()
        .map(|path| -> io::Result<_> {
            let file = OpenOptions::new().write(true).create_new(true).open(path)?;
            Method::Fsync.sync(&file)?;
            Ok(BufWriter::with_capacity(64 * 1024, file))
        })
        .transpose()?;
    let scratch = ScratchDir::new(&config.dir, "fsync-bench")?;
    println!("Target: {}", fs::canonicalize(&config.dir)?.display());
    println!("{} rounds; {} measured batches + {} warmup batches/case; {} writes/batch; overwrite/fallocate file {} bytes",
        config.rounds, config.iterations, config.warmup, config.sync_every, config.file_size);
    println!("Buffered pwrite; single thread; setup/warmup/drain excluded; method order rotates each round.");
    if config.methods.contains(&Method::None) {
        println!("none = NO per-batch durability. Its final fsync is reported as drain_ms, outside MiB/s.");
    }
    println!("\nround workload    block method         MiB/s   batches/s write_avg_us sync_avg_us sync_p50_us sync_p95_us sync_p99_us batch_p99_us  drain_ms");
    let mut samples = Vec::new();
    for round in 0..config.rounds {
        for &workload in &config.workloads {
            for &block in &config.block_sizes {
                for slot in 0..config.methods.len() {
                    let method = config.methods[(slot + round) % config.methods.len()];
                    let sample =
                        run_case(&config, scratch.path(), workload, method, block, round + 1)?;
                    sample.print();
                    samples.push(sample);
                }
            }
        }
    }
    if config.methods.contains(&Method::Fsync) && config.methods.contains(&Method::Fdatasync) {
        print_comparison(&config, &samples);
    }
    if let Some(out) = csv.as_mut() {
        writeln!(out, "round,workload,block_bytes,method,sync_every,iterations,warmup,writes,sync_calls,measured_bytes,elapsed_s,mib_s,batches_s,write_batch_avg_us,sync_avg_us,sync_p50_us,sync_p95_us,sync_p99_us,batch_p99_us,drain_ms")?;
        for sample in &samples {
            sample.write_csv(out, &config)?;
        }
        out.flush()?;
        Method::Fsync.sync(out.get_ref())?;
        println!(
            "\nCSV: {}",
            fs::canonicalize(config.csv.as_ref().unwrap())?.display()
        );
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}\nUse --help for usage.");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_counts_sizes_and_overflows() {
        for args in [
            vec!["--iterations", "0"],
            vec!["--sync-every", "0"],
            vec!["--rounds", "0"],
            vec!["--block-sizes", "0"],
            vec!["--block-sizes", "4K,4KiB"],
            vec!["--methods", "none,none"],
            vec!["--block-sizes", "2M", "--file-size", "1M"],
            vec![
                "--workloads",
                "fallocate",
                "--block-sizes",
                "4K",
                "--iterations",
                "2",
                "--warmup",
                "1",
                "--sync-every",
                "2",
                "--file-size",
                "20K",
            ],
            vec!["--iterations", "18446744073709551615", "--warmup", "1"],
        ] {
            assert!(Config::parse(args.into_iter().map(str::to_owned)).is_err());
        }
        assert!(parse_size("18446744073709551615G").is_err());
        assert_eq!(parse_size("16KiB").unwrap(), 16384);
    }

    #[test]
    fn fallocate_sets_length_and_sequential_writes_preserve_it() {
        let dir = ScratchDir::new(&env::temp_dir(), "fsync-bench-test").unwrap();
        let file = File::create_new(dir.path().join("preallocated.dat")).unwrap();
        let file_size = 16384;
        prepare(&file, Workload::Fallocate, file_size).unwrap();
        assert_eq!(file.metadata().unwrap().len(), file_size);
        let mut untouched = [1_u8; 8];
        file.read_exact_at(&mut untouched, 8192).unwrap();
        assert_eq!(untouched, [0; 8]);
        let mut data = payload(4096);
        let mut position = 0;
        let mut sequence = 1;
        for expected_tag in 1..=3_u64 {
            write_batch(
                &file,
                &mut data,
                Workload::Fallocate,
                file_size,
                1,
                &mut position,
                &mut sequence,
            )
            .unwrap();
            Method::Fdatasync.sync(&file).unwrap();
            assert_eq!(file.metadata().unwrap().len(), file_size);
            let mut tag = [0; 8];
            file.read_exact_at(&mut tag, (expected_tag - 1) * 4096)
                .unwrap();
            assert_eq!(u64::from_le_bytes(tag), expected_tag);
        }
        assert_eq!(position, 12288);
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let values: Vec<_> = (1..=100).rev().map(f64::from).collect();
        let stats = Stats::new(&values);
        assert_eq!((stats.p50, stats.p95, stats.p99), (50.0, 95.0, 99.0));
        assert_eq!(stats.mean, 50.5);
        assert_eq!(Stats::new(&[7.0]).p99, 7.0);
    }

    #[test]
    fn overwrite_wraps_without_growing_and_append_extends() {
        let dir = ScratchDir::new(&env::temp_dir(), "fsync-bench-test").unwrap();
        let file = File::create_new(dir.path().join("test.dat")).unwrap();
        prepare(&file, Workload::Overwrite, 24).unwrap();
        let mut data = payload(16);
        let mut position = 0;
        let mut sequence = 1;
        write_batch(
            &file,
            &mut data,
            Workload::Overwrite,
            24,
            3,
            &mut position,
            &mut sequence,
        )
        .unwrap();
        assert_eq!(file.metadata().unwrap().len(), 24);
        let mut tag = [0; 8];
        file.read_exact_at(&mut tag, 0).unwrap();
        assert_eq!(u64::from_le_bytes(tag), 3);
        position = 24;
        write_batch(
            &file,
            &mut data,
            Workload::Append,
            24,
            2,
            &mut position,
            &mut sequence,
        )
        .unwrap();
        assert_eq!(file.metadata().unwrap().len(), 56);
        file.read_exact_at(&mut tag, 40).unwrap();
        assert_eq!(u64::from_le_bytes(tag), 5);
        Method::Fsync.sync(&file).unwrap();
    }
}
