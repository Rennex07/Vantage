use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub enum Ease {
    Out,
    Smooth,
}

impl Ease {
    pub fn sample(self, phase: f64) -> f64 {
        match self {
            Self::Out => 1.0 - (1.0 - phase).powi(3),
            Self::Smooth => phase * phase * (3.0 - 2.0 * phase),
        }
    }
}

pub fn progress(now: Instant, start: Instant, duration: Duration) -> f64 {
    if duration.is_zero() {
        return 1.0;
    }
    (now.saturating_duration_since(start).as_secs_f64() / duration.as_secs_f64()).min(1.0)
}

pub struct Transition {
    from: f64,
    to: f64,
    start: Instant,
    duration: Duration,
    easing: Ease,
}

impl Transition {
    pub fn new(from: f64, to: f64, start: Instant, duration: Duration, easing: Ease) -> Self {
        Self {
            from,
            to,
            start,
            duration,
            easing,
        }
    }

    pub fn value(&self, now: Instant) -> f64 {
        let phase = progress(now, self.start, self.duration);
        if phase == 1.0 {
            return self.to;
        }
        self.from + (self.to - self.from) * self.easing.sample(phase)
    }

    pub fn finished(&self, now: Instant) -> bool {
        progress(now, self.start, self.duration) == 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_retargeted_transition_keeps_the_current_pose() {
        let now = Instant::now();
        let duration = Duration::from_millis(200);
        let forward = Transition::new(0.0, 1.0, now, duration, Ease::Out);
        let reverse_at = now + duration / 2;
        let visible = forward.value(reverse_at);
        let reverse = Transition::new(visible, 0.0, reverse_at, duration, Ease::Out);
        assert_eq!(reverse.value(reverse_at), visible);
        assert!(reverse.value(reverse_at + duration / 2) < visible);
        assert_eq!(reverse.value(reverse_at + duration), 0.0);
        assert!(reverse.finished(reverse_at + duration));
    }

    #[test]
    fn clock_samples_before_start_and_immediate_transitions_are_defined() {
        let now = Instant::now();
        let duration = Duration::from_millis(100);
        let forward = Transition::new(2.0, 4.0, now + duration, duration, Ease::Smooth);
        assert_eq!(forward.value(now), 2.0);
        assert!(!forward.finished(now));
        let immediate = Transition::new(2.0, 4.0, now, Duration::ZERO, Ease::Smooth);
        assert_eq!(immediate.value(now), 4.0);
        assert!(immediate.finished(now));
    }
}
