use std::{io, time::Instant};

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    delegate_seat,
    input::{
        Seat, SeatHandler, SeatState,
        keyboard::{FilterResult, keysyms},
        pointer::{AxisFrame, ButtonEvent, CursorImageStatus, MotionEvent},
    },
    reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface},
    utils::{Logical, Point, SERIAL_COUNTER, Serial},
    wayland::selection::data_device::set_data_device_focus,
};

use crate::shell::{dock::DockAction, launch::App};

use super::State;

pub(super) const BUTTON_LEFT: u32 = 0x110;

#[derive(Clone, Copy)]
pub enum Shortcut {
    Exit,
    SwitchVt(i32),
}

impl State {
    pub fn pointer_location(&self) -> Point<f64, Logical> {
        self.pointer.current_location()
    }

    fn move_pointer(&mut self, location: Point<f64, Logical>, serial: Serial, time: u32) {
        let pointer = self.pointer.clone();
        self.move_dragged_window(location);
        let on_dock = self
            .dock_hits
            .as_ref()
            .is_some_and(|dock| dock.contains(location));
        let surface = if !pointer.is_grabbed()
            && (on_dock || self.status_at(location) || self.dock_pressed.is_some())
        {
            None
        } else {
            self.surface_under(location)
        };
        pointer.motion(
            self,
            surface,
            &MotionEvent {
                location,
                serial,
                time,
            },
        );
        pointer.frame(self);
    }
    pub fn host_focus(&mut self, focused: bool) {
        self.host_focused = focused;
        if focused {
            let keyboard = self.keyboard.clone();
            if keyboard.current_focus().is_none() {
                self.focus_last_window();
            }
        } else {
            self.redraw |= self.dock_pressed.take().is_some();
            self.dragged_window = None;
            let keyboard = self.keyboard.clone();
            keyboard.set_focus(self, None, SERIAL_COUNTER.next_serial());
            // The host can deliver the releases to another window or VT.
            for key in keyboard.pressed_keys() {
                keyboard.input::<(), _>(
                    self,
                    key,
                    KeyState::Released,
                    SERIAL_COUNTER.next_serial(),
                    0,
                    |_, _, _| FilterResult::Forward,
                );
            }
            self.intercepted_keys.clear();
        }
    }

