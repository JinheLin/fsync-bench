//! Shared infrastructure for the fsync and WAL micro-benchmarks.
//! Timing loops remain in the binaries so the measured operations are easy to audit.

#[cfg(not(target_os = "linux"))]
compile_error!("These benchmarks require Linux file synchronization semantics.");

mod cli;
mod linux;
mod stats;

pub use cli::{parse_size, parse_unique, Result};
pub use linux::{
    discard_clean_pages, fill_payload, payload, preallocate, AlignedBuffer, ScratchDir, SyncMethod,
};
pub use stats::Stats;
