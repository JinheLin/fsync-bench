#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
compile_error!("wal-bench currently targets Linux x86_64 (Linux O_DIRECT flag ABI).");

use std::alloc::Layout;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::os::raw::c_int;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Instant;

use fsync_bench::{
    discard_clean_pages, fill_payload as fill_record, parse_size, parse_unique as list,
    preallocate as allocate, AlignedBuffer, Result, ScratchDir, Stats, SyncMethod,
};
const O_DIRECT: c_int = 0o40000;
const HELP: &str = "\
wal-bench: sequential single-writer WAL, ONE explicit sync after EVERY write

Usage: wal-bench [OPTIONS]
  --dir PATH           Directory on the target filesystem [./bench-data]
  --block-sizes LIST   Bytes per WAL write [4K,16K,64K]
  --iterations N       Measured writes AND syncs/case/round [500]
  --warmup N           Unmeasured writes AND syncs/case [50]
  --rounds N           Rotate the four I/O/sync combinations [4]
  --file-size SIZE     Initialized/preallocated WAL segment size [64M]
  --layouts LIST       initialized,fallocate,append [initialized]
  --io-modes LIST      buffered,direct [buffered,direct]
  --methods LIST       fsync,fdatasync [fsync,fdatasync]
  --alignment SIZE     Buffer alignment and I/O size/offset multiple [4K]
  --csv PATH           Per-round results; must be a new file
  -h, --help           Show help

Binary size suffixes: K/KiB, M/MiB, G/GiB. Options accept --key=value.
Writes never wrap; initialized/fallocate segments must hold warmup + measurement.
All modes write identical aligned byte counts. No per-write padding or batching.
Preparation, warmup, final drain, readback checks and cleanup are excluded.
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IoMode {
    Buffered,
    Direct,
}

