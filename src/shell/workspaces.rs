use std::time::{Duration, Instant};

pub const WORKSPACE_COUNT: usize = 3;
const SWITCH: Duration = Duration::from_millis(300);

#[derive(Default)]
pub struct Workspaces {
    pub active: usize,
    slide: Option<Slide>,
}

struct Slide {
    from: f64,
    started: Instant,
}

impl Workspaces {
    pub fn select(&mut self, index: usize, now: Instant) {
        if index >= WORKSPACE_COUNT || index == self.active {
            return;
        }
        self.slide = Some(Slide {
            from: self.position(now),
            started: now,
        });
        self.active = index;
    }

    pub fn position(&self, now: Instant) -> f64 {
        let target = self.active as f64;
        self.slide.as_ref().map_or(target, |slide| {
            let t = progress(now, slide.started, SWITCH);
            let eased = t * t * (3.0 - 2.0 * t);
            slide.from + (target - slide.from) * eased
        })
    }

    pub fn is_animating(&self) -> bool {
        self.slide.is_some()
    }

    pub fn finish_frame(&mut self, now: Instant) {
        if self
            .slide
            .as_ref()
            .is_some_and(|slide| now.duration_since(slide.started) >= SWITCH)
        {
            self.slide = None;
        }
    }
}

pub fn progress(now: Instant, start: Instant, duration: Duration) -> f64 {
    (now.saturating_duration_since(start).as_secs_f64() / duration.as_secs_f64()).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_again_continues_from_the_visible_position() {
        let start = Instant::now();
        let midway = start + SWITCH / 2;
        let mut workspaces = Workspaces::default();
        workspaces.select(2, start);
        assert_eq!(workspaces.position(midway), 1.0);
        workspaces.select(0, midway);
        assert_eq!(workspaces.position(midway), 1.0);
        assert_eq!(workspaces.position(midway + SWITCH), 0.0);
        workspaces.finish_frame(midway + SWITCH);
        assert!(!workspaces.is_animating());
    }
}
