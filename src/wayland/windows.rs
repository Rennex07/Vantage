use std::time::{Duration, Instant};

use smithay::{
    backend::renderer::{
        element::{
            AsRenderElements, surface::WaylandSurfaceRenderElement, utils::RescaleRenderElement,
        },
        gles::GlesRenderer,
    },
    delegate_xdg_shell,
    desktop::{PopupKind, Space, Window, WindowSurfaceType, space::SpaceElement},
    reexports::wayland_server::protocol::{
        wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface,
    },
    utils::{Logical, Physical, Point, Rectangle, SERIAL_COUNTER, Scale, Serial, Size},
    wayland::{
        compositor::{get_parent, is_sync_subsurface, with_states},
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
        },
    },
};

use crate::shell::{
    glass::FrostedWindow,
    launch::{App, Target},
    motion::{Opening, Pose},
    status::BAR_HEIGHT,
    timeline::{Animation, Ease},
};

use super::{State, input::BUTTON_LEFT};

#[derive(Default)]
pub struct WindowScene {
    pub elements: Vec<RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>>,
    pub glass: Vec<FrostedWindow>,
}

pub(super) struct WindowDrag {
    window: Window,
    location: Point<i32, Logical>,
    pointer: Point<f64, Logical>,
}

impl State {
    pub(super) fn surface_committed(&mut self, surface: &WlSurface) {
        self.surface_epoch = self.surface_epoch.wrapping_add(1);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            self.update_window(&root);
        }
        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface)
            && !popup.is_initial_configure_sent()
        {
            let _ = popup.send_configure();
        }
        self.redraw = true;
    }

    fn update_window(&mut self, root: &WlSurface) {
        let Some((index, window)) = self.find_window(root) else {
            return;
        };
        let toplevel = window.toplevel().expect("Wayland window");
        let was_empty = window.bbox().size.is_empty();
        window.on_commit();
        let is_empty = window.bbox().size.is_empty();
        if !toplevel.is_initial_configure_sent() && (was_empty || !is_empty) {
            if let Some(output) = self.spaces[index].output_geometry(&self.output) {
                configure_toplevel(toplevel, output.size);
            }
            toplevel.send_configure();
        }
        if was_empty && !is_empty {
            let launch =
                window_app(&window).and_then(|app| self.pending_launches[app.index()].take());
            let desktop_launch = match self.dock_target(&window) {
                Some(Target::Desktop(id)) => self.pending_desktop.remove(&id),
                _ => None,
            };
            let workspace = launch
                .map(|launch| launch.workspace)
                .or_else(|| desktop_launch.map(|(_, workspace)| workspace))
                .unwrap_or(index);
            let Some(output) = self.spaces[workspace].output_geometry(&self.output) else {
                return;
            };
            if workspace != index {
                self.spaces[index].unmap_elem(&window);
            }
            let location = centered_location(&window, output.size);
            self.opening_windows
                .retain(|(opening, _)| opening != &window);
            if !self.appearance.reduce_motion && !geometry::is_fullscreen(&window) {
                self.opening_windows
                    .push((window.clone(), Opening::new(Instant::now(), launch)));
            }
            let active = workspace == self.workspaces.active;
            self.spaces[workspace].map_element(window.clone(), location, active);
            if active && self.host_focused && !self.overview.open && !self.lock_active() {
                self.focus_window(Some(&window));
            }
        } else if !was_empty && is_empty {
            self.minimize_motion.retain(|(other, _)| other != &window);
            self.minimized_windows.retain(|other| other != &window);
            self.forget_window_geometry(&window);
            self.opening_windows.retain(|(other, _)| other != &window);
            if index == self.workspaces.active {
                self.focus_last_window();
            }
            return;
        }
        self.apply_window_geometry(&window);
    }

    pub(super) fn find_window(&self, root: &WlSurface) -> Option<(usize, Window)> {
        self.spaces.iter().enumerate().find_map(|(index, space)| {
            space
                .elements()
                .find(|window| {
                    window
                        .toplevel()
                        .is_some_and(|top| top.wl_surface() == root)
                })
                .cloned()
                .map(|window| (index, window))
        })
    }

    pub(super) fn select_workspace(&mut self, index: usize) {
        if index == self.workspaces.active {
            return;
        }
        self.dragged_window = None;
        self.finish_resize();
        self.focus_window(None);
        self.workspaces
            .select(index, Instant::now(), !self.appearance.reduce_motion);
        self.focus_last_window();
    }

    pub(super) fn focus_window(&mut self, target: Option<&Window>) {
        if let Some(window) = target {
            if self.minimized_windows.contains(window) {
                self.animate_minimize(window, false);
            }
            self.minimized_windows.retain(|other| other != window);
            self.spaces[self.workspaces.active].raise_element(window, true);
        }
        for window in self.spaces[self.workspaces.active].elements() {
            let surface = window.toplevel().expect("Wayland window");
            if target.is_none() {
                window.set_activated(false);
            }
            if surface.is_initial_configure_sent() {
                surface.send_pending_configure();
            }
        }
        let surface = target
            .and_then(Window::toplevel)
            .map(|top| top.wl_surface().clone());
        self.keyboard
            .clone()
            .set_focus(self, surface, SERIAL_COUNTER.next_serial());
        self.redraw = true;
    }

    pub(super) fn focus_last_window(&mut self) {
        if !self.host_focused || self.overview.open || self.lock_active() {
            return;
        }
        let window = self.spaces[self.workspaces.active]
            .elements()
            .rev()
            .find(|window| {
                !window.bbox().size.is_empty() && !self.minimized_windows.contains(window)
            })
            .cloned();
        self.focus_window(window.as_ref());
    }

    pub(super) fn focused_window(&self) -> Option<Window> {
        let surface = self.keyboard.current_focus()?;
        self.find_window(&surface).map(|(_, window)| window)
    }

    pub(super) fn minimize_focused_window(&mut self) {
        if let Some(window) = self.focused_window() {
            self.minimize_window(window);
        }
    }

    fn minimize_window(&mut self, window: Window) {
        if window.bbox().size.is_empty() || self.minimized_windows.contains(&window) {
            return;
        }
        let focused = self.focused_window().as_ref() == Some(&window);
        self.animate_minimize(&window, true);
        self.minimized_windows.push(window);
        if focused {
            self.focus_last_window();
        }
        self.redraw = true;
    }

    fn animate_minimize(&mut self, window: &Window, minimized: bool) {
        if self.appearance.reduce_motion
            || self.overview.open
            || self.lock_active()
            || geometry::is_fullscreen(window)
            || self
                .opening_windows
                .iter()
                .any(|(other, _)| other == window)
        {
            self.minimize_motion.retain(|(other, _)| other != window);
            return;
        }
        let now = Instant::now();
        let target = f64::from(minimized);
        if let Some((_, track)) = self
            .minimize_motion
            .iter_mut()
            .find(|(other, _)| other == window)
        {
            track.retarget(target, now, Duration::from_millis(180), Ease::Out);
        } else {
            let mut track = Animation::default();
            track.retarget(1.0 - target, now, Duration::ZERO, Ease::Out);
            track.retarget(target, now, Duration::from_millis(180), Ease::Out);
            self.minimize_motion.push((window.clone(), track));
        }
    }

    fn minimizing_amount(&self, window: &Window, now: Instant) -> Option<f64> {
        self.minimize_motion
            .iter()
            .find(|(other, _)| other == window)
            .map(|(_, track)| track.value(now))
    }

    pub(super) fn close_focused_window(&self) {
        if let Some(surface) = self.keyboard.current_focus()
            && let Some((_, window)) = self.find_window(&surface)
            && let Some(toplevel) = window.toplevel()
        {
            toplevel.send_close();
        }
    }

    fn dock_target(&self, window: &Window) -> Option<Target> {
        if let Some(app) = window_app(window) {
            return Some(Target::BuiltIn(app));
        }
        with_app_id(window, |id| {
            crate::shell::app_identity::desktop_id(id, &self.installed_desktop_ids)
                .map(Target::Desktop)
        })
        .flatten()
    }

    pub fn running_dock_apps(&self) -> Vec<Target> {
        let mut running = Vec::new();
        for window in self.spaces.iter().flat_map(|space| space.elements()) {
            if !window.bbox().size.is_empty()
                && let Some(target) = self.dock_target(window)
                && !running.contains(&target)
            {
                running.push(target);
            }
        }
        running
    }

    pub fn focused_dock_app(&self) -> Option<Target> {
        self.dock_target(&self.focused_window()?)
    }

    pub(super) fn open_dock_menu(&mut self, point: Point<f64, Logical>) -> bool {
        let Some(dock) = &self.dock_hits else {
            return false;
        };
        let Some(crate::shell::dock::Action::Launch(target)) = dock.action_at(point) else {
            return false;
        };
        let Some(slot) = dock.slot_at(point) else {
            return false;
        };
        let Some(mode) = self.output.current_mode() else {
            return false;
        };
        let target = target.clone();
        let limit = ((mode.size.h - 100) / 34).clamp(0, 8) as usize;
        let windows = self.spaces.iter().enumerate().flat_map(|(index, space)| space.elements().rev().map(move |window| (index, window)))
            .filter(|(_, window)| !window.bbox().size.is_empty() && self.dock_target(window).as_ref() == Some(&target))
            .take(limit)
            .map(|(index, window)| {
                let title: String = window.toplevel().map_or_else(String::new, |top| with_states(top.wl_surface(), |state| {
                    state.data_map.get::<XdgToplevelSurfaceData>().and_then(|entry| {
                        let entry = entry.lock().ok()?;
                        entry.title.as_ref().or(entry.app_id.as_ref()).map(|text| text.chars().filter(|c| !c.is_control() && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')).take(80).collect())
                    }).unwrap_or_default()
                }));
                let minimized = if self.minimized_windows.contains(window) { " (minimized)" } else { "" };
                (window.clone(), format!("{}: {}{}", index + 1, if title.is_empty() { "Window" } else { &title }, minimized))
            }).collect();
        self.dock_menu = crate::shell::dock_menu::Menu::new(
            target,
            windows,
            dock.anchor(slot),
            dock.top(),
            mode.size,
        );
        self.status_panel = crate::shell::status::Panel::Closed;
        self.redraw = true;
        self.dock_menu.is_some()
    }

    pub(super) fn choose_dock_menu(
        &mut self,
        menu: crate::shell::dock_menu::Menu,
        row: usize,
        open: impl FnOnce(Target, Instant) -> std::io::Result<()>,
    ) {
        if let Some((window, _)) = menu.windows.get(row) {
            let workspace = self
                .spaces
                .iter()
                .position(|space| space.element_location(window).is_some());
            if let Some(workspace) = workspace
                && !window.bbox().size.is_empty()
            {
                self.select_workspace(workspace);
                self.focus_window(Some(window));
            }
        } else if row == menu.windows.len() {
            self.focus_dock_window(&menu.target);
        } else if row == menu.windows.len() + 1 {
            if let Err(error) = open(menu.target, Instant::now()) {
                eprintln!("Could not open new window: {error}");
                self.show_notice("Application".into(), "The app could not be started.");
            }
        }
        self.redraw = true;
    }

    pub(super) fn focus_dock_window(&mut self, target: &Target) -> bool {
        let active = self.workspaces.active;
        let candidates: Vec<_> = std::iter::once(active)
            .chain((0..self.spaces.len()).filter(|&index| index != active))
            .flat_map(|index| {
                self.spaces[index]
                    .elements()
                    .rev()
                    .map(move |window| (index, window))
            })
            .filter(|(_, window)| {
                !window.bbox().size.is_empty() && self.dock_target(window).as_ref() == Some(target)
            })
            .map(|(index, window)| (index, window.clone()))
            .collect();
        let focused = self.focused_window();
        let ordered: Vec<_> = candidates
            .iter()
            .map(|(_, window)| window.clone())
            .collect();
        let selected = crate::shell::app_identity::cycle_window(
            &mut self.dock_cycle,
            &ordered,
            focused.as_ref(),
        );
        let found = selected.and_then(|selected| {
            candidates
                .into_iter()
                .find(|(_, window)| *window == selected)
        });
        if let Some((workspace, window)) = found {
            if self.overview.open {
                self.toggle_overview();
            }
            self.select_workspace(workspace);
            self.focus_window(Some(&window));
            true
        } else {
            false
        }
    }

    pub(super) fn window_under(
        &self,
        position: Point<f64, Logical>,
    ) -> Option<(&Window, Point<i32, Logical>)> {
        let now = Instant::now();
        let active = self.workspaces.active;
        let offset = self.workspace_offset(active, self.workspaces.position(now));
        let space = &self.spaces[active];
        let fullscreen = self.fullscreen_window(active);
        space.elements().rev().find_map(|window| {
            if self.minimized_windows.contains(window)
                || fullscreen.is_some_and(|top| top != window)
            {
                return None;
            }
            let target = self.window_target(space, window, offset)?;
            let pose = self.window_pose(window, target, now);
            // Wayland pointer coordinates are translation-only. The client can receive
            // keys immediately and starts receiving pointer input at its full size.
            if pose.rect.size != target.size || pose.opacity < 0.5 {
                return None;
            }
            let origin = pose.rect.loc.to_i32_round() - window.geometry().loc;
            window
                .is_in_input_region(&(position - origin.to_f64()))
                .then_some((window, origin))
        })
    }

    fn fullscreen_window(&self, index: usize) -> Option<&Window> {
        self.spaces[index]
            .elements()
            .rev()
            .find(|window| {
                !self.minimized_windows.contains(window) && !window.bbox().size.is_empty()
            })
            .filter(|window| geometry::is_fullscreen(window))
    }

    pub fn fullscreen_visible(&self, now: Instant) -> bool {
        self.overview.amount(now) == 0.0
            && self.workspaces.position(now) == self.workspaces.active as f64
            && self.fullscreen_window(self.workspaces.active).is_some()
    }

    pub(super) fn surface_under(
        &self,
        position: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let (window, origin) = self.window_under(position)?;
        let (surface, relative_origin) =
            window.surface_under(position - origin.to_f64(), WindowSurfaceType::ALL)?;
        Some((surface, (origin + relative_origin).to_f64()))
    }

    pub fn window_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        position: f64,
        now: Instant,
    ) -> WindowScene {
        self.popups.cleanup();
        let mut windows = WindowScene::default();
        let overview = self.overview.amount(now);
        self.overview.visible_cards.clear();
        for index in 0..self.spaces.len() {
            self.spaces[index].refresh();
            if (index as f64 - position).abs() >= 1.0
                || (overview == 1.0 && index != self.workspaces.active)
            {
                continue;
            }
            let space = &self.spaces[index];
            let offset = self.workspace_offset(index, position);
            let fullscreen = (overview == 0.0)
                .then(|| self.fullscreen_window(index).cloned())
                .flatten();
            for window in space.elements_for_output(&self.output).rev() {
                if fullscreen.as_ref().is_some_and(|top| top != window) {
                    continue;
                }
                let minimized = self.minimized_windows.contains(window);
                let minimizing = overview == 0.0
                    && self
                        .minimizing_amount(window, now)
                        .is_some_and(|amount| amount < 1.0);
                if minimized && !minimizing && (index != self.workspaces.active || overview == 0.0)
                {
                    continue;
                }
                let Some(target) = self.window_target(space, window, offset) else {
                    continue;
                };
                let mut pose = self.window_pose(window, target, now);
                if minimized && overview > 0.0 {
                    pose.opacity *= overview as f32;
                }
                if overview > 0.0
                    && index == self.workspaces.active
                    && let Some(card) = self
                        .overview
                        .cards
                        .iter()
                        .find(|card| card.window == *window)
                {
                    let thumbnail = card.thumbnail;
                    pose.rect = Rectangle::new(
                        (
                            pose.rect.loc.x + (thumbnail.loc.x - pose.rect.loc.x) * overview,
                            pose.rect.loc.y + (thumbnail.loc.y - pose.rect.loc.y) * overview,
                        )
                            .into(),
                        (
                            pose.rect.size.w + (thumbnail.size.w - pose.rect.size.w) * overview,
                            pose.rect.size.h + (thumbnail.size.h - pose.rect.size.h) * overview,
                        )
                            .into(),
                    );
                    if pose.opacity > 0.0 {
                        self.overview
                            .visible_cards
                            .push((window.clone(), pose.rect));
                    }
                }
                if pose.opacity == 0.0 {
                    continue;
                }
                let scale = Scale::from((
                    pose.rect.size.w / target.size.w,
                    pose.rect.size.h / target.size.h,
                ));
                let pivot = pose.rect.loc.to_i32_round();
                let origin = pivot - window.geometry().loc;
                let elements = window.render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                    renderer,
                    (origin.x, origin.y).into(),
                    1.0.into(),
                    pose.opacity,
                );
                windows.elements.extend(elements.into_iter().map(|element| {
                    RescaleRenderElement::from_element(element, (pivot.x, pivot.y).into(), scale)
                }));
                if !geometry::is_fullscreen(window)
                    && let Some(theme) = frost_theme(window, self.appearance.theme)
                {
                    let size = pose.rect.size.to_i32_round();
                    let bounds: Rectangle<i32, Physical> =
                        Rectangle::new((pivot.x, pivot.y).into(), (size.w, size.h).into());
                    windows.glass.push(FrostedWindow {
                        behind: windows.elements.len(),
                        bounds,
                        opacity: pose.opacity,
                        radius: 22.0,
                        theme,
                    });
                }
            }
        }
        windows
    }

    pub fn windows_are_opening(&self) -> bool {
        !self.opening_windows.is_empty() || !self.minimize_motion.is_empty()
    }

    pub fn finish_window_openings(&mut self, now: Instant) {
        self.opening_windows
            .retain(|(_, opening)| opening.active(now));
        self.minimize_motion.retain_mut(|(_, track)| {
            track.finish_frame(now);
            track.is_animating()
        });
    }

    pub fn opening_tiles(
        &self,
        position: f64,
        now: Instant,
    ) -> impl Iterator<Item = (App, Rectangle<f64, Logical>, f32)> + '_ {
        self.spaces
            .iter()
            .enumerate()
            .filter(move |(index, _)| (*index as f64 - position).abs() < 1.0)
            .flat_map(move |(index, space)| {
                let offset = self.workspace_offset(index, position);
                self.opening_windows
                    .iter()
                    .filter_map(move |(window, opening)| {
                        let target = self.window_target(space, window, offset)?;
                        let pose = opening.pose(now, target);
                        pose.card.map(|(app, alpha)| (app, pose.rect, alpha))
                    })
            })
    }

    fn window_target(
        &self,
        space: &Space<Window>,
        window: &Window,
        offset: Point<i32, Logical>,
    ) -> Option<Rectangle<f64, Logical>> {
        let size = window.geometry().size;
        if size.is_empty() {
            return None;
        }
        Some(Rectangle::new(
            (space.element_location(window)? + offset).to_f64(),
            size.to_f64(),
        ))
    }

    fn window_pose(&self, window: &Window, rect: Rectangle<f64, Logical>, now: Instant) -> Pose {
        let mut pose = self
            .opening_windows
            .iter()
            .find(|(opening, _)| opening == window)
            .map_or(
                Pose {
                    rect,
                    opacity: 1.0,
                    card: None,
                },
                |(_, opening)| opening.pose(now, rect),
            );
        if self.overview.amount(now) == 0.0
            && let Some(amount) = self.minimizing_amount(window, now)
        {
            let shrink = 1.0 - 0.18 * amount;
            let size = pose.rect.size.upscale(shrink);
            pose.rect.loc += Point::from((
                (pose.rect.size.w - size.w) / 2.0,
                (pose.rect.size.h - size.h) / 2.0 + 36.0 * amount,
            ));
            pose.rect.size = size;
            pose.opacity *= (1.0 - amount) as f32;
            pose.card = None;
        }
        pose
    }

    fn workspace_offset(&self, index: usize, position: f64) -> Point<i32, Logical> {
        let width = self.output.current_mode().map_or(0, |mode| mode.size.w);
        (
            ((index as f64 - position) * f64::from(width)).round() as i32,
            0,
        )
            .into()
    }

    pub fn frame_presented(&mut self, time: Instant, locked_frame: bool) {
        let stamp = smithay::utils::Clock::<smithay::utils::Monotonic>::new().now();
        if self.lock_active() {
            if locked_frame {
                self.lock_frame_presented();
            }
            return;
        }
        if let smithay::input::pointer::CursorImageStatus::Surface(surface) = &self.cursor {
            smithay::desktop::utils::send_frames_surface_tree(
                surface,
                &self.output,
                stamp,
                Some(Duration::ZERO),
                |_, _| Some(self.output.clone()),
            );
        }
        let position = self.workspaces.position(time);
        for (index, space) in self.spaces.iter().enumerate() {
            if (index as f64 - position).abs() >= 1.0
                && !(self.overview.open && index == self.workspaces.active)
            {
                continue;
            }
            let fullscreen = (self.overview.amount(time) == 0.0)
                .then(|| self.fullscreen_window(index))
                .flatten();
            for window in space.elements_for_output(&self.output) {
                if fullscreen.is_some_and(|top| top != window)
                    || (self.minimized_windows.contains(window)
                        && self.overview.amount(time) == 0.0
                        && !self
                            .minimizing_amount(window, time)
                            .is_some_and(|amount| amount < 1.0))
                {
                    continue;
                }
                window.send_frame(&self.output, stamp, Some(Duration::ZERO), |_, _| {
                    Some(self.output.clone())
                });
            }
        }
    }

    pub(super) fn move_dragged_window(&mut self, pointer: Point<f64, Logical>) {
        let Some(drag) = &self.dragged_window else {
            return;
        };
        let space = &mut self.spaces[self.workspaces.active];
        let Some(output) = space.output_geometry(&self.output) else {
            return;
        };
        let target = drag.location + (pointer - drag.pointer).to_i32_round();
        let top = self
            .status_hits
            .map_or(BAR_HEIGHT as i32, |(bar, _)| bar.size.h);
        let top = (top + 2).min(output.size.h.saturating_sub(1));
        let location = (
            target.x.clamp(
                (80 - drag.window.geometry().size.w).min(0),
                (output.size.w - 80).max(0),
            ),
            target.y.clamp(top, (output.size.h - 52).max(top)),
        );
        space.map_element(drag.window.clone(), location, true);
        self.redraw = true;
    }

    pub(super) fn finish_move(&mut self, pointer: Point<f64, Logical>) {
        let Some(drag) = self.dragged_window.take() else {
            return;
        };
        if let Some((layout, _)) = self.snap_target(pointer) {
            self.place_window(&drag.window, layout);
        }
        self.redraw = true;
    }

    fn snap_target(
        &self,
        pointer: Point<f64, Logical>,
    ) -> Option<(geometry::Layout, Rectangle<i32, Logical>)> {
        let area = self.work_area(self.workspaces.active)?;
        let layout = if pointer.x <= 12.0 {
            geometry::Layout::Left
        } else if pointer.x >= f64::from(area.size.w - 12) {
            geometry::Layout::Right
        } else if pointer.y < f64::from(area.loc.y) {
            geometry::Layout::Maximized
        } else {
            return None;
        };
        Some((layout, layout.bounds(area)))
    }

    pub fn snap_preview(&self) -> Option<Rectangle<i32, Physical>> {
        let drag = self.dragged_window.as_ref()?;
        let (layout, area) = self.snap_target(self.pointer.current_location())?;
        if !geometry::can_place(&drag.window, layout, area) {
            return None;
        }
        Some(area.to_physical(1))
    }

    pub(super) fn center_windows(&mut self) {
        self.dragged_window = None;
        self.finish_resize();
        for space in &mut self.spaces {
            let Some(output) = space.output_geometry(&self.output) else {
                continue;
            };
            let windows = space.elements().cloned().collect::<Vec<_>>();
            for window in windows {
                if geometry::has_maximized_geometry(&window) {
                    continue;
                }
                let location = centered_location(&window, output.size);
                space.map_element(window, location, false);
            }
        }
        self.refresh_maximized_windows();
    }
}

