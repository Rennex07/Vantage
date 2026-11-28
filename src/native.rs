use std::{
    cmp::Reverse, error::Error, ffi::CStr, io, os::unix::net::UnixStream, path::Path, time::Instant,
};

use signal_hook::{
    consts::{SIGINT, SIGTERM},
    low_level::pipe,
};
use smithay::{
    backend::{
        allocator::{
            Fourcc,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{DrmDevice, DrmDeviceFd, DrmDeviceNotifier, DrmEvent, GbmBufferedSurface},
        egl::{EGLContext, EGLDisplay},
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            Bind,
            gles::{GlesRenderer, ffi},
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{all_gpus, primary_gpu},
    },
    output::{Mode, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{
            EventLoop, Interest, Mode as PollMode, PostAction,
            generic::Generic,
            timer::{TimeoutAction, Timer},
        },
        drm::control::{Device, Mode as DrmMode, ModeTypeFlags, connector, crtc},
        input::Libinput,
        rustix::fs::OFlags,
    },
    utils::{DeviceFd, Transform},
};

use crate::{
    frame_metrics::FrameMetrics,
    graphics::Profile,
    session::{Scene, Server},
    shell::launch,
    wayland::{Shortcut, State},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
type Scanout = GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>;

pub fn run() -> Result<()> {
    let requested_size = match std::env::var("VANTAGE_DISPLAY_SIZE") {
        Ok(value) => parse_display_size(&value).or_else(|| {
            eprintln!("Ignoring invalid VANTAGE_DISPLAY_SIZE: expected WIDTHxHEIGHT");
            None
        }),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            eprintln!("Ignoring invalid VANTAGE_DISPLAY_SIZE: expected WIDTHxHEIGHT");
            None
        }
    };
    let (mut session, session_events) = LibSeatSession::new()?;
    let seat = session.seat();
    if !session.is_active() {
        return Err("Start Vantage from an active local TTY login".into());
    }
    let mut outputs = all_gpus(&seat)?;
    if let Some(primary) = primary_gpu(&seat)?
        && let Some(index) = outputs.iter().position(|path| *path == primary)
    {
        outputs.swap(0, index);
    }
    let (kms, drm_events) = outputs
        .iter()
        .find_map(|path| match Kms::open(&mut session, path, requested_size) {
            Ok(output) => Some(output),
            Err(error) => {
                eprintln!("Could not use {}: {error}", path.display());
                None
            }
        })
        .ok_or("No DRM device with a usable connected output on the active seat")?;

    if let Some((width, height)) = requested_size
        && kms.mode.size() != (width, height)
    {
        eprintln!("Could not use requested display size {width}x{height}; using a fallback mode");
    }

    let mut input = Libinput::new_with_udev(LibinputSessionInterface::from(session.clone()));
    input
        .udev_assign_seat(&seat)
        .map_err(|()| io::Error::other("Could not assign libinput seat"))?;

    let mut event_loop = EventLoop::<Native>::try_new()?;
    let handle = event_loop.handle();
    let mut server = Server::new(&handle)?;
    let mut state = State::with_output(
        &server.handle(),
        kms.name.clone(),
        kms.properties(),
        kms.mode.into(),
    )?;
    state.advertise_dmabuf(&kms.renderer);
    state.native_session = true;
    match crate::notification_bus::Service::start(state.notifications.clone()) {
        Ok(service) => {
            state.notifications_available = true;
            let interval = std::time::Duration::from_millis(80);
            handle.insert_source(Timer::from_duration(interval), move |_, _, native| {
                let state = &mut native.state;
                let now = Instant::now();
                let private = state.notifications_private(now);
                let privacy_changed = state.notifications.borrow_mut().set_private(private);
                if (service.dispatch(&state.notifications, now) || privacy_changed) && !private {
                    state.redraw = true;
                }
                TimeoutAction::ToDuration(interval)
            })?;
        }
        Err(error) => eprintln!("Notifications unavailable: {error}"),
    }
    let x11 = match crate::desktop::x11::X11::start(&server.socket) {
        Ok(x11) => Some(x11),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            eprintln!("Could not start Xwayland: {error}");
            None
        }
    };
    let _services = crate::desktop::Services::start(&server.socket, x11.as_ref());
    let metrics = FrameMetrics::new("native DRM", Some(Mode::from(kms.mode).refresh));
    let mut native = Native {
        kms,
        state,
        session,
        input: input.clone(),
        metrics,
        pending_flip: false,
        pending_lock_frame: false,
        x11,
        error: None,
    };
    handle.insert_source(drm_events, |event, _, native| match event {
        DrmEvent::VBlank(crtc) if crtc == native.kms.scanout.crtc() && native.pending_flip => {
            native.pending_flip = false;
            match native.kms.scanout.frame_submitted() {
                Ok(Some(())) => {
                    native
                        .state
                        .frame_presented(Instant::now(), native.pending_lock_frame);
                    native.metrics.presented();
                }
                Ok(None) => native.metrics.cancel_pending(),
                Err(error) => native.error = Some(error.into()),
            }
        }
        DrmEvent::Error(error) => native.error = Some(error.into()),
        _ => (),
    })?;
    handle.insert_source(session_events, |event, _, native| {
        if let Err(error) = native.session_event(event) {
            native.error = Some(error);
        }
    })?;
    let socket = server.socket.clone();
    let app_exits = server.app_exits.clone();
    handle.insert_source(LibinputInputBackend::new(input), move |event, _, native| {
        if !native.session.is_active() {
            return;
        }
        let mut vt = None;
        let x11 = native.x11.as_ref();
        native.state.input(
            event,
            (
                |app, started| launch::open(app, started, &socket, &app_exits, x11),
                |tool: crate::desktop::Tool| tool.open_with_x11(&socket, x11),
            ),
            Some(&mut |shortcut| match shortcut {
                Shortcut::SwitchVt(number) => vt = Some(number),
            }),
        );
        if let Some(vt) = vt
            && let Err(error) = native.session.change_vt(vt)
        {
            eprintln!("Could not switch to VT {vt}: {error}");
        }
    })?;
    let (exit_reader, exit_writer) = UnixStream::pair()?;
    pipe::register(SIGINT, exit_writer.try_clone()?)?;
    pipe::register(SIGTERM, exit_writer)?;
    handle.insert_source(
        Generic::new(exit_reader, Interest::READ, PollMode::Level),
        |_, _, native| {
            native.state.closed = true;
            Ok(PostAction::Remove)
        },
    )?;

    println!(
        "Vantage on {seat}, {} {} at {:.2} Hz ({}), {} logical CPUs available",
        native.kms.name,
        native.kms.mode.name().to_string_lossy(),
        f64::from(Mode::from(native.kms.mode).refresh) / 1000.0,
        native.kms.renderer_name,
        std::thread::available_parallelism().map_or(1, |count| count.get()),
    );
    println!("WAYLAND_DISPLAY={}", server.socket.to_string_lossy());
    while !native.state.closed {
        if let Some(error) = native.error.take() {
            return Err(error);
        }
        server.dispatch(&mut native.state)?;
        native
            .state
            .import_pending_dmabufs(&mut native.kms.renderer);
        server.flush()?;
        if native.session.is_active()
            && !native.pending_flip
            && (native.state.redraw || native.kms.scene.is_animating(&native.state))
        {
            native.state.redraw = false;
            native.pending_lock_frame = native.state.lock_active();
            native.kms.draw(&mut native.state, &mut native.metrics)?;
            native.pending_flip = true;
        }
        // Page flips pace animation and coalesce bursts of client or input activity.
        event_loop.dispatch(None, &mut native)?;
    }
    Ok(())
}

