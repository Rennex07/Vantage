use std::time::{Duration, Instant};

const BATCH_SIZE: usize = 300;

/// Bounded CPU timings. DRM event times are callback arrival times, not hardware timestamps.
pub struct FrameMetrics {
    backend: &'static str,
    enabled: bool,
    binding: Vec<Duration>,
    drawing: Vec<Duration>,
    submission: Vec<Duration>,
    presentation: Vec<Duration>,
    pending: Option<Instant>,
    frame_budget: Option<Duration>,
    over_budget: usize,
    animation: bool,
    previous_sequence: Option<u32>,
    sequence_samples: usize,
    extra_intervals: u64,
    cancelled: usize,
}

impl FrameMetrics {
    pub fn new(backend: &'static str, refresh: Option<i32>) -> Self {
        let enabled = std::env::var_os("VANTAGE_FRAME_METRICS").is_some_and(|v| v == "1");
        if enabled {
            eprintln!(
                "Frame metrics enabled ({backend}): CPU wall times; reports every {BATCH_SIZE} frames and on exit"
            );
        }
        Self {
            backend,
            enabled,
            binding: Vec::new(),
            drawing: Vec::new(),
            submission: Vec::new(),
            presentation: Vec::new(),
            pending: None,
            frame_budget: refresh
                .filter(|&refresh| refresh > 0)
                .map(|refresh| Duration::from_nanos(1_000_000_000_000 / refresh as u64)),
            over_budget: 0,
            animation: false,
            previous_sequence: None,
            sequence_samples: 0,
            extra_intervals: 0,
            cancelled: 0,
        }
    }

    pub fn start(&self) -> Option<Instant> {
        self.enabled.then(Instant::now)
    }

    pub fn submitted(
        &mut self,
        start: Option<Instant>,
        draw_start: Option<Instant>,
        submit_start: Option<Instant>,
    ) {
        if let (Some(start), Some(draw_start), Some(submit_start)) =
            (start, draw_start, submit_start)
        {
            let now = Instant::now();
            self.binding.push(draw_start.duration_since(start));
            self.drawing.push(submit_start.duration_since(draw_start));
            self.submission.push(now.duration_since(submit_start));
            if self
                .frame_budget
                .is_some_and(|budget| now.duration_since(start) > budget)
            {
                self.over_budget += 1;
            }
            if self.backend == "native DRM" {
                self.pending = Some(now);
            } else if self.drawing.len() == BATCH_SIZE {
                self.report();
            }
        }
    }

    // Reset the comparison across idle periods, VT switches and non-animation
    // draws: a gap in page flips is normal when nothing needs repainting.
    pub fn animation_frame(&mut self, active: bool) {
        if !active {
            self.previous_sequence = None;
        }
        self.animation = active;
    }

    pub fn presented(&mut self, sequence: Option<u32>) {
        if let Some(submitted) = self.pending.take() {
            self.presentation.push(submitted.elapsed());
            if self.animation {
                if let (Some(previous), Some(current)) = (self.previous_sequence, sequence)
                    && let Some(extra) = extra_intervals(previous, current)
                {
                    self.sequence_samples += 1;
                    self.extra_intervals += u64::from(extra);
                }
                self.previous_sequence = sequence;
            } else {
                self.previous_sequence = None;
            }
            if self.drawing.len() == BATCH_SIZE {
                self.report();
            }
        }
    }

    pub fn cancel_pending(&mut self) {
        if self.pending.take().is_some() {
            self.cancelled += 1;
        }
        self.previous_sequence = None;
        if self.drawing.len() == BATCH_SIZE {
            self.report();
        }
    }

    fn report(&mut self) {
        if self.drawing.is_empty() {
            return;
        }
        eprintln!(
            "Frame metrics ({}): {} submitted frames",
            self.backend,
            self.drawing.len()
        );
        print_stats("CPU buffer bind", &mut self.binding);
        print_stats("CPU scene draw", &mut self.drawing);
        print_stats("CPU submission", &mut self.submission);
        if self.presentation.is_empty() {
            eprintln!("  DRM callback latency: unavailable");
        } else {
            print_stats("DRM submit-to-callback latency", &mut self.presentation);
        }
        if let Some(budget) = self.frame_budget {
            eprintln!(
                "  CPU frames over {:.3} ms mode interval: {}",
                budget.as_secs_f64() * 1000.0,
                self.over_budget,
            );
        }
        if self.backend == "native DRM" {
            eprintln!(
                "  Continuous animation DRM sequence samples: {}; extra refresh intervals: {}; cancelled submissions: {}",
                self.sequence_samples, self.extra_intervals, self.cancelled
            );
        }
        self.sequence_samples = 0;
        self.extra_intervals = 0;
        self.cancelled = 0;
        self.over_budget = 0;
        self.binding.clear();
        self.drawing.clear();
        self.submission.clear();
        self.presentation.clear();
    }
}

impl Drop for FrameMetrics {
    fn drop(&mut self) {
        self.report();
    }
}

fn extra_intervals(previous: u32, current: u32) -> Option<u32> {
    let delta = current.wrapping_sub(previous);
    // Duplicate or discontinuous counters are not reliable pacing samples.
    (1..=1000).contains(&delta).then(|| delta - 1)
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
    fn sequence_gaps_count_only_valid_forward_intervals() {
        assert_eq!(extra_intervals(8, 9), Some(0));
        assert_eq!(extra_intervals(8, 11), Some(2));
        assert_eq!(extra_intervals(u32::MAX, 0), Some(0));
        assert_eq!(extra_intervals(8, 8), None);
        assert_eq!(extra_intervals(9, 8), None);
    }

    #[test]
    fn nearest_rank_percentiles_include_tail() {
        let values: Vec<_> = (1..=100).map(Duration::from_millis).collect();
        assert_eq!(percentile(&values, 50), Duration::from_millis(50));
        assert_eq!(percentile(&values, 95), Duration::from_millis(95));
        assert_eq!(percentile(&values, 99), Duration::from_millis(99));
        assert_eq!(percentile(&values[..1], 99), Duration::from_millis(1));
    }
}