fn frost_theme(window: &Window, theme: crate::palette::Theme) -> Option<crate::palette::Theme> {
    with_app_id(window, |app_id| {
        if App::Files.matches(app_id) {
            // Thunar imports the launch palette. Its backdrop must keep the same contrast.
            window.user_data().insert_if_missing(|| theme);
            window.user_data().get::<crate::palette::Theme>().copied()
        } else {
            matches!(app_id, "org.vantage.Settings" | "org.vantage.Apps").then_some(theme)
        }
    })
    .flatten()
}

fn window_app(window: &Window) -> Option<App> {
    with_app_id(window, |id| {
        App::ALL.into_iter().find(|app| app.matches(id))
    })
    .flatten()
}

fn with_app_id<T>(window: &Window, read: impl FnOnce(&str) -> T) -> Option<T> {
    let surface = window.toplevel()?.wl_surface();
    with_states(surface, |states| {
        let data = states
            .data_map
            .get::<XdgToplevelSurfaceData>()?
            .lock()
            .ok()?;
        let id = data.app_id.as_deref()?;
        Some(read(id))
    })
}

fn centered_location(window: &Window, output: Size<i32, Logical>) -> Point<i32, Logical> {
    let size = window.geometry().size;
    (
        ((output.w - size.w) / 2).max(0),
        ((output.h - size.h - 58) / 2).max(BAR_HEIGHT as i32 + 6),
    )
        .into()
}

