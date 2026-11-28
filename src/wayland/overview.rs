use std::time::{Duration, Instant};

use smithay::{
    desktop::Window,
    utils::{Logical, Point, Rectangle, Size},
    wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData},
};

use super::State;
use crate::shell::{
    status::BAR_HEIGHT,
    timeline::{Animation, Ease},
    workspaces::WORKSPACE_COUNT,
};

const TRANSITION: Duration = Duration::from_millis(215);
const CONTROLS_HEIGHT: i32 = 116;
pub const CONTROL_SIZE: (i32, i32) = (402, 98);

pub fn controls_bounds(output: Size<i32, Logical>, bar_height: i32) -> Rectangle<i32, Logical> {
    let width = CONTROL_SIZE.0.min(output.w.saturating_sub(24)).max(1);
    Rectangle::new(
        ((output.w - width) / 2, bar_height + 12).into(),
        (width, CONTROL_SIZE.1).into(),
    )
}

pub fn workspace_control(index: usize) -> Rectangle<i32, Logical> {
    Rectangle::new((index as i32 * 136, 42).into(), (130, 56).into())
}

pub fn search_control() -> Rectangle<i32, Logical> {
    Rectangle::new((0, 0).into(), (CONTROL_SIZE.0, 34).into())
}

#[derive(Default)]
pub(super) struct Overview {
    pub open: bool,
    motion: Animation,
    pub(super) visible_cards: Vec<(Window, Rectangle<f64, Logical>)>,
    pub selected: usize,
    pub cards: Vec<Card>,
}

pub struct Card {
    pub window: Window,
    pub thumbnail: Rectangle<f64, Logical>,
    pub title: String,
}

impl Overview {
    pub fn amount(&self, now: Instant) -> f64 {
        self.motion.value(now)
    }

    pub fn hit(&self, position: Point<f64, Logical>) -> Option<usize> {
        let (window, _) = self
            .visible_cards
            .iter()
            .find(|(_, bounds)| bounds.contains(position))?;
        self.cards.iter().position(|card| card.window == *window)
    }

    pub(super) fn settle(&mut self) {
        self.motion.settle();
    }
}

