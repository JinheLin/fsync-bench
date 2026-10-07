/// Latency statistics, in the same units as the supplied samples.
#[derive(Debug, Default)]
pub struct Stats {
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

impl Stats {
    /// Nearest-rank percentiles. Aggregate raw samples, not per-round percentiles.
    pub fn new(samples: &[f64]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        let mut sorted = samples.to_vec();
        sorted.sort_unstable_by(f64::total_cmp);
        let percentile = |p: usize| sorted[(sorted.len() * p).div_ceil(100) - 1];
        Self {
            mean: sorted.iter().sum::<f64>() / sorted.len() as f64,
            p50: percentile(50),
            p95: percentile(95),
            p99: percentile(99),
            max: *sorted.last().unwrap(),
        }
    }
}
