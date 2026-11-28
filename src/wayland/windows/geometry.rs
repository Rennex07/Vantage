use std::cell::RefCell;

use smithay::{
    desktop::Window,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    reexports::wayland_server::{Resource, protocol::wl_output::WlOutput},
    utils::{Logical, Point, Rectangle, Serial, Size},
    wayland::{
        compositor::with_states,
        shell::xdg::{SurfaceCachedState, ToplevelSurface, XdgToplevelSurfaceData},
    },
};

use super::{BUTTON_LEFT, State, fullscreen::Fullscreen};
use crate::shell::status::BAR_HEIGHT;

pub(super) use xdg_toplevel::{ResizeEdge, WmCapabilities};

#[derive(Clone, Copy)]
struct Resize {
    original: Rectangle<i32, Logical>,
    pointer: Point<f64, Logical>,
    direction: (i32, i32),
    finishing: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::wayland) enum Layout {
    #[default]
    Floating,
    Maximized,
    Left,
    Right,
}

impl Layout {
    pub(super) fn bounds(self, mut area: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        let half = area.size.w / 2;
        match self {
            Self::Left => area.size.w = half.max(1),
            Self::Right => {
                area.loc.x += half;
                area.size.w -= half;
            }
            _ => (),
        }
        area
    }
}

#[derive(Default)]
struct Geometry {
    layout: Layout,
    restore: Option<Rectangle<i32, Logical>>,
    restore_pending: bool,
    restore_in_place: bool,
    resize: Option<Resize>,
    fullscreen: Fullscreen<Rectangle<i32, Logical>, Serial>,
}

fn geometry(window: &Window) -> &RefCell<Geometry> {
    window
        .user_data()
        .insert_if_missing(RefCell::<Geometry>::default);
    window
        .user_data()
        .get()
        .expect("window geometry was initialized")
}

pub(super) fn has_maximized_geometry(window: &Window) -> bool {
    let placement = geometry(window).borrow();
    placement.restore.is_some() || placement.fullscreen.saved.is_some()
}

pub(super) fn fullscreen_pending(window: &Window) -> bool {
    geometry(window).borrow().fullscreen.saved.is_some() || is_fullscreen(window)
}

pub(super) fn is_fullscreen(window: &Window) -> bool {
    window.toplevel().is_some_and(|top| {
        top.current_state()
            .states
            .contains(xdg_toplevel::State::Fullscreen)
    })
}

pub(super) fn can_place(window: &Window, layout: Layout, area: Rectangle<i32, Logical>) -> bool {
    if !matches!(layout, Layout::Left | Layout::Right) {
        return true;
    }
    let top = window.toplevel().expect("Wayland window");
    let min = with_states(top.wl_surface(), |state| {
        state
            .cached_state
            .get::<SurfaceCachedState>()
            .current()
            .min_size
    });
    min.w <= area.size.w && min.h <= area.size.h
}

impl State {
    pub(super) fn set_fullscreen(
        &mut self,
        top: &ToplevelSurface,
        fullscreen: bool,
        output: Option<WlOutput>,
    ) {
        let Some((index, window)) = self.find_window(top.wl_surface()) else {
            return;
        };
        let Some(area) = self.spaces[index].output_geometry(&self.output) else {
            return;
        };
        let Some(location) = self.spaces[index].element_location(&window) else {
            return;
        };
        if self.resizing_window.as_ref() == Some(&window) {
            self.finish_resize();
        }
        if self
            .dragged_window
            .as_ref()
            .is_some_and(|drag| drag.window == window)
        {
            self.dragged_window = None;
        }
        let mut placement = geometry(&window).borrow_mut();
        let layout = placement.layout;
        if fullscreen {
            let current = Rectangle::new(location, window.geometry().size);
            // A floating restore may still be waiting on its own configure.
            let saved = if placement.restore_pending {
                placement.restore.unwrap_or(current)
            } else {
                current
            };
            placement.fullscreen.enter(saved);
            placement.resize = None;
        } else {
            placement.fullscreen.requested = false;
        }
        let saved = placement.fullscreen.saved;
        use xdg_toplevel::State as Xdg;
        top.with_pending_state(|state| {
            if fullscreen {
                state.states.set(Xdg::Fullscreen);
                state.size = Some(area.size);
                state.fullscreen_output = output.filter(|output| self.output.owns(output));
            } else if state.states.unset(Xdg::Fullscreen) {
                state.fullscreen_output = None;
                state.size = if layout == Layout::Floating {
                    saved
                        .filter(|rect| !rect.size.is_empty())
                        .map(|rect| rect.size)
                } else {
                    self.work_area(index).map(|area| layout.bounds(area).size)
                };
            }
        });
        if top.is_initial_configure_sent() {
            let serial = top.send_configure();
            if !fullscreen && saved.is_some() {
                placement.fullscreen.exit = Some(serial);
            }
        } else if !fullscreen {
            placement.fullscreen = Fullscreen::default();
        }
        self.opening_windows
            .retain(|(opening, _)| opening != &window);
        self.redraw = true;
    }