struct Native {
    kms: Kms,
    state: State,
    session: LibSeatSession,
    input: Libinput,
    metrics: FrameMetrics,
    pending_flip: bool,
    pending_lock_frame: bool,
    x11: Option<crate::desktop::x11::X11>,
    error: Option<Box<dyn Error>>,
}

impl AsMut<State> for Native {
    fn as_mut(&mut self) -> &mut State {
        &mut self.state
    }
}

impl Native {
    fn session_event(&mut self, event: SessionEvent) -> Result<()> {
        match event {
            SessionEvent::PauseSession => {
                self.input.suspend();
                self.kms.drm.pause();
                self.state.host_focus(false);
            }
            SessionEvent::ActivateSession => {
                self.kms.drm.activate(true)?;
                // A page flip pending before the VT switch may never arrive.
                if self.pending_flip {
                    self.kms.scanout.frame_submitted()?;
                    self.pending_flip = false;
                    self.metrics.cancel_pending();
                }
                self.kms.scanout.reset_buffers();
                self.input
                    .resume()
                    .map_err(|()| io::Error::other("Could not resume libinput"))?;
                self.state.host_focus(true);
                self.state.redraw = true;
            }
        }
        Ok(())
    }
}

struct Kms {
    scanout: Scanout,
    scene: Scene,
    renderer: GlesRenderer,
    drm: DrmDevice,
    mode: DrmMode,
    name: String,
    dimensions: (i32, i32),
    renderer_name: String,
}