    pub fn input<I: InputBackend>(
        &mut self,
        input: InputEvent<I>,
        open_app: impl FnOnce(App, Instant) -> io::Result<()>,
        shortcut: Option<&mut dyn FnMut(Shortcut)>,
    ) {
        let keyboard = self.keyboard.clone();
        let pointer = self.pointer.clone();
        let serial = SERIAL_COUNTER.next_serial();

        match input {
            InputEvent::Keyboard { event } => {
                if !self.host_focused {
                    return;
                }
                let code = event.key_code();
                let state = event.state();
                let action = keyboard.input(
                    self,
                    code,
                    state,
                    serial,
                    event.time_msec(),
                    |data, modifiers, key| {
                        if state == KeyState::Released && data.intercepted_keys.remove(&code.raw())
                        {
                            return FilterResult::Intercept(None);
                        }
                        if state == KeyState::Pressed
                            && data.status_open
                            && !modifiers.ctrl
                            && !modifiers.alt
                            && key.modified_sym().raw() == keysyms::KEY_Escape
                        {
                            data.status_open = false;
                            data.redraw = true;
                            data.intercepted_keys.insert(code.raw());
                            return FilterResult::Intercept(None);
                        }
                        if shortcut.is_some()
                            && state == KeyState::Pressed
                            && modifiers.ctrl
                            && modifiers.alt
                        {
                            let action = match key.modified_sym().raw() {
                                keysyms::KEY_Escape => Some(Shortcut::Exit),
                                sym @ keysyms::KEY_XF86Switch_VT_1
                                    ..=keysyms::KEY_XF86Switch_VT_12 => Some(Shortcut::SwitchVt(
                                    (sym - keysyms::KEY_XF86Switch_VT_1 + 1) as i32,
                                )),
                                sym @ keysyms::KEY_F1..=keysyms::KEY_F12 => {
                                    Some(Shortcut::SwitchVt((sym - keysyms::KEY_F1 + 1) as i32))
                                }
                                _ => None,
                            };
                            if action.is_some() {
                                data.intercepted_keys.insert(code.raw());
                                return FilterResult::Intercept(action);
                            }
                        }
                        FilterResult::Forward
                    },
                );
                if let (Some(Some(action)), Some(handle)) = (action, shortcut) {
                    handle(action);
                }
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let Some(output) =
                    self.spaces[self.workspaces.active].output_geometry(&self.output)
                else {
                    return;
                };
                let location = event.position_transformed(output.size);
                self.redraw |= self.dock_hits.as_ref().is_some_and(|dock| {
                    dock.action_at(pointer.current_location()) != dock.action_at(location)
                });
                self.move_pointer(location, serial, event.time_msec());
                self.redraw |= shortcut.is_some();
            }
            InputEvent::PointerMotion { event } => {
                let Some(output) =
                    self.spaces[self.workspaces.active].output_geometry(&self.output)
                else {
                    return;
                };
                let location = pointer.current_location() + event.delta();
                let location = Point::from((
                    location.x.clamp(0.0, f64::from(output.size.w - 1)),
                    location.y.clamp(0.0, f64::from(output.size.h - 1)),
                ));
                self.move_pointer(location, serial, event.time_msec());
                self.redraw |= shortcut.is_some();
            }
            InputEvent::PointerButton { event } => {
                let location = pointer.current_location();
                if event.button_code() == BUTTON_LEFT {
                    if event.state() == ButtonState::Released {
                        self.dragged_window = None;
                    }
                    if event.state() == ButtonState::Pressed && !pointer.is_grabbed() {
                        if self.status_toggle_at(location) {
                            self.status_open = !self.status_open;
                            self.redraw = true;
                            self.move_pointer(location, serial, event.time_msec());
                            return;
                        }
                        if self.status_open && !self.status_at(location) {
                            self.status_open = false;
                            self.redraw = true;
                        }
                    }
                    let action = self
                        .dock_hits
                        .as_ref()
                        .and_then(|dock| dock.action_at(location));
                    if event.state() == ButtonState::Pressed
                        && !pointer.is_grabbed()
                        && let Some(action) = action
                    {
                        self.dock_pressed = Some(action);
                        self.redraw = true;
                        return;
                    }
                    if event.state() == ButtonState::Released
                        && let Some(pressed) = self.dock_pressed.take()
                    {
                        self.redraw = true;
                        if action == Some(pressed) {
                            match pressed {
                                DockAction::App(app) => self.start_app(app, open_app),
                                DockAction::Workspace(index) => self.select_workspace(index),
                            }
                        }
                        self.move_pointer(location, serial, event.time_msec());
                        return;
                    }
                }
                if event.state() == ButtonState::Pressed && !pointer.is_grabbed() {
                    let blocked = self
                        .dock_hits
                        .as_ref()
                        .is_some_and(|dock| dock.contains(location));
                    if !blocked
                        && !self.status_at(location)
                        && let Some((window, _)) = self.window_under(location)
                    {
                        self.focus_window(Some(&window.clone()));
                    }
                    self.move_pointer(location, serial, event.time_msec());
                }
                pointer.button(
                    self,
                    &ButtonEvent {
                        button: event.button_code(),
                        state: event.state(),
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event } => {
                let source = event.source();
                let mut axes = AxisFrame::new(event.time_msec()).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let discrete = event.amount_v120(axis);
                    let amount = event
                        .amount(axis)
                        .unwrap_or_else(|| discrete.unwrap_or(0.0) / 8.0);
                    if amount != 0.0 {
                        axes = axes.value(axis, amount);
                        if let Some(discrete) = discrete {
                            axes = axes.v120(axis, discrete as i32);
                        }
                    } else if source == AxisSource::Finger && event.amount(axis) == Some(0.0) {
                        axes = axes.stop(axis);
                    }
                }
                pointer.axis(self, axes);
                pointer.frame(self);
            }
            _ => {}
        }
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, surface: Option<&WlSurface>) {
        set_data_device_focus(&self.display, seat, surface.and_then(Resource::client));
    }

    fn cursor_image(&mut self, _: &Seat<Self>, _: CursorImageStatus) {}
}

delegate_seat!(State);