    pub(super) fn restore_for_drag(&mut self, window: &Window) -> Option<Size<i32, Logical>> {
        let placement = geometry(window).borrow();
        if placement.layout == Layout::Floating {
            return None;
        }
        let size = placement.restore?.size;
        drop(placement);
        self.place_window(window, Layout::Floating);
        geometry(window).borrow_mut().restore_in_place = true;
        Some(size)
    }

    pub(super) fn drag_origin(
        &self,
        surface: &ToplevelSurface,
        serial: Serial,
    ) -> Option<Point<f64, Logical>> {
        let grab = self.pointer.grab_start_data()?;
        (self.pointer.has_grab(serial)
            && grab.button == BUTTON_LEFT
            && grab
                .focus
                .is_some_and(|(focus, _)| focus.id().same_client_as(&surface.wl_surface().id())))
        .then_some(grab.location)
    }

    pub(super) fn start_resize(
        &mut self,
        surface: ToplevelSurface,
        serial: Serial,
        edge: ResizeEdge,
    ) {
        let Some(pointer) = self.drag_origin(&surface, serial) else {
            return;
        };
        let Some((index, window)) = self.find_window(surface.wl_surface()) else {
            return;
        };
        if index != self.workspaces.active || has_maximized_geometry(&window) || self.overview.open
        {
            return;
        }
        let Some(location) = self.spaces[index].element_location(&window) else {
            return;
        };
        let direction = match edge {
            ResizeEdge::Top => (0, -1),
            ResizeEdge::Bottom => (0, 1),
            ResizeEdge::Left => (-1, 0),
            ResizeEdge::Right => (1, 0),
            ResizeEdge::TopLeft => (-1, -1),
            ResizeEdge::TopRight => (1, -1),
            ResizeEdge::BottomLeft => (-1, 1),
            ResizeEdge::BottomRight => (1, 1),
            _ => return,
        };
        self.finish_resize();
        self.dragged_window = None;
        geometry(&window).borrow_mut().resize = Some(Resize {
            original: Rectangle::new(location, window.geometry().size),
            pointer,
            direction,
            finishing: false,
        });
        self.resizing_window = Some(window);
    }

    pub(in crate::wayland) fn resize_dragged_window(&self, pointer: Point<f64, Logical>) {
        let Some(window) = &self.resizing_window else {
            return;
        };
        let Some(resize) = geometry(window).borrow().resize else {
            return;
        };
        let top = window.toplevel().expect("Wayland window");
        let (min, max) = with_states(top.wl_surface(), |state| {
            let mut guard = state.cached_state.get::<SurfaceCachedState>();
            let cached = guard.current();
            (cached.min_size, cached.max_size)
        });
        let delta = (pointer - resize.pointer).to_i32_round::<i32>();
        let original = resize.original.size;
        let size: Size<i32, Logical> = (
            bounded(original.w + delta.x * resize.direction.0, min.w, max.w),
            bounded(original.h + delta.y * resize.direction.1, min.h, max.h),
        )
            .into();
        top.with_pending_state(|state| {
            state.size = Some(size);
            state.states.set(xdg_toplevel::State::Resizing);
        });
        top.send_pending_configure();
    }