impl IoMode {
    fn name(self) -> &'static str {
        match self {
            Self::Buffered => "buffered",
            Self::Direct => "direct",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileLayout {
    Initialized,
    Fallocate,
    Append,
}

impl FileLayout {
    fn name(self) -> &'static str {
        match self {
            Self::Initialized => "initialized",
            Self::Fallocate => "fallocate",
            Self::Append => "append",
        }
    }
}

struct Config {
    dir: PathBuf,
    blocks: Vec<usize>,
    iterations: usize,
    warmup: usize,
    rounds: usize,
    file_size: u64,
    layouts: Vec<FileLayout>,
    modes: Vec<IoMode>,
    methods: Vec<SyncMethod>,
    alignment: usize,
    csv: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            dir: PathBuf::from("./bench-data"),
            blocks: vec![4096, 16384, 65536],
            iterations: 500,
            warmup: 50,
            rounds: 4,
            file_size: 64 * 1024 * 1024,
            layouts: vec![FileLayout::Initialized],
            modes: vec![IoMode::Buffered, IoMode::Direct],
            methods: vec![SyncMethod::Fsync, SyncMethod::Fdatasync],
            alignment: 4096,
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
                    config.blocks = list(&value, |s| Ok(usize::try_from(parse_size(s)?)?))?
                }
                "--iterations" => config.iterations = value.parse()?,
                "--warmup" => config.warmup = value.parse()?,
                "--rounds" => config.rounds = value.parse()?,
                "--file-size" => config.file_size = parse_size(&value)?,
                "--alignment" => config.alignment = usize::try_from(parse_size(&value)?)?,
                "--layouts" => {
                    config.layouts = list(&value, |s| match s {
                        "initialized" => Ok(FileLayout::Initialized),
                        "fallocate" => Ok(FileLayout::Fallocate),
                        "append" => Ok(FileLayout::Append),
                        _ => Err(format!("unknown layout: {s}").into()),
                    })?
                }
                "--io-modes" => {
                    config.modes = list(&value, |s| match s {
                        "buffered" => Ok(IoMode::Buffered),
                        "direct" => Ok(IoMode::Direct),
                        _ => Err(format!("unknown I/O mode: {s}").into()),
                    })?
                }
                "--methods" => {
                    config.methods = list(&value, |s| match s {
                        "fsync" => Ok(SyncMethod::Fsync),
                        "fdatasync" => Ok(SyncMethod::Fdatasync),
                        _ => Err(format!(
                            "unknown sync method: {s}; every WAL write requires fsync or fdatasync"
                        )
                        .into()),
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
        if self.iterations == 0 || self.rounds == 0 {
            return Err("iterations and rounds must be positive".into());
        }
        if self.alignment < 16 || !self.alignment.is_power_of_two() {
            return Err("alignment must be a power of two >= 16 bytes".into());
        }
        if self.file_size == 0 || self.file_size > i64::MAX as u64 {
            return Err("file-size must be positive and fit a Linux file offset".into());
        }
        let writes = self
            .iterations
            .checked_add(self.warmup)
            .ok_or("write count overflow")?;
        for &block in &self.blocks {
            if block < 16 || block % self.alignment != 0 {
                return Err(format!(
                    "block {block} must be >= 16 and a multiple of alignment {}",
                    self.alignment
                )
                .into());
            }
            Layout::from_size_align(block, self.alignment)?;
            let bytes = (writes as u64)
                .checked_mul(block as u64)
                .ok_or("WAL byte count overflow")?;
            if bytes > i64::MAX as u64 {
                return Err("WAL byte count exceeds a Linux file offset".into());
            }
            if self
                .layouts
                .iter()
                .any(|layout| *layout != FileLayout::Append)
                && bytes > self.file_size
            {
                return Err(format!("file-size must be >= {bytes} bytes for block {block}, including warmup; WAL writes never wrap").into());
            }
        }
        Ok(())
    }
}

fn prepare(path: &Path, layout: FileLayout, bytes: u64) -> io::Result<()> {
    let file = File::create_new(path)?;
    if layout != FileLayout::Append {
        allocate(&file, bytes)?;
    }
    if layout == FileLayout::Initialized {
        let zeros = vec![0; 1024 * 1024];
        let mut offset = 0;
        while offset < bytes {
            let n = (bytes - offset).min(zeros.len() as u64) as usize;
            file.write_all_at(&zeros[..n], offset)?;
            offset += n as u64;
        }
    }
    SyncMethod::Fsync.sync(&file)?;
    discard_clean_pages(&file)?;
    // The buffered preparation descriptor closes before a direct descriptor opens.
    Ok(())
}

fn set_header(data: &mut [u8], sequence: u64, end_lsn: u64) {
    data[..8].copy_from_slice(&sequence.to_le_bytes());
    data[8..16].copy_from_slice(&end_lsn.to_le_bytes());
}

fn write_record(file: &File, data: &[u8], offset: u64) -> io::Result<()> {
    loop {
        match file.write_at(data, offset) {
            Ok(n) if n == data.len() => return Ok(()),
            // A short direct write can leave the remainder unaligned. Abort the
            // case rather than count a partial WAL record as a committed write.
            Ok(n) => {
                return Err(io::Error::other(format!(
                    "short WAL write: {n}/{} bytes at offset {offset}",
                    data.len()
                )))
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
}

struct Measurement {
    round: usize,
    layout: FileLayout,
    mode: IoMode,
    method: SyncMethod,
    block: usize,
    elapsed_s: f64,
    write_us: Vec<f64>,
    sync_us: Vec<f64>,
    commit_us: Vec<f64>,
}

fn verify_record(file: &File, offset: u64, sequence: u64, reference: &[u8]) -> io::Result<()> {
    let mut data = vec![0; reference.len()];
    file.read_exact_at(&mut data, offset)?;
    if data[..8] != sequence.to_le_bytes()
        || data[8..16] != (offset + reference.len() as u64).to_le_bytes()
        || data[16..] != reference[16..]
    {
        return Err(io::Error::other(format!(
            "WAL readback mismatch at offset {offset}"
        )));
    }
    Ok(())
}

fn run_case(
    config: &Config,
    dir: &Path,
    layout: FileLayout,
    block: usize,
    mode: IoMode,
    method: SyncMethod,
    round: usize,
) -> Result<Measurement> {
    let path = dir.join("segment.wal");
    prepare(&path, layout, config.file_size)?;
    SyncMethod::Fsync.sync(&File::open(dir)?)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if mode == IoMode::Direct {
        options.custom_flags(O_DIRECT);
    }
    let file = options.open(&path)?;
    let mut data = AlignedBuffer::new(block, config.alignment)?;
    fill_record(data.bytes_mut());
    let mut offset = 0;
    let mut sequence = 1;
    for _ in 0..config.warmup {
        set_header(data.bytes_mut(), sequence, offset + block as u64);
        write_record(&file, data.bytes(), offset)?;
        method.sync(&file)?;
        offset += block as u64;
        sequence += 1;
    }
    SyncMethod::Fsync.sync(&file)?;
    let first_measured_offset = offset;
    let first_measured_sequence = sequence;
    let mut result = Measurement {
        round,
        layout,
        mode,
        method,
        block,
        elapsed_s: 0.0,
        write_us: Vec::with_capacity(config.iterations),
        sync_us: Vec::with_capacity(config.iterations),
        commit_us: Vec::with_capacity(config.iterations),
    };
    let start = Instant::now();
    for _ in 0..config.iterations {
        let commit_start = Instant::now();
        set_header(data.bytes_mut(), sequence, offset + block as u64);
        write_record(&file, data.bytes(), offset)?;
        let written = Instant::now();
        method.sync(&file)?;
        let durable = Instant::now();
        result
            .write_us
            .push((written - commit_start).as_secs_f64() * 1e6);
        result.sync_us.push((durable - written).as_secs_f64() * 1e6);
        result
            .commit_us
            .push((durable - commit_start).as_secs_f64() * 1e6);
        offset += block as u64;
        sequence += 1;
    }
    result.elapsed_s = start.elapsed().as_secs_f64();
    SyncMethod::Fsync.sync(&file)?;
    let expected_size = if layout == FileLayout::Append {
        offset
    } else {
        config.file_size
    };
    if file.metadata()?.len() != expected_size {
        return Err("unexpected WAL segment size".into());
    }
    drop(file);
    // Verify through a fresh buffered descriptor only after all direct I/O finishes.
    let reader = File::open(&path)?;
    verify_record(
        &reader,
        first_measured_offset,
        first_measured_sequence,
        data.bytes(),
    )?;
    verify_record(&reader, offset - block as u64, sequence - 1, data.bytes())?;
    drop(reader);
    fs::remove_file(path)?;
    SyncMethod::Fsync.sync(&File::open(dir)?)?;
    Ok(result)
}

fn print_row(round: &str, sample: &Measurement) {
    let write = Stats::new(&sample.write_us);
    let sync = Stats::new(&sample.sync_us);
    let commit = Stats::new(&sample.commit_us);
    let commits_s = sample.commit_us.len() as f64 / sample.elapsed_s;
    let mib_s = commits_s * sample.block as f64 / (1024.0 * 1024.0);
    println!("{:>5} {:<11} {:>7} {:<8} {:<9} {:>9.2} {:>11.1} {:>12.2} {:>11.2} {:>12.2} {:>12.2} {:>12.2} {:>12.2}",
        round, sample.layout.name(), sample.block, sample.mode.name(), sample.method.name(), mib_s, commits_s,
        write.mean, sync.mean, commit.p50, commit.p95, commit.p99, commit.max);
}

const CSV_HEADER: &str = "round,layout,block_bytes,io_mode,sync_method,alignment_bytes,segment_bytes,warmup,writes,sync_calls,measured_bytes,elapsed_s,mib_s,commits_s,write_avg_us,write_p50_us,write_p95_us,write_p99_us,sync_avg_us,sync_p50_us,sync_p95_us,sync_p99_us,commit_avg_us,commit_p50_us,commit_p95_us,commit_p99_us,commit_max_us";

impl Measurement {
    fn csv(&self, out: &mut impl Write, config: &Config) -> io::Result<()> {
        let write = Stats::new(&self.write_us);
        let sync = Stats::new(&self.sync_us);
        let commit = Stats::new(&self.commit_us);
        let count = self.commit_us.len();
        let commits_s = count as f64 / self.elapsed_s;
        let mib_s = commits_s * self.block as f64 / (1024.0 * 1024.0);
        let segment = if self.layout == FileLayout::Append {
            (config.warmup + config.iterations) as u64 * self.block as u64
        } else {
            config.file_size
        };
        writeln!(out, "{},{},{},{},{},{},{},{},{},{},{},{:.9},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}",
            self.round, self.layout.name(), self.block, self.mode.name(), self.method.name(),
            config.alignment, segment, config.warmup, count, self.sync_us.len(), count as u64 * self.block as u64,
            self.elapsed_s, mib_s, commits_s, write.mean, write.p50, write.p95, write.p99,
            sync.mean, sync.p50, sync.p95, sync.p99, commit.mean, commit.p50, commit.p95, commit.p99, commit.max)
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{HELP}");
        return Ok(());
    }
    let config = Config::parse(args.into_iter())?;
    let mut csv = config
        .csv
        .as_ref()
        .map(|path| -> io::Result<_> {
            let file = OpenOptions::new().write(true).create_new(true).open(path)?;
            SyncMethod::Fsync.sync(&file)?;
            Ok(BufWriter::with_capacity(64 * 1024, file))
        })
        .transpose()?;
    let scratch = ScratchDir::new(&config.dir, "wal-bench")?;
    let combinations: Vec<_> = config
        .modes
        .iter()
        .flat_map(|&mode| config.methods.iter().map(move |&method| (mode, method)))
        .collect();
    println!("Target: {}", fs::canonicalize(&config.dir)?.display());
    println!("Single writer, sequential WAL records; ONE explicit sync after EVERY write; {} byte alignment.", config.alignment);
    println!("{} rounds; {} measured + {} warmup writes/case; segment {} bytes; combination order rotates each round.", config.rounds, config.iterations, config.warmup, config.file_size);
    println!("Preparation pages advised DONTNEED; setup/warmup/drain/readback/cleanup excluded. All modes use the same aligned buffer.");
    println!("\nround layout        block I/O      sync          MiB/s   commits/s write_avg_us sync_avg_us commit_p50_us commit_p95_us commit_p99_us commit_max_us");
    let mut samples = Vec::new();
    for round in 0..config.rounds {
        for &layout in &config.layouts {
            for &block in &config.blocks {
                for slot in 0..combinations.len() {
                    let (mode, method) = combinations[(slot + round) % combinations.len()];
                    let sample = run_case(
                        &config,
                        scratch.path(),
                        layout,
                        block,
                        mode,
                        method,
                        round + 1,
                    )
                    .map_err(|e| {
                        format!(
                            "{} / {} / {} / block {block}: {e}",
                            layout.name(),
                            mode.name(),
                            method.name()
                        )
                    })?;
                    print_row(&sample.round.to_string(), &sample);
                    samples.push(sample);
                }
            }
        }
    }
    println!("\nAll rounds combined: throughput from total bytes/time; percentiles from pooled per-write samples.");
    for &layout in &config.layouts {
        for &block in &config.blocks {
            for &(mode, method) in &combinations {
                let cases: Vec<_> = samples
                    .iter()
                    .filter(|s| {
                        s.layout == layout
                            && s.block == block
                            && s.mode == mode
                            && s.method == method
                    })
                    .collect();
                let elapsed_s: f64 = cases.iter().map(|s| s.elapsed_s).sum();
                let writes: Vec<_> = cases
                    .iter()
                    .flat_map(|s| s.write_us.iter().copied())
                    .collect();
                let syncs: Vec<_> = cases
                    .iter()
                    .flat_map(|s| s.sync_us.iter().copied())
                    .collect();
                let commits: Vec<_> = cases
                    .iter()
                    .flat_map(|s| s.commit_us.iter().copied())
                    .collect();
                let aggregate = Measurement {
                    round: 0,
                    layout,
                    block,
                    mode,
                    method,
                    elapsed_s,
                    write_us: writes,
                    sync_us: syncs,
                    commit_us: commits,
                };
                print_row("all", &aggregate);
            }
        }
    }
    if let Some(out) = csv.as_mut() {
        writeln!(out, "{CSV_HEADER}")?;
        for sample in &samples {
            sample.csv(out, &config)?;
        }
        out.flush()?;
        SyncMethod::Fsync.sync(out.get_ref())?;
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
    fn aligned_buffer_and_record_headers() {
        for alignment in [512, 4096, 8192] {
            let mut buffer = AlignedBuffer::new(16384, alignment).unwrap();
            assert_eq!(buffer.bytes().as_ptr() as usize % alignment, 0);
            assert!(buffer.bytes().iter().all(|&b| b == 0));
            fill_record(buffer.bytes_mut());
            set_header(buffer.bytes_mut(), 42, 65536);
            assert_eq!(&buffer.bytes()[..8], &42_u64.to_le_bytes());
            assert_eq!(&buffer.bytes()[8..16], &65536_u64.to_le_bytes());
        }
    }

    #[test]
    fn rejects_misalignment_capacity_overflow_and_missing_sync() {
        for args in [
            vec!["--block-sizes", "1K"],
            vec!["--alignment", "3000"],
            vec!["--alignment", "0"],
            vec!["--iterations", "0"],
            vec!["--rounds", "0"],
            vec!["--methods", "none"],
            vec!["--block-sizes", "4K,4K"],
            vec![
                "--file-size",
                "8K",
                "--block-sizes",
                "4K",
                "--iterations",
                "2",
                "--warmup",
                "1",
            ],
            vec!["--iterations", "18446744073709551615", "--warmup", "1"],
        ] {
            assert!(Config::parse(args.into_iter().map(str::to_owned)).is_err());
        }
        assert!(Config::parse(
            ["--alignment=512", "--block-sizes=512,1K"]
                .into_iter()
                .map(str::to_owned)
        )
        .is_ok());
    }

    #[test]
    fn readback_detects_incorrect_sequence_lsn_and_payload() {
        let dir = ScratchDir::new(&env::temp_dir(), "wal-bench-test").unwrap();
        let file = File::create_new(dir.path().join("test.wal")).unwrap();
        let mut data = AlignedBuffer::new(4096, 4096).unwrap();
        fill_record(data.bytes_mut());
        set_header(data.bytes_mut(), 5, 8192);
        write_record(&file, data.bytes(), 4096).unwrap();
        verify_record(&file, 4096, 5, data.bytes()).unwrap();
        assert!(verify_record(&file, 4096, 6, data.bytes()).is_err());
        file.write_all_at(&0_u64.to_le_bytes(), 4096 + 8).unwrap();
        assert!(verify_record(&file, 4096, 5, data.bytes()).is_err());
        set_header(data.bytes_mut(), 5, 8192);
        write_record(&file, data.bytes(), 4096).unwrap();
        let corrupted = [data.bytes()[16] ^ 0xff];
        file.write_all_at(&corrupted, 4096 + 16).unwrap();
        assert!(verify_record(&file, 4096, 5, data.bytes()).is_err());
    }

    #[test]
    fn commit_statistics_use_nearest_rank() {
        let samples: Vec<_> = (1..=100).rev().map(f64::from).collect();
        let stats = Stats::new(&samples);
        assert_eq!(
            (stats.mean, stats.p50, stats.p95, stats.p99, stats.max),
            (50.5, 50.0, 95.0, 99.0, 100.0)
        );
    }
}