impl State {
    pub(super) fn overview_action(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<crate::shell::status::Action> {
        use crate::{desktop::Tool, shell::status::Action};
        let (bar, _) = self.status_hits?;
        let output = self.spaces[self.workspaces.active].output_geometry(&self.output)?;
        let bounds = controls_bounds(output.size, bar.size.h);
        let local = location - bounds.loc.to_f64();
        let point: Point<f64, Logical> = (
            local.x * f64::from(CONTROL_SIZE.0) / f64::from(bounds.size.w),
            local.y,
        )
            .into();
        if search_control().to_f64().contains(point) {
            return Some(Action::Launch(Tool::Apps));
        }
        (0..WORKSPACE_COUNT)
            .find(|&index| workspace_control(index).to_f64().contains(point))
            .map(Action::Workspace)
    }

    pub fn workspace_window_count(&self, index: usize) -> usize {
        self.spaces[index]
            .elements()
            .filter(|window| !window.geometry().size.is_empty())
            .count()
    }

    pub fn overview_open(&self) -> bool {
        self.overview.open
    }

    pub fn overview_amount(&self, now: Instant) -> f64 {
        self.overview.amount(now)
    }

    pub fn overview_animating(&self) -> bool {
        self.overview.motion.is_animating()
    }

    pub fn overview_cards(&self) -> (&[Card], usize) {
        (&self.overview.cards, self.overview.selected)
    }

    pub(super) fn switch_window(&mut self, delta: isize) {
        if !self.overview.open {
            let focused = self.focused_window();
            self.set_overview(true);
            if let Some(output) = self.spaces[self.workspaces.active].output_geometry(&self.output)
            {
                self.build_overview_cards(output.size, BAR_HEIGHT as i32);
            }
            if let Some(index) = self
                .overview
                .cards
                .iter()
                .position(|card| Some(&card.window) == focused.as_ref())
            {
                self.overview.selected = index;
            } else if delta > 0 {
                self.overview.selected = self.overview.cards.len().saturating_sub(1);
            }
        }
        self.switching_windows = true;
        self.step_overview(delta);
    }

    pub(super) fn toggle_overview(&mut self) {
        self.set_overview(!self.overview.open);
    }

    fn set_overview(&mut self, open: bool) {
        let now = Instant::now();
        let duration = if self.appearance.reduce_motion {
            Duration::ZERO
        } else {
            TRANSITION
        };
        self.overview
            .motion
            .retarget(f64::from(open), now, duration, Ease::Out);
        self.overview.open = open;
        self.switching_windows = false;
        self.status_panel = crate::shell::status::Panel::Closed;
        self.dock_pressed = None;
        self.dragged_window = None;
        self.finish_resize();
        if open {
            self.overview.selected = 0;
            self.focus_window(None);
        } else {
            self.focus_last_window();
        }
        self.redraw = true;
    }

    pub(super) fn step_overview(&mut self, delta: isize) {
        let count = self.overview.cards.len();
        if count > 0 {
            self.overview.selected =
                (self.overview.selected as isize + delta).rem_euclid(count as isize) as usize;
        }
        self.redraw = true;
    }

    pub(super) fn choose_overview(&mut self, index: usize) {
        let window = self
            .overview
            .cards
            .get(index)
            .map(|card| card.window.clone());
        self.set_overview(false);
        if let Some(window) = window
            && self.spaces[self.workspaces.active]
                .element_location(&window)
                .is_some()
            && !window.bbox().size.is_empty()
        {
            self.focus_window(Some(&window));
        }
    }

    pub fn update_overview(&mut self, now: Instant, output: Size<i32, Logical>, bar_height: i32) {
        self.overview.motion.finish_frame(now);
        if self.overview.amount(now) == 0.0 {
            self.overview.cards.clear();
            return;
        }
        self.build_overview_cards(output, bar_height);
    }

    fn build_overview_cards(&mut self, output: Size<i32, Logical>, bar_height: i32) {
        let space = &self.spaces[self.workspaces.active];
        let mut windows: Vec<_> = space
            .elements()
            .rev()
            .filter(|window| !window.geometry().size.is_empty())
            .cloned()
            .collect();
        // Preserve tile order while previews zoom back to their desktop stacking order.
        if !self.overview.cards.is_empty() {
            windows.sort_by_key(|window| {
                self.overview
                    .cards
                    .iter()
                    .position(|card| card.window == *window)
                    .unwrap_or(usize::MAX)
            });
        }
        let count = windows.len();
        self.overview.selected = self.overview.selected.min(count.saturating_sub(1));
        let slots = layout(output, bar_height, self.dock_reserve, count);
        self.overview.cards = windows
            .into_iter()
            .zip(slots)
            .map(|(window, slot)| {
                let size = window.geometry().size.to_f64();
                let scale = (slot.size.w / size.w).min(slot.size.h / size.h).min(0.86);
                let size = Size::from((size.w * scale, size.h * scale));
                let loc = slot.loc
                    + Point::from(((slot.size.w - size.w) / 2.0, (slot.size.h - size.h) / 2.0));
                let title = window.toplevel().map_or_else(String::new, |top| {
                    with_states(top.wl_surface(), |state| {
                        state
                            .data_map
                            .get::<XdgToplevelSurfaceData>()
                            .and_then(|entry| {
                                let entry = entry.lock().ok()?;
                                entry.title.as_ref().or(entry.app_id.as_ref()).map(|text| {
                                    text.chars().filter(|c| !c.is_control()).take(90).collect()
                                })
                            })
                            .unwrap_or_default()
                    })
                });
                Card {
                    window,
                    thumbnail: Rectangle::new(loc, size),
                    title,
                }
            })
            .collect();
    }
}

fn layout(
    output: Size<i32, Logical>,
    bar_height: i32,
    dock_reserve: i32,
    count: usize,
) -> Vec<Rectangle<f64, Logical>> {
    if count == 0 {
        return Vec::new();
    }
    let w = f64::from(output.w.max(1));
    let h = f64::from(output.h.max(1));
    let top = f64::from(bar_height.max(BAR_HEIGHT as i32) + CONTROLS_HEIGHT) + 12.0;
    let usable = (h - top - f64::from(dock_reserve) - 18.0).max(1.0);
    let columns = ((count as f64 * (w / usable / 1.55).clamp(0.5, 2.25))
        .sqrt()
        .ceil() as usize)
        .min(count);
    let rows = count.div_ceil(columns);
    let cell_w = w * 0.91 / columns as f64;
    let cell_h = usable / rows as f64;
    let gap_x = 24.0_f64.min(cell_w * 0.18);
    let gap_y = 34.0_f64.min(cell_h * 0.26);
    (0..count)
        .map(|index| {
            let row = index / columns;
            let column = index % columns;
            let occupied = (count - row * columns).min(columns);
            let x = (w - occupied as f64 * cell_w) / 2.0 + column as f64 * cell_w;
            let y = top + row as f64 * cell_h;
            Rectangle::new(
                (x + gap_x / 2.0, y).into(),
                ((cell_w - gap_x).max(1.0), (cell_h - gap_y).max(1.0)).into(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_fit_between_shell_panels_without_overlapping() {
        let slots = layout((1280, 800).into(), 47, 97, 7);
        assert_eq!(slots.len(), 7);
        for (index, slot) in slots.iter().enumerate() {
            assert!(slot.loc.x > 0.0 && slot.loc.y > 42.0);
            assert!(slot.loc.x + slot.size.w < 1280.0 && slot.loc.y + slot.size.h < 714.0);
            assert!(
                slots[..index]
                    .iter()
                    .all(|other| slot.intersection(*other).is_none())
            );
        }
    }
}