    pub(in crate::wayland) fn finish_resize(&mut self) {
        let Some(window) = self.resizing_window.take() else {
            return;
        };
        let top = window.toplevel().expect("Wayland window");
        top.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Resizing);
        });
        let mut placement = geometry(&window).borrow_mut();
        if top.send_pending_configure().is_some()
            && let Some(resize) = &mut placement.resize
        {
            resize.finishing = true;
        } else {
            placement.resize = None;
        }
    }

    pub(in crate::wayland) fn toggle_maximized_window(&mut self) {
        self.toggle_window_layout(Layout::Maximized);
    }

    pub(in crate::wayland) fn snap_focused_window(&mut self, right: bool) {
        self.toggle_window_layout(if right { Layout::Right } else { Layout::Left });
    }

    fn toggle_window_layout(&mut self, layout: Layout) {
        if let Some(window) = self.focused_window() {
            let current = geometry(&window).borrow().layout;
            self.place_window(
                &window,
                if current == layout {
                    Layout::Floating
                } else {
                    layout
                },
            );
        }
    }

    pub(super) fn set_maximized(&mut self, top: &ToplevelSurface, maximize: bool) {
        if let Some((_, window)) = self.find_window(top.wl_surface()) {
            self.place_window(
                &window,
                if maximize {
                    Layout::Maximized
                } else {
                    Layout::Floating
                },
            );
        }
    }

    pub(super) fn place_window(&mut self, window: &Window, layout: Layout) {
        let top = window.toplevel().expect("Wayland window");
        if fullscreen_pending(window) {
            if top.is_initial_configure_sent() {
                top.send_configure();
            }
            return;
        }
        let Some((index, _)) = self.find_window(top.wl_surface()) else {
            return;
        };
        let Some(location) = self.spaces[index].element_location(window) else {
            return;
        };
        let Some(area) = self.work_area(index).map(|area| layout.bounds(area)) else {
            return;
        };
        if !can_place(window, layout, area) {
            return;
        }
        if self.resizing_window.as_ref() == Some(window) {
            self.finish_resize();
        }
        let mut placement = geometry(window).borrow_mut();
        if placement.layout == layout {
            return;
        }
        placement.layout = layout;
        placement.restore_in_place = false;
        let current = Rectangle::new(location, window.geometry().size);
        placement.resize = None;
        if layout == Layout::Floating {
            placement.restore_pending = placement.restore.is_some();
        } else {
            placement.restore.get_or_insert(current);
            placement.restore_pending = false;
        }
        use xdg_toplevel::State as Xdg;
        top.with_pending_state(|state| {
            for flag in [
                Xdg::Maximized,
                Xdg::TiledLeft,
                Xdg::TiledRight,
                Xdg::TiledTop,
                Xdg::TiledBottom,
            ] {
                state.states.unset(flag);
            }
            match layout {
                Layout::Maximized => {
                    state.states.set(Xdg::Maximized);
                }
                Layout::Left | Layout::Right => {
                    state.states.set(Xdg::TiledTop);
                    state.states.set(Xdg::TiledBottom);
                    state.states.set(if layout == Layout::Left {
                        Xdg::TiledLeft
                    } else {
                        Xdg::TiledRight
                    });
                }
                Layout::Floating => (),
            }
            state.size = if layout == Layout::Floating {
                placement
                    .restore
                    .filter(|rect| !rect.size.is_empty())
                    .map(|rect| rect.size)
            } else {
                Some(area.size)
            };
        });
        if top.is_initial_configure_sent() {
            top.send_pending_configure();
        }
    }

    pub(super) fn apply_window_geometry(&mut self, window: &Window) {
        let Some(top) = window.toplevel() else {
            return;
        };
        let Some((index, _)) = self.find_window(top.wl_surface()) else {
            return;
        };
        let mut placement = geometry(window).borrow_mut();
        let state = top.current_state();
        use xdg_toplevel::State as Xdg;
        let committed = if state.states.contains(Xdg::Maximized) {
            Layout::Maximized
        } else if state.states.contains(Xdg::TiledLeft) {
            Layout::Left
        } else if state.states.contains(Xdg::TiledRight) {
            Layout::Right
        } else {
            Layout::Floating
        };
        let location = if state.states.contains(Xdg::Fullscreen) {
            self.spaces[index]
                .output_geometry(&self.output)
                .map(|area| {
                    let size = window.geometry().size;
                    area.loc
                        + Point::from((
                            (area.size.w - size.w).max(0) / 2,
                            (area.size.h - size.h).max(0) / 2,
                        ))
                })
        } else if let Some(saved) =
            placement
                .fullscreen
                .restore(with_states(top.wl_surface(), |states| {
                    states
                        .data_map
                        .get::<XdgToplevelSurfaceData>()?
                        .lock()
                        .ok()?
                        .current_serial
                }))
        {
            if committed == Layout::Floating {
                if placement.restore_pending {
                    placement.restore = None;
                    placement.restore_pending = false;
                    placement.restore_in_place = false;
                }
                if saved.size.is_empty() {
                    self.spaces[index]
                        .output_geometry(&self.output)
                        .map(|area| super::centered_location(window, area.size))
                } else {
                    Some(clamp_location(saved, self.work_area(index)))
                }
            } else {
                self.work_area(index).map(|area| committed.bounds(area).loc)
            }
        } else if committed != Layout::Floating {
            if placement.restore_in_place {
                None
            } else {
                self.work_area(index).map(|area| committed.bounds(area).loc)
            }
        } else if placement.restore_pending && state.size == placement.restore.map(|rect| rect.size)
        {
            placement.restore_pending = false;
            let restore_in_place = std::mem::take(&mut placement.restore_in_place);
            placement
                .restore
                .take()
                .filter(|rect| !restore_in_place && !rect.size.is_empty())
                .map(|rect| clamp_location(rect, self.work_area(index)))
        } else if let Some(resize) = placement.resize {
            let size = window.geometry().size;
            let mut point = resize.original.loc;
            if resize.direction.0 < 0 {
                point.x += resize.original.size.w - size.w;
            }
            if resize.direction.1 < 0 {
                point.y += resize.original.size.h - size.h;
            }
            if resize.finishing && !state.states.contains(xdg_toplevel::State::Resizing) {
                placement.resize = None;
            }
            Some(point)
        } else {
            None
        };
        if let Some(location) = location
            && self.spaces[index].element_location(window) != Some(location)
        {
            let space = &mut self.spaces[index];
            let above: Vec<_> = space
                .elements()
                .skip_while(|other| *other != window)
                .skip(1)
                .cloned()
                .collect();
            space.map_element(window.clone(), location, false);
            for other in above {
                space.raise_element(&other, false);
            }
        }
    }

    pub(super) fn forget_window_geometry(&mut self, window: &Window) {
        if self.resizing_window.as_ref() == Some(window) {
            self.resizing_window = None;
        }
        if self
            .dragged_window
            .as_ref()
            .is_some_and(|drag| &drag.window == window)
        {
            self.dragged_window = None;
        }
        *geometry(window).borrow_mut() = Geometry::default();
    }

    pub(super) fn refresh_maximized_windows(&mut self) {
        for index in 0..self.spaces.len() {
            let Some(area) = self.work_area(index) else {
                continue;
            };
            for window in self.spaces[index].elements() {
                let placement = geometry(window).borrow();
                if (placement.fullscreen.requested || placement.layout != Layout::Floating)
                    && let Some(top) = window.toplevel()
                {
                    top.with_pending_state(|state| {
                        state.size = if placement.fullscreen.requested {
                            self.spaces[index]
                                .output_geometry(&self.output)
                                .map(|output| output.size)
                        } else {
                            Some(placement.layout.bounds(area).size)
                        };
                    });
                    if top.is_initial_configure_sent() {
                        top.send_pending_configure();
                    }
                }
            }
        }
    }

    pub(super) fn work_area(&self, index: usize) -> Option<Rectangle<i32, Logical>> {
        let output = self.spaces[index].output_geometry(&self.output)?;
        let top = self
            .bar_height
            .unwrap_or(BAR_HEIGHT as i32)
            .min(output.size.h - 1);
        let height = (output.size.h - top - self.dock_reserve).max(1);
        Some(Rectangle::new(
            (0, top).into(),
            (output.size.w, height).into(),
        ))
    }
}

fn clamp_location(
    rect: Rectangle<i32, Logical>,
    area: Option<Rectangle<i32, Logical>>,
) -> Point<i32, Logical> {
    let Some(area) = area else { return rect.loc };
    let max_x = area.loc.x + (area.size.w - rect.size.w).max(0);
    let max_y = area.loc.y + (area.size.h - rect.size.h).max(0);
    (
        rect.loc.x.clamp(area.loc.x, max_x),
        rect.loc.y.clamp(area.loc.y, max_y),
    )
        .into()
}

fn bounded(value: i32, min: i32, max: i32) -> i32 {
    let min = min.max(1);
    value.clamp(min, if max == 0 { i32::MAX } else { max.max(min) })
}
