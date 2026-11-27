use std::time::{Duration, Instant};

const BATCH_SIZE: usize = 300;

/// Bounded CPU timings. DRM event times are callback arrival times, not hardware timestamps.
pub struct FrameMetrics {
    backend: &'static str,
    enabled: bool,
    preparation: Vec<Duration>,
    submission: Vec<Duration>,
    presentation: Vec<Duration>,
    pending: Option<Instant>,
}

impl FrameMetrics {
    pub fn new(backend: &'static str) -> Self {
        let enabled = std::env::var_os("VANTAGE_FRAME_METRICS").is_some_and(|v| v == "1");
        if enabled {
            eprintln!(
                "Frame metrics enabled ({backend}): CPU wall times; reports every {BATCH_SIZE} frames and on exit"
            );
        }
        Self {
            backend,
            enabled,
            preparation: Vec::new(),
            submission: Vec::new(),
            presentation: Vec::new(),
            pending: None,
        }
    }

    pub fn start(&self) -> Option<Instant> {
        self.enabled.then(Instant::now)
    }

    pub fn submitted(&mut self, start: Option<Instant>, submit_start: Option<Instant>) {
        if let (Some(start), Some(submit_start)) = (start, submit_start) {
            let now = Instant::now();
            self.preparation.push(submit_start.duration_since(start));
            self.submission.push(now.duration_since(submit_start));
            if self.backend == "native DRM" {
                self.pending = Some(now);
            } else if self.preparation.len() == BATCH_SIZE {
                self.report();
            }
        }
    }

    pub fn presented(&mut self) {
        if let Some(submitted) = self.pending.take() {
            self.presentation.push(submitted.elapsed());
            if self.preparation.len() == BATCH_SIZE {
                self.report();
            }
        }
    }

    pub fn cancel_pending(&mut self) {
        self.pending = None;
        if self.preparation.len() == BATCH_SIZE {
            self.report();
        }
    }

    fn report(&mut self) {
        if self.preparation.is_empty() {
            return;
        }
        eprintln!(
            "Frame metrics ({}): {} submitted frames",
            self.backend,
            self.preparation.len()
        );
        print_stats("CPU preparation", &mut self.preparation);
        print_stats("CPU submission", &mut self.submission);
        if self.presentation.is_empty() {
            eprintln!("  DRM callback latency: unavailable");
        } else {
            print_stats("DRM submit-to-callback latency", &mut self.presentation);
        }
        eprintln!("  Hardware presentation times, missed intervals and effect costs: unavailable");
        self.preparation.clear();
        self.submission.clear();
        self.presentation.clear();
    }
}

impl Drop for FrameMetrics {
    fn drop(&mut self) {
        self.report();
    }
}

fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    sorted[(sorted.len() * percent).div_ceil(100).saturating_sub(1)]
}

fn print_stats(label: &str, samples: &mut [Duration]) {
    samples.sort_unstable();
    let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
    eprintln!(
        "  {label}: n={} median={:.3} ms p95={:.3} ms p99={:.3} ms",
        samples.len(),
        ms(percentile(samples, 50)),
        ms(percentile(samples, 95)),
        ms(percentile(samples, 99))
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles_include_tail() {
        let values: Vec<_> = (1..=100).map(Duration::from_millis).collect();
        assert_eq!(percentile(&values, 50), Duration::from_millis(50));
        assert_eq!(percentile(&values, 95), Duration::from_millis(95));
        assert_eq!(percentile(&values, 99), Duration::from_millis(99));
        assert_eq!(percentile(&values[..1], 99), Duration::from_millis(1));
    }
}
