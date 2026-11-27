use std::time::{Duration, Instant};

use smithay::{
    delegate_compositor, delegate_output, delegate_shm, delegate_viewporter,
    desktop::{PopupManager, Space, Window},
    input::{
        SeatState,
        keyboard::{KeyboardHandle, XkbConfig},
        pointer::{CursorImageStatus, PointerHandle},
    },
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::wayland_server::{
        Client, DisplayHandle,
        backend::ClientData,
        protocol::{wl_buffer::WlBuffer, wl_surface::WlSurface},
    },
    utils::{Logical, Physical, Point, Rectangle, Size, Transform},
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        output::OutputHandler,
        selection::data_device::DataDeviceState,
        shell::xdg::XdgShellState,
        shm::{ShmHandler, ShmState},
        viewporter::ViewporterState,
    },
};

use crate::{
    preferences::Appearance,
    shell::{
        dock::{Action as DockAction, DockHits},
        launch::{self, App, Exit, Target},
        motion::{LAUNCH_TIMEOUT, Launch, Opening},
        status::{self, Action, Panel, Status},
        workspaces::{WORKSPACE_COUNT, Workspaces},
    },
};

mod clipboard;
mod dmabuf;
mod input;
mod lock;
pub(crate) mod overview;
mod windows;

pub use input::Shortcut;
pub use windows::WindowScene;

#[derive(Default)]
pub struct ClientState {
    compositor: CompositorClientState,
    locker: bool,
}

impl ClientData for ClientState {}

pub struct State {
    compositor: CompositorState,
    shm: ShmState,
    dmabuf: dmabuf::Buffers,
    display: DisplayHandle,
    data_device: DataDeviceState,
    output: Output,
    spaces: [Space<Window>; WORKSPACE_COUNT],
    opening_windows: Vec<(Window, Opening)>,
    dragged_window: Option<windows::WindowDrag>,
    resizing_window: Option<Window>,
    overview: overview::Overview,
    switching_windows: bool,
    minimized_windows: Vec<Window>,
    pending_launches: [Option<Launch>; App::ALL.len()],
    pub notice: Option<(String, &'static str, Instant)>,
    pub status_panel: Panel,
    pub status_hovered: Option<Action>,
    status_pressed: Option<Action>,
    pub status_error: Option<&'static str>,
    pub native_session: bool,
    lock: lock::Lock,
    pub status_hits: Option<(Rectangle<i32, Physical>, Rectangle<i32, Physical>)>,
    host_focused: bool,
    pub workspaces: Workspaces,
    popups: PopupManager,
    xdg_shell: XdgShellState,
    seat_state: SeatState<Self>,
    keyboard: KeyboardHandle<Self>,
    pointer: PointerHandle<Self>,
    cursor: CursorImageStatus,
    intercepted_keys: std::collections::HashSet<u32>,
    pub redraw: bool,
    pub closed: bool,
    pub pending_clients: bool,
    pub dock_hits: Option<DockHits>,
    pub dock_pressed: Option<DockAction>,
    pub dock_pins: Vec<crate::preferences::DockPin>,
    pub status: Status,
    pub display_brightness: u8,
    pub appearance: Appearance,
}

impl State {
    pub fn new(
        display: &DisplayHandle,
        size: Size<i32, Physical>,
    ) -> Result<Self, smithay::input::keyboard::Error> {
        Self::with_output(
            display,
            "Vantage-Nested".into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Vantage".into(),
                model: "Nested window".into(),
            },
            Mode {
                size,
                refresh: 60_000,
            },
        )
    }