impl Kms {
    fn open(
        session: &mut LibSeatSession,
        path: &Path,
        requested_size: Option<(u16, u16)>,
    ) -> Result<(Self, DrmDeviceNotifier)> {
        let fd = session.open(path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NONBLOCK)?;
        let fd = DrmDeviceFd::new(DeviceFd::from(fd));
        let (mut drm, events) = DrmDevice::new(fd.clone(), false)?;
        let gbm = GbmDevice::new(fd)?;
        // EGL retains a clone of the GBM device until the context is dropped.
        let display = unsafe { EGLDisplay::new(gbm.clone())? };
        let context = EGLContext::new(&display)?;
        // This thread owns the context, renderer, and all GL targets.
        let mut renderer = unsafe { GlesRenderer::new(context)? };
        let renderer_name = renderer.with_context(|gl| {
            // GL_RENDERER is a context-owned, NUL-terminated string when available.
            let name = unsafe { gl.GetString(ffi::RENDERER) };
            if name.is_null() {
                "unknown GLES renderer".to_owned()
            } else {
                unsafe { CStr::from_ptr(name.cast()) }
                    .to_string_lossy()
                    .into_owned()
            }
        })?;
        let driver = path
            .file_name()
            .and_then(|card| {
                std::fs::read_link(Path::new("/sys/class/drm").join(card).join("device/driver"))
                    .ok()
            })
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "unknown".into());
        let profile = Profile::from_environment();
        let safe_graphics = profile.safe(&driver, &renderer_name);
        let scene = Scene::new(&mut renderer, true, safe_graphics)?;
        let formats = renderer.egl_context().display().dmabuf_render_formats();
        let mut failure = None;
        for (connector, crtc, mode) in connected_outputs(&drm, requested_size)? {
            let surface = match drm.create_surface(crtc, mode, &[connector.handle()]) {
                Ok(surface) => surface,
                Err(error) => {
                    failure = Some(Box::new(error) as Box<dyn Error>);
                    continue;
                }
            };
            let allocator = GbmAllocator::new(
                gbm.clone(),
                GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
            );
            match GbmBufferedSurface::new(
                surface,
                allocator,
                &[Fourcc::Xrgb8888, Fourcc::Argb8888],
                formats.iter().copied(),
            ) {
                Ok(scanout) => {
                    let name = format!(
                        "{}-{}",
                        connector.interface().as_str(),
                        connector.interface_id()
                    );
                    let dimensions = connector
                        .size()
                        .map_or((0, 0), |(w, h)| (w as i32, h as i32));
                    println!("DRM device: {}", path.display());
                    println!(
                        "DRM driver: {driver}; graphics profile: {profile:?} (safe effects: {safe_graphics})"
                    );
                    return Ok((
                        Self {
                            scanout,
                            scene,
                            renderer,
                            drm,
                            mode,
                            name,
                            dimensions,
                            renderer_name,
                        },
                        events,
                    ));
                }
                Err(error) => failure = Some(Box::new(error)),
            }
        }
        Err(failure
            .unwrap_or_else(|| "No connected DRM connector with an available mode and CRTC".into()))
    }

    fn properties(&self) -> PhysicalProperties {
        PhysicalProperties {
            size: self.dimensions.into(),
            subpixel: Subpixel::Unknown,
            make: "Vantage".into(),
            model: self.name.clone(),
        }
    }

    fn draw(&mut self, state: &mut State, metrics: &mut FrameMetrics) -> Result<()> {
        let start = metrics.start();
        let (mut buffer, _) = self.scanout.next_buffer()?;
        let mut target = self.renderer.bind(&mut buffer)?;
        let draw_start = metrics.start();
        let size = Mode::from(self.mode).size;
        let shell_scale = (f64::from(size.h) / 1080.0).clamp(1.0, 2.0);
        let sync = self.scene.draw(
            state,
            &mut self.renderer,
            &mut target,
            size,
            Transform::Normal,
            shell_scale,
        )?;
        let submit_start = metrics.start();
        self.scanout.queue_buffer(Some(sync), None, ())?;
        metrics.submitted(start, draw_start, submit_start);
        Ok(())
    }
}

