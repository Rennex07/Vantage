use std::time::{Duration, Instant};

use super::timeline::{Animation, Ease};

pub const WORKSPACE_COUNT: usize = 3;
const SWITCH: Duration = Duration::from_millis(300);

#[derive(Default)]
pub struct Workspaces {
    pub active: usize,
    slide: Animation,
}

impl Workspaces {
    pub fn select(&mut self, index: usize, now: Instant, animate: bool) {
        if index >= WORKSPACE_COUNT || index == self.active {
            return;
        }
        self.slide.retarget(
            index as f64,
            now,
            if animate { SWITCH } else { Duration::ZERO },
            Ease::Smooth,
        );
        self.active = index;
    }

    pub fn position(&self, now: Instant) -> f64 {
        self.slide.value(now)
    }

    pub fn is_animating(&self) -> bool {
        self.slide.is_animating()
    }

    pub fn finish_frame(&mut self, now: Instant) {
        self.slide.finish_frame(now);
    }

    pub fn settle(&mut self) {
        self.slide.settle();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_again_continues_from_the_visible_position() {
        let start = Instant::now();
        let midway = start + SWITCH / 2;
        let mut workspaces = Workspaces::default();
        workspaces.select(2, start, true);
        assert_eq!(workspaces.position(midway), 1.0);
        workspaces.select(0, midway, true);
        assert_eq!(workspaces.position(midway), 1.0);
        assert_eq!(workspaces.position(midway + SWITCH), 0.0);
        workspaces.finish_frame(midway + SWITCH);
        assert!(!workspaces.is_animating());
    }
}