    pub fn with_output(
        display: &DisplayHandle,
        name: String,
        properties: PhysicalProperties,
        mode: Mode,
    ) -> Result<Self, smithay::input::keyboard::Error> {
        let output = Output::new(name, properties);
        output.create_global::<Self>(display);
        output.change_current_state(
            None,
            Some(Transform::Normal),
            Some(Scale::Integer(1)),
            Some((0, 0).into()),
        );
        let spaces = std::array::from_fn(|_| {
            let mut space = Space::default();
            space.map_output(&output, (0, 0));
            space
        });
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(display, "Vantage");
        let keyboard = seat.add_keyboard(XkbConfig::default(), 250, 30)?;
        let pointer = seat.add_pointer();
        ViewporterState::new::<Self>(display);
        let mut state = Self {
            spaces,
            opening_windows: Vec::new(),
            dragged_window: None,
            resizing_window: None,
            overview: overview::Overview::default(),
            switching_windows: false,
            minimized_windows: Vec::new(),
            pending_launches: [None; App::ALL.len()],
            notice: None,
            status_panel: Panel::Closed,
            status_hovered: None,
            status_pressed: None,
            status_error: None,
            native_session: false,
            lock: lock::Lock::new(display),
            status_hits: None,
            host_focused: true,
            workspaces: Workspaces::default(),
            popups: PopupManager::default(),
            xdg_shell: XdgShellState::new::<Self>(display),
            seat_state,
            keyboard,
            pointer,
            cursor: CursorImageStatus::default_named(),
            intercepted_keys: Default::default(),
            compositor: CompositorState::new::<Self>(display),
            shm: ShmState::new::<Self>(display, Vec::new()),
            dmabuf: dmabuf::Buffers::new(),
            redraw: true,
            closed: false,
            pending_clients: false,
            dock_hits: None,
            dock_pressed: None,
            dock_pins: crate::preferences::dock_pins(),
            status: Status::read(),
            display_brightness: crate::preferences::brightness(),
            appearance: crate::preferences::appearance(),
            display: display.clone(),
            data_device: DataDeviceState::new::<Self>(display),
            output,
        };
        state.set_output_mode(mode);
        Ok(state)
    }

    fn start_app(&mut self, app: App, open: impl FnOnce(Target, Instant) -> std::io::Result<()>) {
        if self.restore_dock_window(app) || self.pending_launches[app.index()].is_some() {
            return;
        }
        let Some(dock) = &self.dock_hits else {
            return;
        };
        let launch = Launch {
            app,
            started: Instant::now(),
            anchor: dock.anchor(app.index()),
            workspace: self.workspaces.active,
        };
        self.notice = None;
        match open(Target::BuiltIn(app), launch.started) {
            Ok(()) => self.pending_launches[app.index()] = Some(launch),
            Err(error) => {
                eprintln!("Could not open {}: {error}", app.name());
                self.show_notice(app.name().into(), "The app could not be started.");
            }
        }
        self.redraw = true;
    }

    fn start_pinned(
        &mut self,
        id: String,
        open: impl FnOnce(Target, Instant) -> std::io::Result<()>,
    ) {
        self.notice = None;
        if !self.focus_pinned_window(&id)
            && let Err(error) = open(Target::Desktop(id.clone()), Instant::now())
        {
            eprintln!("Could not open {id}: {error}");
            self.show_notice(launch::desktop_name(&id), "The app could not be started.");
        }
        self.redraw = true;
    }

    fn show_notice(&mut self, name: String, message: &'static str) {
        self.notice = Some((name, message, Instant::now() + Duration::from_secs(5)));
        self.redraw = true;
    }

    pub fn launch_exited(&mut self, exit: Exit) {
        match exit.target {
            Target::BuiltIn(app) => {
                let pending = &mut self.pending_launches[app.index()];
                if pending.is_some_and(|launch| launch.started == exit.started) {
                    *pending = None;
                    self.show_notice(app.name().into(), exit.message);
                }
            }
            Target::Desktop(id) => self.show_notice(launch::desktop_name(&id), exit.message),
        }
    }

    pub fn finish_shell_frame(&mut self, now: Instant) {
        for app in App::ALL {
            let pending = &mut self.pending_launches[app.index()];
            if pending.is_some_and(|launch| now >= launch.started + LAUNCH_TIMEOUT) {
                *pending = None;
                self.show_notice(
                    app.name().into(),
                    "No window appeared. Click the dock icon to retry.",
                );
            }
        }
    }

