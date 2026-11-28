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

use crate::{
    desktop::Tool,
    shell::{
        dock::Action as DockAction,
        launch::Target,
        status::{Action, Panel},
    },
};

use super::State;

pub(super) const BUTTON_LEFT: u32 = 0x110;

#[derive(Clone, Copy)]
pub enum Shortcut {
    SwitchVt(i32),
}

impl State {
    pub fn pointer_location(&self) -> Point<f64, Logical> {
        self.pointer.current_location()
    }

    pub fn cursor(&self) -> &CursorImageStatus {
        &self.cursor
    }

    fn move_pointer(&mut self, location: Point<f64, Logical>, serial: Serial, time: u32) {
        let pointer = self.pointer.clone();
        if self.lock_active() {
            pointer.motion(
                self,
                None,
                &MotionEvent {
                    location,
                    serial,
                    time,
                },
            );
            pointer.frame(self);
            self.lock_pointer_moved();
            return;
        }
        self.redraw |= self
            .dock_hits
            .as_ref()
            .is_some_and(|dock| dock.slot_at(pointer.current_location()) != dock.slot_at(location));
        self.move_dragged_window(location);
        self.resize_dragged_window(location);
        let hovered = self.status_action_at(location);
        self.redraw |= self.status_hovered != hovered;
        self.status_hovered = hovered;
        let on_dock = self
            .dock_hits
            .as_ref()
            .is_some_and(|dock| dock.contains(location));
        let surface = if self.overview.open
            || (!pointer.is_grabbed()
                && (on_dock || self.status_at(location) || self.dock_pressed.is_some()))
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
            self.status_pressed = None;
            if self.switching_windows {
                self.toggle_overview();
            }
            self.dragged_window = None;
            self.finish_resize();
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
            self.cancel_lock_pointer_wake();
        }
    }

    pub fn input<I: InputBackend>(
        &mut self,
        input: InputEvent<I>,
        launchers: (
            impl FnOnce(Target, Instant) -> io::Result<()>,
            impl Fn(Tool) -> io::Result<()>,
        ),
        shortcut: Option<&mut dyn FnMut(Shortcut)>,
    ) {
        let keyboard = self.keyboard.clone();
        let pointer = self.pointer.clone();
        let serial = SERIAL_COUNTER.next_serial();
        let (open_app, open_tool) = launchers;

        if self.lock_active()
            && !matches!(
                input,
                InputEvent::PointerMotion { .. } | InputEvent::PointerMotionAbsolute { .. }
            )
        {
            if let InputEvent::PointerButton { ref event } = input
                && self.host_focused
                && event.button_code() == BUTTON_LEFT
                && event.state() == ButtonState::Pressed
            {
                self.lock_click();
            }
            if let InputEvent::Keyboard { ref event } = input
                && self.host_focused
            {
                let state = event.state();
                let code = event.key_code();
                let key = keyboard.input(
                    self,
                    code,
                    state,
                    serial,
                    event.time_msec(),
                    |data, modifiers, key| {
                        let new_press = if state == KeyState::Pressed {
                            data.intercepted_keys.insert(code.raw())
                        } else {
                            data.intercepted_keys.remove(&code.raw());
                            false
                        };
                        let symbol = key.modified_sym();
                        let character = if modifiers.ctrl || modifiers.logo {
                            None
                        } else {
                            symbol.key_char()
                        };
                        FilterResult::Intercept(new_press.then_some((symbol.raw(), character)))
                    },
                );
                if let Some(Some((symbol, character))) = key {
                    self.lock_key(symbol, character);
                }
            }
            return;
        }

        match input {
            InputEvent::Keyboard { event } => {
                if !self.host_focused {
                    return;
                }
                let code = event.key_code();
                let state = event.state();
                let mut request_lock = false;
                let action = keyboard.input(
                    self,
                    code,
                    state,
                    serial,
                    event.time_msec(),
                    |data, modifiers, key| {
                        if state == KeyState::Released && data.switching_windows && !modifiers.alt {
                            data.choose_overview(data.overview.selected);
                        }
                        if state == KeyState::Released && data.intercepted_keys.remove(&code.raw())
                        {
                            return FilterResult::Intercept(None);
                        }
                        if state == KeyState::Pressed {
                            let symbol = key.modified_sym().raw();
                            let overview_shortcut = modifiers.logo
                                && !modifiers.ctrl
                                && !modifiers.alt
                                && symbol == keysyms::KEY_Tab;
                            let handled = if modifiers.alt
                                && !modifiers.logo
                                && !modifiers.ctrl
                                && matches!(symbol, keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab)
                            {
                                data.switch_window(if modifiers.shift { -1 } else { 1 });
                                true
                            } else if data.native_session
                                && !modifiers.ctrl
                                && !modifiers.alt
                                && ((modifiers.logo && symbol == keysyms::KEY_l)
                                    || (!modifiers.logo
                                        && !modifiers.shift
                                        && matches!(
                                            symbol,
                                            keysyms::KEY_F9 | keysyms::KEY_XF86ScreenSaver
                                        )))
                            {
                                request_lock = true;
                                true
                            } else if overview_shortcut {
                                data.toggle_overview();
                                true
                            } else if data.notifications_available
                                && modifiers.logo
                                && !modifiers.ctrl
                                && !modifiers.alt
                                && symbol == keysyms::KEY_n
                            {
                                data.apply_status_action(Action::Notifications, &open_tool);
                                true
                            } else if modifiers.logo
                                && !modifiers.ctrl
                                && !modifiers.alt
                                && matches!(symbol, keysyms::KEY_s | keysyms::KEY_space)
                            {
                                data.apply_status_action(Action::Launch(Tool::Apps), &open_tool);
                                true
                            } else if data.overview.open && !(modifiers.ctrl && modifiers.alt) {
                                match symbol {
                                    keysyms::KEY_Escape => data.toggle_overview(),
                                    keysyms::KEY_Return | keysyms::KEY_KP_Enter => {
                                        data.choose_overview(data.overview.selected)
                                    }
                                    keysyms::KEY_Left
                                    | keysyms::KEY_Up
                                    | keysyms::KEY_ISO_Left_Tab => data.step_overview(-1),
                                    keysyms::KEY_Right | keysyms::KEY_Down | keysyms::KEY_Tab => {
                                        data.step_overview(if modifiers.shift { -1 } else { 1 })
                                    }
                                    _ => (),
                                }
                                true
                            } else if modifiers.logo
                                && !modifiers.ctrl
                                && !modifiers.alt
                                && matches!(
                                    symbol,
                                    keysyms::KEY_Up
                                        | keysyms::KEY_Down
                                        | keysyms::KEY_Left
                                        | keysyms::KEY_Right
                                )
                            {
                                match symbol {
                                    keysyms::KEY_Up => data.toggle_maximized_window(),
                                    keysyms::KEY_Down => data.minimize_focused_window(),
                                    keysyms::KEY_Left => data.snap_focused_window(false),
                                    _ => data.snap_focused_window(true),
                                }
                                true
                            } else {
                                false
                            };
                            if handled {
                                data.intercepted_keys.insert(code.raw());
                                return FilterResult::Intercept(None);
                            }
                        }
                        if state == KeyState::Pressed
                            && data.status_panel.open()
                            && !modifiers.ctrl
                            && !modifiers.alt
                            && key.modified_sym().raw() == keysyms::KEY_Escape
                        {
                            data.status_panel = Panel::Closed;
                            data.redraw = true;
                            data.intercepted_keys.insert(code.raw());
                            return FilterResult::Intercept(None);
                        }
                        if state == KeyState::Pressed
                            && !modifiers.ctrl
                            && modifiers.alt
                            && key.modified_sym().raw() == keysyms::KEY_F4
                        {
                            data.close_focused_window();
                            data.intercepted_keys.insert(code.raw());
                            return FilterResult::Intercept(None);
                        }
                        if data.native_session
                            && state == KeyState::Pressed
                            && modifiers.ctrl
                            && modifiers.alt
                            && key.modified_sym().raw() == keysyms::KEY_Escape
                        {
                            data.status_panel = Panel::Logout;
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
                if request_lock {
                    self.apply_status_action(Action::Lock, &open_tool);
                }
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
                self.move_pointer(location, serial, event.time_msec());
                self.redraw |= shortcut.is_some() && !self.lock_active();
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
                self.redraw |= shortcut.is_some() && !self.lock_active();
            }
            InputEvent::PointerButton { event } => {
                let location = pointer.current_location();
                if event.button_code() == BUTTON_LEFT {
                    if event.state() == ButtonState::Released
                        && let Some(pressed) = self.status_pressed.take()
                    {
                        let action = self.status_action_at(location).or_else(|| {
                            self.overview
                                .open
                                .then(|| self.overview_action(location))
                                .flatten()
                        });
                        if action == Some(pressed) {
                            self.apply_status_action(pressed, &open_tool);
                        }
                        self.move_pointer(location, serial, event.time_msec());
                        return;
                    }
                    if event.state() == ButtonState::Released {
                        self.finish_move(location);
                        self.finish_resize();
                        if self.overview.open && !self.status_at(location) {
                            let index = self.overview.hit(location).unwrap_or(usize::MAX);
                            self.choose_overview(index);
                            self.move_pointer(location, serial, event.time_msec());
                            return;
                        }
                    }
                    if event.state() == ButtonState::Pressed && !pointer.is_grabbed() {
                        if let Some(action) = self.status_action_at(location) {
                            self.status_pressed = Some(action);
                            return;
                        }
                        if self.status_panel.open() && !self.status_at(location) {
                            self.status_panel = Panel::Closed;
                            self.redraw = true;
                        }
                        if self.overview.open {
                            self.status_pressed = self.overview_action(location);
                            return;
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
                        self.dock_pressed = Some(action.clone());
                        self.redraw = true;
                        return;
                    }
                    if event.state() == ButtonState::Released
                        && let Some(pressed) = self.dock_pressed.take()
                    {
                        self.redraw = true;
                        if action == Some(&pressed) {
                            match pressed {
                                DockAction::Launch(Target::BuiltIn(app)) => {
                                    self.start_app(app, open_app)
                                }
                                DockAction::Launch(Target::Desktop(id)) => {
                                    self.start_pinned(id, open_app)
                                }
                                DockAction::Add => {
                                    self.apply_status_action(Action::Launch(Tool::Apps), &open_tool)
                                }
                            }
                        }
                        self.move_pointer(location, serial, event.time_msec());
                        return;
                    }
                }
                if self.overview.open {
                    return;
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
                if self.overview.open {
                    return;
                }
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

    fn apply_status_action(&mut self, action: Action, open: impl FnOnce(Tool) -> io::Result<()>) {
        self.status_error = None;
        match action {
            Action::Notifications
                if self.notifications_available && !self.notifications_private(Instant::now()) =>
            {
                if self.status_panel == Panel::Notifications {
                    self.status_panel = Panel::Closed;
                } else {
                    if self.overview.open {
                        self.toggle_overview();
                    }
                    self.notification_page = 0;
                    self.status_panel = Panel::Notifications;
                }
            }
            Action::NotificationQuiet => self.notifications.borrow_mut().toggle_quiet(),
            Action::NotificationClear => self.notifications.borrow_mut().clear(),
            Action::NotificationPage(delta) => {
                self.notification_page = self
                    .notification_page
                    .saturating_add_signed(delta)
                    .min(self.notifications.borrow().pages() - 1);
            }
            Action::NotificationDismiss(id) => self.notifications.borrow_mut().dismiss(id),
            Action::NotificationInvoke(id, index, revision) => {
                let mut center = self.notifications.borrow_mut();
                if center.revision() == revision {
                    center.invoke(id, index);
                }
            }
            Action::Notifications => (),
            Action::Overview => self.toggle_overview(),
            Action::Power => self.status_panel = Panel::Power,
            Action::Lock => {
                let error = if super::lock::local_password_missing() {
                    Some("Set a Debian password in Console first.")
                } else if let Err(error) = self.start_locker() {
                    eprintln!("Could not open screen locker: {error}");
                    Some(if error.kind() == io::ErrorKind::NotFound {
                        "Screen lock needs the Vantage PAM configuration."
                    } else {
                        "Could not open the screen locker."
                    })
                } else {
                    self.status_panel = Panel::Closed;
                    None
                };
                if let Some(error) = error {
                    self.status_panel = Panel::Power;
                    self.status_error = Some(error);
                }
            }
            Action::Toggle => {
                self.status_panel = if self.status_panel.open() {
                    Panel::Closed
                } else {
                    Panel::Settings
                };
            }
            Action::Workspace(index) => {
                self.status_panel = Panel::Closed;
                if self.overview.open && index != self.workspaces.active {
                    self.overview.selected = 0;
                    self.overview.cards.clear();
                }
                self.select_workspace(index);
            }
            Action::Launch(tool) => {
                if self.overview.open {
                    self.toggle_overview();
                }
                match open(tool) {
                    Ok(()) => self.status_panel = Panel::Closed,
                    Err(error) => {
                        eprintln!("Could not open {tool:?}: {error}");
                        self.status_panel = Panel::Settings;
                        self.status_error = Some(match tool {
                            Tool::Apps => "Could not open Applications.",
                            Tool::Network => "Could not open Network & Wi-Fi.",
                            Tool::Bluetooth => "Bluetooth settings needs the blueman package.",
                            Tool::Sound => "Could not open Sound settings.",
                            Tool::Mixer => "The advanced mixer needs pavucontrol.",
                            Tool::Settings => "Could not open Settings.",
                        });
                    }
                }
            }
            Action::Logout if self.status_panel == Panel::Logout => self.closed = true,
            Action::Logout => self.status_panel = Panel::Logout,
            Action::Shutdown if self.status_panel == Panel::Shutdown => self.shutdown(),
            Action::Shutdown => self.status_panel = Panel::Shutdown,
            Action::Cancel => {
                self.status_panel = if self.status_panel == Panel::Power {
                    Panel::Settings
                } else {
                    Panel::Power
                };
            }
        }
        self.redraw = true;
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
        let client = surface
            .and_then(Resource::client)
            .filter(|_| !self.lock_active());
        set_data_device_focus(&self.display, seat, client);
    }

    fn cursor_image(&mut self, _: &Seat<Self>, cursor: CursorImageStatus) {
        self.cursor = cursor;
        self.redraw = true;
    }
}

delegate_seat!(State);
