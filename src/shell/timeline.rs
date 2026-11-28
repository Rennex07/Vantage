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

#[derive(Default)]
pub struct Animation {
    target: f64,
    transition: Option<Transition>,
}

impl Animation {
    pub fn value(&self, now: Instant) -> f64 {
        self.transition
            .as_ref()
            .map_or(self.target, |track| track.value(now))
    }

    pub fn retarget(&mut self, target: f64, now: Instant, duration: Duration, easing: Ease) {
        if duration.is_zero() {
            self.target = target;
            self.settle();
        } else if target != self.target {
            let from = self.value(now);
            self.transition =
                (from != target).then(|| Transition::new(from, target, now, duration, easing));
            self.target = target;
        }
    }

    pub fn is_animating(&self) -> bool {
        self.transition.is_some()
    }

    pub fn finish_frame(&mut self, now: Instant) {
        if self
            .transition
            .as_ref()
            .is_some_and(|track| track.finished(now))
        {
            self.settle();
        }
    }

    pub fn settle(&mut self) {
        self.transition = None;
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

    #[test]
    fn independent_tracks_reverse_without_jumping_and_can_snap() {
        let now = Instant::now();
        let duration = Duration::from_millis(200);
        let halfway = now + duration / 2;
        let mut overview = Animation::default();
        let mut panel = Animation::default();
        overview.retarget(1.0, now, duration, Ease::Smooth);
        panel.retarget(1.0, halfway, duration, Ease::Out);
        overview.retarget(0.0, halfway, duration, Ease::Smooth);
        assert_eq!(overview.value(halfway), 0.5);
        assert_eq!(panel.value(halfway), 0.0);
        panel.retarget(1.0, halfway, Duration::ZERO, Ease::Out);
        assert_eq!(panel.value(halfway), 1.0);
        assert!(!panel.is_animating());
        let end = halfway + duration;
        assert_eq!(overview.value(end), 0.0);
        overview.finish_frame(end);
        assert!(!overview.is_animating());
    }
}