    pub fn shell_is_animating(&self) -> bool {
        !self.appearance.reduce_motion && self.pending_launches.iter().any(Option::is_some)
    }

    pub fn dock_launch_intensity(&self, now: Instant) -> (f32, f32) {
        let strength = |app: App| {
            self.pending_launches[app.index()].map_or(0.0, |launch| {
                if self.appearance.reduce_motion {
                    return 0.8;
                }
                let elapsed = now.saturating_duration_since(launch.started).as_secs_f32();
                0.65 + 0.35 * (elapsed * 9.0).cos()
            })
        };
        (strength(App::Console), strength(App::Files))
    }

    pub fn launch_tiles(
        &self,
        now: Instant,
    ) -> impl Iterator<Item = (App, Rectangle<f64, Logical>, f32)> + '_ {
        self.pending_launches
            .iter()
            .flatten()
            .filter(|launch| {
                !self.appearance.reduce_motion && launch.workspace == self.workspaces.active
            })
            .map(move |launch| (launch.app, launch.tile(now), 1.0))
    }

    fn status_at(&self, point: Point<f64, Logical>) -> bool {
        self.status_hits.is_some_and(|(bar, panel)| {
            bar.to_f64().to_logical(1.0).contains(point)
                || (self.status_panel.open() && panel.to_f64().to_logical(1.0).contains(point))
        })
    }

    fn status_action_at(&self, point: Point<f64, Logical>) -> Option<Action> {
        self.status_hits
            .and_then(|hits| status::action_at(hits, point, self.status_panel, self.native_session))
    }

    pub fn refresh_preferences(&mut self) {
        self.check_locker();
        let pins = crate::preferences::dock_pins();
        if self.dock_pins != pins {
            self.dock_pins = pins;
            self.dock_hits = None;
            self.dock_pressed = None;
            self.redraw = true;
        }
        let appearance = crate::preferences::appearance();
        if self.appearance != appearance {
            self.appearance = appearance;
            if appearance.reduce_motion {
                self.workspaces.settle();
                self.overview.settle();
                self.opening_windows.clear();
            }
            self.redraw = true;
        }
        self.finish_shell_frame(Instant::now());
        let brightness = crate::preferences::brightness();
        if self.display_brightness != brightness {
            self.display_brightness = brightness;
            self.redraw = true;
        }
    }

    pub fn refresh_status(&mut self) {
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, _, until)| Instant::now() >= *until)
        {
            self.notice = None;
            self.redraw = true;
        }
        let status = Status::read();
        if status != self.status {
            self.status = status;
            self.redraw = true;
        }
    }

    pub fn resize_output(&mut self, size: Size<i32, Physical>) {
        self.set_output_mode(Mode {
            size,
            refresh: 60_000,
        });
    }

    fn set_output_mode(&mut self, mode: Mode) {
        if mode.size.w <= 0 || mode.size.h <= 0 {
            return;
        }
        let previous = self.output.current_mode();
        if previous == Some(mode) {
            return;
        }
        self.output.set_preferred(mode);
        self.output
            .change_current_state(Some(mode), None, None, None);
        if let Some(previous) = previous {
            self.output.delete_mode(previous);
        }
        self.center_windows();
        self.configure_locker();
    }
}

impl AsMut<State> for State {
    fn as_mut(&mut self) -> &mut State {
        self
    }
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client
            .get_data::<ClientState>()
            .expect("registered Vantage client")
            .compositor
    }

    fn commit(&mut self, surface: &WlSurface) {
        smithay::backend::renderer::utils::on_commit_buffer_handler::<Self>(surface);
        if self.lock_surface() == Some(surface) {
            self.update_lock_focus();
        }
        self.surface_committed(surface);
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _: &WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm
    }
}

impl OutputHandler for State {}

delegate_compositor!(State);
delegate_output!(State);
delegate_shm!(State);
delegate_viewporter!(State);
