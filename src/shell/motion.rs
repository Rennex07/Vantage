use std::time::{Duration, Instant};

use smithay::utils::{Logical, Point, Rectangle, Size};

use super::{
    launch::App,
    timeline::{Ease, progress},
};

pub const LAUNCH_TIMEOUT: Duration = Duration::from_secs(6);
const LEAVE_DOCK: Duration = Duration::from_millis(145);
const UNFOLD: Duration = Duration::from_millis(330);
const OTHER_OPENING: Duration = Duration::from_millis(230);

#[derive(Clone, Copy)]
pub struct Launch {
    pub app: App,
    pub started: Instant,
    pub anchor: Point<f64, Logical>,
    pub workspace: usize,
}

impl Launch {
    pub fn tile(self, now: Instant) -> Rectangle<f64, Logical> {
        let phase = progress(now, self.started, LEAVE_DOCK);
        let lift = Ease::Out.sample(phase);
        let size = Size::from((44.0 - 14.0 * lift, 39.0 - 19.0 * lift));
        let center = self.anchor - Point::from((0.0, 54.0 * lift));
        Rectangle::new(center - Point::from((size.w / 2.0, size.h / 2.0)), size)
    }
}

pub struct Opening {
    start: Instant,
    launch: Option<Launch>,
}

impl Opening {
    pub fn new(now: Instant, launch: Option<Launch>) -> Self {
        let start = launch.map_or(now, |launch| now.max(launch.started + LEAVE_DOCK));
        Self { start, launch }
    }

    pub fn active(&self, now: Instant) -> bool {
        let duration = if self.launch.is_some() {
            UNFOLD
        } else {
            OTHER_OPENING
        };
        now < self.start + duration
    }

    pub fn pose(&self, now: Instant, target: Rectangle<f64, Logical>) -> Pose {
        let Some(launch) = self.launch else {
            let p = Ease::Out.sample(progress(now, self.start, OTHER_OPENING));
            return Pose {
                rect: Rectangle::new(
                    target.loc + Point::from((0.0, 22.0 * (1.0 - p))),
                    target.size,
                ),
                opacity: (0.24 + 0.76 * p) as f32,
                card: None,
            };
        };
        if now < self.start {
            return Pose {
                rect: launch.tile(now),
                opacity: 0.0,
                card: Some((launch.app, 1.0)),
            };
        }
        let p = progress(now, self.start, UNFOLD);
        let origin = launch.tile(self.start);
        let from = center(origin);
        let to = center(target);
        let flight = Ease::Smooth.sample((p / 0.46).min(1.0));
        let expand = Ease::Out.sample(((p - 0.22) / 0.78).clamp(0.0, 1.0));
        let reveal = Ease::Smooth.sample(((p - 0.46) / 0.44).clamp(0.0, 1.0)) as f32;
        let size = origin.size + (target.size - origin.size).upscale(expand);
        let position = from + (to - from).upscale(flight);
        Pose {
            rect: Rectangle::new(position - Point::from((size.w / 2.0, size.h / 2.0)), size),
            opacity: reveal,
            card: (reveal < 1.0).then_some((launch.app, 1.0 - reveal)),
        }
    }
}

pub struct Pose {
    pub rect: Rectangle<f64, Logical>,
    pub opacity: f32,
    pub card: Option<(App, f32)>,
}

fn center(rect: Rectangle<f64, Logical>) -> Point<f64, Logical> {
    rect.loc + Point::from((rect.size.w / 2.0, rect.size.h / 2.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_client_starts_at_the_dock_and_unfolds_into_its_window() {
        let start = Instant::now();
        let launch = Launch {
            app: App::Console,
            started: start,
            anchor: (470.0, 670.0).into(),
            workspace: 0,
        };
        let opening = Opening::new(start, Some(launch));
        let target = Rectangle::new((260.0, 135.0).into(), (760.0, 420.0).into());
        let initial = opening.pose(start, target);
        assert_eq!(center(initial.rect), launch.anchor);
        assert_eq!(initial.opacity, 0.0);
        let flight = opening.pose(start + LEAVE_DOCK + Duration::from_millis(90), target);
        assert!(center(flight.rect).y < launch.tile(start + LEAVE_DOCK).loc.y);
        assert!(flight.rect.size.w < target.size.w);
        let end = start + LEAVE_DOCK + UNFOLD;
        let settled = opening.pose(end, target);
        assert_eq!(settled.rect, target);
        assert_eq!(settled.opacity, 1.0);
        assert!(!opening.active(end));
    }
}