fn configure_toplevel(surface: &ToplevelSurface, size: Size<i32, Logical>) {
    surface.with_pending_state(|state| {
        state.capabilities.replace([
            geometry::WmCapabilities::Maximize,
            geometry::WmCapabilities::Minimize,
            geometry::WmCapabilities::Fullscreen,
        ]);
        state.bounds = Some(((size.w - 24).max(1), (size.h - 130).max(1)).into());
    });
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let space = &mut self.spaces[self.workspaces.active];
        if let Some(output) = space.output_geometry(&self.output) {
            configure_toplevel(&surface, output.size);
        }
        space.map_element(Window::new_wayland_window(surface), (0, 0), false);
    }

    fn move_request(&mut self, surface: ToplevelSurface, _: WlSeat, serial: Serial) {
        let Some(pointer) = self.drag_origin(&surface, serial) else {
            return;
        };
        let Some((index, window)) = self.find_window(surface.wl_surface()) else {
            return;
        };
        if index == self.workspaces.active
            && !geometry::fullscreen_pending(&window)
            && let Some(mut location) = self.spaces[index].element_location(&window)
        {
            if let Some(restored) = self.restore_for_drag(&window) {
                let width = f64::from(window.geometry().size.w.max(1));
                let fraction = ((pointer.x - f64::from(location.x)) / width).clamp(0.0, 1.0);
                location.x = (pointer.x - fraction * f64::from(restored.w)).round() as i32;
            }
            self.dragged_window = Some(WindowDrag {
                window,
                location,
                pointer,
            });
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        _: WlSeat,
        serial: Serial,
        edge: geometry::ResizeEdge,
    ) {
        self.start_resize(surface, serial, edge);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.set_maximized(&surface, true);
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.set_maximized(&surface, false);
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, output: Option<WlOutput>) {
        self.set_fullscreen(&surface, true, output);
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.set_fullscreen(&surface, false, None);
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some((_, window)) = self.find_window(surface.wl_surface()) {
            self.minimize_window(window);
        }
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        if let Some((_, window)) = self.find_window(surface.wl_surface())
            && !window.bbox().size.is_empty()
            && let Some(Target::Desktop(id)) = self.dock_target(&window)
        {
            self.pending_desktop.remove(&id);
        }
        self.redraw = true;
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some((index, window)) = self.find_window(surface.wl_surface()) {
            let focused = self.keyboard.current_focus().as_ref() == Some(surface.wl_surface());
            self.spaces[index].unmap_elem(&window);
            self.dock_cycle.retain(|other| other != &window);
            self.minimize_motion.retain(|(other, _)| other != &window);
            self.minimized_windows.retain(|other| other != &window);
            self.forget_window_geometry(&window);
            self.opening_windows
                .retain(|(opening, _)| opening != &window);
            if focused {
                self.focus_last_window();
            }
        }
        self.redraw = true;
    }

    fn new_popup(&mut self, surface: PopupSurface, _: PositionerState) {
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn popup_destroyed(&mut self, _: PopupSurface) {
        self.redraw = true;
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.positioner = positioner;
            state.geometry = positioner.get_geometry();
        });
        surface.send_repositioned(token);
        self.redraw = true;
    }

    fn grab(&mut self, _: PopupSurface, _: WlSeat, _: Serial) {}
}

delegate_xdg_shell!(State);
mod fullscreen;
mod geometry;
