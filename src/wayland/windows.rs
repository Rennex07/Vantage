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
    reexports::wayland_server::{
        Resource,
        protocol::{wl_seat::WlSeat, wl_surface::WlSurface},
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
    launch::App,
    motion::{Opening, Pose},
    status::BAR_HEIGHT,
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
        if !toplevel.is_initial_configure_sent() {
            toplevel.send_configure();
        }
        let was_empty = window.bbox().size.is_empty();
        window.on_commit();
        if was_empty && !window.bbox().size.is_empty() {
            let launch =
                window_app(&window).and_then(|app| self.pending_launches[app.index()].take());
            let workspace = launch.map_or(index, |launch| launch.workspace);
            let Some(output) = self.spaces[workspace].output_geometry(&self.output) else {
                return;
            };
            if workspace != index {
                self.spaces[index].unmap_elem(&window);
            }
            let location = centered_location(&window, output.size);
            self.opening_windows
                .retain(|(opening, _)| opening != &window);
            self.opening_windows
                .push((window.clone(), Opening::new(Instant::now(), launch)));
            let active = workspace == self.workspaces.active;
            self.spaces[workspace].map_element(window.clone(), location, active);
            if active && self.host_focused {
                self.focus_window(Some(&window));
            }
        } else if index == self.workspaces.active && !was_empty && window.bbox().size.is_empty() {
            self.focus_last_window();
        }
    }

    fn find_window(&self, root: &WlSurface) -> Option<(usize, Window)> {
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
        self.focus_window(None);
        self.workspaces.select(index, Instant::now());
        self.focus_last_window();
    }

    pub(super) fn focus_window(&mut self, target: Option<&Window>) {
        if let Some(window) = target {
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
        if !self.host_focused {
            return;
        }
        let window = self.spaces[self.workspaces.active]
            .elements()
            .rev()
            .find(|window| !window.bbox().size.is_empty())
            .cloned();
        self.focus_window(window.as_ref());
    }

    pub(super) fn window_under(
        &self,
        position: Point<f64, Logical>,
    ) -> Option<(&Window, Point<i32, Logical>)> {
        let now = Instant::now();
        let active = self.workspaces.active;
        let offset = self.workspace_offset(active, self.workspaces.position(now));
        let space = &self.spaces[active];
        space.elements().rev().find_map(|window| {
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
        for index in 0..self.spaces.len() {
            self.spaces[index].refresh();
            if (index as f64 - position).abs() >= 1.0 {
                continue;
            }
            let space = &self.spaces[index];
            let offset = self.workspace_offset(index, position);
            for window in space.elements_for_output(&self.output).rev() {
                let Some(target) = self.window_target(space, window, offset) else {
                    continue;
                };
                let pose = self.window_pose(window, target, now);
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
                if window_app(window) == Some(App::Files) {
                    let size = pose.rect.size.to_i32_round();
                    let bounds: Rectangle<i32, Physical> =
                        Rectangle::new((pivot.x, pivot.y).into(), (size.w, size.h).into());
                    windows.glass.push(FrostedWindow {
                        behind: windows.elements.len(),
                        bounds,
                        opacity: pose.opacity,
                    });
                }
            }
        }
        windows
    }

    pub fn windows_are_opening(&self) -> bool {
        !self.opening_windows.is_empty()
    }

    pub fn finish_window_openings(&mut self, now: Instant) {
        self.opening_windows
            .retain(|(_, opening)| opening.active(now));
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
        self.opening_windows
            .iter()
            .find(|(opening, _)| opening == window)
            .map_or(
                Pose {
                    rect,
                    opacity: 1.0,
                    card: None,
                },
                |(_, opening)| opening.pose(now, rect),
            )
    }

    fn workspace_offset(&self, index: usize, position: f64) -> Point<i32, Logical> {
        let width = self.output.current_mode().map_or(0, |mode| mode.size.w);
        (
            ((index as f64 - position) * f64::from(width)).round() as i32,
            0,
        )
            .into()
    }

    pub fn frame_presented(&self, time: Instant) {
        let position = self.workspaces.position(time);
        for (index, space) in self.spaces.iter().enumerate() {
            if (index as f64 - position).abs() >= 1.0 {
                continue;
            }
            for window in space.elements_for_output(&self.output) {
                window.send_frame(
                    &self.output,
                    time.duration_since(self.started),
                    Some(Duration::ZERO),
                    |_, _| Some(self.output.clone()),
                );
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
        let top = (BAR_HEIGHT as i32 + 2).min(output.size.h.saturating_sub(1));
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

    pub(super) fn center_windows(&mut self) {
        self.dragged_window = None;
        for space in &mut self.spaces {
            let Some(output) = space.output_geometry(&self.output) else {
                continue;
            };
            let windows = space.elements().cloned().collect::<Vec<_>>();
            for window in windows {
                let location = centered_location(&window, output.size);
                space.map_element(window, location, false);
            }
        }
    }
}

fn window_app(window: &Window) -> Option<App> {
    let surface = window.toplevel()?.wl_surface();
    with_states(surface, |states| {
        let data = states
            .data_map
            .get::<XdgToplevelSurfaceData>()?
            .lock()
            .ok()?;
        let id = data.app_id.as_deref()?;
        App::ALL.into_iter().find(|app| app.matches(id))
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

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let space = &mut self.spaces[self.workspaces.active];
        if let Some(output) = space.output_geometry(&self.output) {
            surface.with_pending_state(|state| {
                state.bounds =
                    Some(((output.size.w - 24).max(1), (output.size.h - 130).max(1)).into());
            });
        }
        space.map_element(Window::new_wayland_window(surface), (0, 0), false);
    }

    fn move_request(&mut self, surface: ToplevelSurface, _: WlSeat, serial: Serial) {
        if !self.pointer.has_grab(serial) {
            return;
        }
        let Some(grab) = self.pointer.grab_start_data() else {
            return;
        };
        if grab.button != BUTTON_LEFT
            || !grab
                .focus
                .is_some_and(|(focus, _)| focus.id().same_client_as(&surface.wl_surface().id()))
        {
            return;
        }
        let Some((index, window)) = self.find_window(surface.wl_surface()) else {
            return;
        };
        if index == self.workspaces.active
            && let Some(location) = self.spaces[index].element_location(&window)
        {
            self.dragged_window = Some(WindowDrag {
                window,
                location,
                pointer: grab.location,
            });
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some((index, window)) = self.find_window(surface.wl_surface()) {
            let focused = self.keyboard.current_focus().as_ref() == Some(surface.wl_surface());
            self.spaces[index].unmap_elem(&window);
            if self
                .dragged_window
                .as_ref()
                .is_some_and(|drag| drag.window == window)
            {
                self.dragged_window = None;
            }
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