fn connected_outputs(
    drm: &DrmDevice,
    requested_size: Option<(u16, u16)>,
) -> Result<Vec<(connector::Info, crtc::Handle, DrmMode)>> {
    let resources = drm.resource_handles()?;
    let mut outputs = Vec::new();
    for &handle in resources.connectors() {
        let connector = drm.get_connector(handle, true)?;
        if connector.state() != connector::State::Connected || connector.modes().is_empty() {
            continue;
        }
        let current_crtc = connector
            .current_encoder()
            .and_then(|encoder| drm.get_encoder(encoder).ok())
            .and_then(|encoder| encoder.crtc());
        let mut crtcs = Vec::new();
        crtcs.extend(current_crtc);
        for encoder in connector.encoders() {
            if let Ok(encoder) = drm.get_encoder(*encoder) {
                for crtc in resources.filter_crtcs(encoder.possible_crtcs()) {
                    if !crtcs.contains(&crtc) {
                        crtcs.push(crtc);
                    }
                }
            }
        }
        let mut modes = connector.modes().to_vec();
        modes.sort_by_cached_key(|mode| {
            Reverse(mode_priority(
                (*mode).into(),
                mode.mode_type().contains(ModeTypeFlags::PREFERRED),
            ))
        });
        for mode in modes {
            for &crtc in &crtcs {
                outputs.push((connector.clone(), crtc, mode));
            }
        }
    }
    if let Some(size) = requested_size {
        // Keep the normal connector and mode order within each group.
        outputs.sort_by_key(|(_, _, mode)| mode.size() != size);
    }
    Ok(outputs)
}

fn parse_display_size(value: &str) -> Option<(u16, u16)> {
    let (width, height) = value.split_once('x')?;
    let size = (width.parse().ok()?, height.parse().ok()?);
    (size.0 > 0 && size.1 > 0).then_some(size)
}

fn mode_priority(mode: Mode, preferred: bool) -> (bool, i64, i32) {
    (
        preferred,
        i64::from(mode.size.w) * i64::from(mode.size.h),
        mode.refresh,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_size_requires_two_positive_dimensions() {
        assert_eq!(parse_display_size("1920x1080"), Some((1920, 1080)));
        for value in ["1920", "1920x0", "-1x1080", "1920x1080@60", "65536x1080"] {
            assert_eq!(parse_display_size(value), None, "{value}");
        }
    }
    #[test]
    fn preferred_modes_rank_before_larger_modes() {
        let large = Mode {
            size: (3840, 2160).into(),
            refresh: 60_000,
        };
        let small = Mode {
            size: (1920, 1080).into(),
            refresh: 144_000,
        };
        assert!(mode_priority(small, true) > mode_priority(large, false));
    }

    #[test]
    fn refresh_breaks_size_ties_and_preferred_takes_priority() {
        let slow = Mode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        };
        let fast = Mode {
            size: (1920, 1080).into(),
            refresh: 144_000,
        };
        assert!(mode_priority(fast, false) > mode_priority(slow, false));
        assert!(mode_priority(fast, true) > mode_priority(fast, false));
    }
}
