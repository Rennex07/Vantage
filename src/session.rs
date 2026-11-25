use std::{
    error::Error,
    ffi::OsString,
    os::fd::AsFd,
    sync::Arc,
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Frame, ImportMem, Renderer, Texture,
            gles::{
                GlesFrame, GlesPixelProgram, GlesRenderer, GlesTarget, GlesTexture, Uniform,
                UniformName, UniformType,
            },
            sync::SyncPoint,
            utils::draw_render_elements,
        },
    },
    reexports::{
        calloop::{
            Interest, LoopHandle, Mode, PostAction,
            channel::{self, Sender},
            generic::Generic,
            timer::{TimeoutAction, Timer},
        },
        wayland_server::{Display, DisplayHandle},
    },
    utils::{Logical, Physical, Rectangle, Size, Transform},
    wayland::socket::ListeningSocketSource,
};

use crate::{
    shell::{
        dock::{Dock, DockFrame},
        glass::Glass,
        launch::{self, App},
        status::{self, StatusBar},
        theme,
    },
    wallpaper,
    wayland::{ClientState, State},
};

pub struct Server {
    display: Display<State>,
    pub socket: OsString,
    pub app_exits: Sender<launch::Exit>,
}

impl Server {
    pub fn new<T: AsMut<State> + 'static>(
        handle: &LoopHandle<'_, T>,
    ) -> Result<Self, Box<dyn Error>> {
        let display = Display::<State>::new()?;
        let listener = ListeningSocketSource::new_auto()?;
        let socket = listener.socket_name().to_owned();
        let mut clients = display.handle();
        handle.insert_source(listener, move |stream, _, _| {
            if let Err(error) = clients.insert_client(stream, Arc::new(ClientState::default())) {
                eprintln!("Could not register Wayland client: {error}");
            }
        })?;
        let client_events = Generic::new(
            display.as_fd().try_clone_to_owned()?,
            Interest::READ,
            Mode::Level,
        );
        handle.insert_source(client_events, |_, _, data| {
            data.as_mut().pending_clients = true;
            Ok(PostAction::Continue)
        })?;
        let (app_exits, exit_events) = channel::channel();
        handle.insert_source(exit_events, |event, _, data| {
            if let channel::Event::Msg(exit) = event {
                data.as_mut().launch_exited(exit);
            }
        })?;
        let interval = Duration::from_secs(2);
        handle.insert_source(Timer::from_duration(interval), move |_, _, data| {
            data.as_mut().refresh_status();
            TimeoutAction::ToDuration(interval)
        })?;
        Ok(Self {
            display,
            socket,
            app_exits,
        })
    }

    pub fn handle(&self) -> DisplayHandle {
        self.display.handle()
    }

    pub fn dispatch(&mut self, state: &mut State) -> Result<(), Box<dyn Error>> {
        if std::mem::take(&mut state.pending_clients) {
            self.display.dispatch_clients(state)?;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), Box<dyn Error>> {
        self.display.flush_clients()?;
        Ok(())
    }
}

pub struct Scene {
    wallpaper: GlesTexture,
    glass: Option<Glass>,
    glass_failed: bool,
    dock: Dock,
    status: StatusBar,
    cursor: Option<GlesTexture>,
    launch_card: GlesPixelProgram,
    notice: Option<(App, &'static str, GlesTexture)>,
    hints: [GlesTexture; App::ALL.len()],
}

impl Scene {
    pub fn new(renderer: &mut GlesRenderer, draw_cursor: bool) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            wallpaper: wallpaper::load(renderer)?,
            glass: Glass::new(renderer)
                .map_err(|error| {
                    eprintln!("Could not initialize Files glass: {error}");
                })
                .ok(),
            glass_failed: false,
            dock: Dock::new(renderer)?,
            status: StatusBar::default(),
            cursor: draw_cursor.then(|| load_cursor(renderer)).transpose()?,
            launch_card: renderer.compile_custom_pixel_shader(
                include_str!("shell/launch.frag"),
                &[UniformName::new("accent", UniformType::_3f)],
            )?,
            notice: None,
            hints: [
                hint_texture(renderer, App::Console)?,
                hint_texture(renderer, App::Files)?,
            ],
        })
    }

    pub fn is_animating(&self, state: &State) -> bool {
        self.dock.is_animating()
            || state.workspaces.is_animating()
            || state.windows_are_opening()
            || state.shell_is_animating()
    }

    pub fn draw(
        &mut self,
        state: &mut State,
        renderer: &mut GlesRenderer,
        target: &mut GlesTarget<'_>,
        size: Size<i32, Physical>,
        transform: Transform,
        scale: f64,
    ) -> Result<SyncPoint, Box<dyn Error>> {
        let now = Instant::now();
        state.finish_shell_frame(now);
        let position = state.workspaces.position(now);
        let bounds = Rectangle::from_size(size);
        let top = status::bounds(size.w, scale);
        let panel_bounds = status::panel_bounds(size, top);
        state.status_hits = Some((top, panel_bounds));
        let bar = self
            .status
            .texture(renderer, top.size, &state.status, state.status_open)?
            .clone();
        let panel = if state.status_open {
            Some(
                self.status
                    .panel(renderer, &state.status, state.workspaces.active)?
                    .clone(),
            )
        } else {
            None
        };
        let notice = if let Some((app, message, _)) = state.notice {
            if !self
                .notice
                .as_ref()
                .is_some_and(|(prev, text, _)| *prev == app && *text == message)
            {
                self.notice = Some((app, message, notice_texture(renderer, app, message)?));
            }
            self.notice.as_ref().map(|(_, _, texture)| texture)
        } else {
            None
        };
        let windows = state.window_elements(renderer, position, now);
        let glass_scene = if !self.glass_failed
            && !windows.glass.is_empty()
            && let Some(glass) = &mut self.glass
        {
            match glass.compose(renderer, &self.wallpaper, size, &windows) {
                Ok(texture) => Some(texture),
                Err(error) => {
                    eprintln!("Could not render Files glass: {error}");
                    self.glass_failed = true;
                    None
                }
            }
        } else {
            None
        };
        let mut frame = renderer.render(target, size, transform)?;
        if let Some(scene) = glass_scene {
            draw_texture(&mut frame, scene, bounds, bounds)?;
        } else {
            wallpaper::draw(&mut frame, &self.wallpaper, bounds)?;
            draw_render_elements(&mut frame, 1.0, &windows.elements, &[bounds])?;
        }
        for (app, rect, alpha) in state
            .launch_tiles(now)
            .chain(state.opening_tiles(position, now))
        {
            draw_launch_card(&mut frame, &self.launch_card, bounds, app, rect, alpha)?;
        }
        draw_texture(&mut frame, &bar, top, bounds)?;
        if let Some(panel) = panel {
            draw_texture(&mut frame, &panel, panel_bounds, bounds)?;
        }
        state.dock_hits = self.dock.draw(
            &mut frame,
            &self.wallpaper,
            bounds,
            DockFrame {
                scale,
                now,
                pressed: state.dock_pressed,
                pointer: state.pointer_location(),
                launching: state.dock_launch_intensity(now),
                workspace_position: position,
            },
        )?;
        if let Some(texture) = notice {
            let width = texture.size().w.min(size.w - 16).max(1);
            let rect = Rectangle::new(
                ((size.w - width) / 2, top.size.h + 12).into(),
                (width, texture.size().h).into(),
            );
            draw_texture(&mut frame, texture, rect, bounds)?;
        } else if state.dock_pressed.is_none()
            && !state.shell_is_animating()
            && !state.windows_are_opening()
            && let Some(dock) = &state.dock_hits
            && let Some(crate::shell::dock::DockAction::App(app)) =
                dock.action_at(state.pointer_location())
        {
            let texture = &self.hints[app.index()];
            let anchor = dock.anchor(app);
            let rect = Rectangle::new(
                (
                    anchor.x.round() as i32 - texture.size().w / 2,
                    anchor.y.round() as i32 - 70,
                )
                    .into(),
                (texture.size().w, texture.size().h).into(),
            );
            draw_texture(&mut frame, texture, rect, bounds)?;
        }
        if let Some(cursor) = &self.cursor {
            let position = state.pointer_location().to_i32_round();
            let target = Rectangle::new((position.x, position.y).into(), (14, 18).into());
            if let Some(visible) = target.intersection(bounds) {
                Frame::render_texture_from_to(
                    &mut frame,
                    cursor,
                    Rectangle::from_size((14.0, 18.0).into()),
                    target,
                    &[Rectangle::from_size(visible.size)],
                    &[],
                    Transform::Normal,
                    1.0,
                )?;
            }
        }
        let sync = frame.finish()?;
        self.dock.finish_frame(now);
        state.workspaces.finish_frame(now);
        state.finish_window_openings(now);
        Ok(sync)
    }
}

fn draw_texture(
    frame: &mut GlesFrame<'_, '_>,
    texture: &GlesTexture,
    target: Rectangle<i32, Physical>,
    bounds: Rectangle<i32, Physical>,
) -> Result<(), smithay::backend::renderer::gles::GlesError> {
    if let Some(visible) = target.intersection(bounds) {
        Frame::render_texture_from_to(
            frame,
            texture,
            Rectangle::from_size(texture.size().to_f64()),
            target,
            &[Rectangle::new(visible.loc - target.loc, visible.size)],
            &[],
            Transform::Normal,
            1.0,
        )?;
    }
    Ok(())
}

fn draw_launch_card(
    frame: &mut GlesFrame<'_, '_>,
    program: &GlesPixelProgram,
    bounds: Rectangle<i32, Physical>,
    app: App,
    rect: Rectangle<f64, Logical>,
    opacity: f32,
) -> Result<(), smithay::backend::renderer::gles::GlesError> {
    let rect = Rectangle::new(
        (rect.loc.x.round() as i32, rect.loc.y.round() as i32).into(),
        (rect.size.w.round() as i32, rect.size.h.round() as i32).into(),
    );
    if let Some(visible) = rect.intersection(bounds) {
        use crate::shell::theme::{AMBER, BLUE};
        let (r, g, b) = if app == App::Console { BLUE } else { AMBER };
        let size = (rect.size.w, rect.size.h).into();
        frame.render_pixel_shader_to(
            program,
            Rectangle::from_size((f64::from(rect.size.w), f64::from(rect.size.h)).into()),
            rect,
            size,
            Some(&[Rectangle::new(visible.loc - rect.loc, visible.size)]),
            opacity,
            &[Uniform::new("accent", (r as f32, g as f32, b as f32))],
        )?;
    }
    Ok(())
}

fn notice_texture(
    renderer: &mut GlesRenderer,
    app: App,
    message: &str,
) -> Result<GlesTexture, Box<dyn Error>> {
    use vte4::gtk::cairo::FontWeight;
    theme::texture(renderer, (376, 68).into(), |context| {
        theme::card(context, 376.0, 68.0, 14.0)?;
        theme::ink(context, theme::AMBER, 1.0);
        theme::dot(context, 19.0, 24.0, 3.0)?;
        theme::font(context, 14.0, FontWeight::Bold);
        theme::ink(context, theme::TEXT, 1.0);
        context.move_to(31.0, 29.0);
        context.show_text(&format!("Could not open {}", app.name()))?;
        theme::font(context, 12.0, FontWeight::Normal);
        theme::ink(context, theme::MUTED, 1.0);
        context.move_to(17.0, 51.0);
        context.show_text(message)
    })
}

fn hint_texture(renderer: &mut GlesRenderer, app: App) -> Result<GlesTexture, Box<dyn Error>> {
    use vte4::gtk::cairo::FontWeight;
    let width = if app == App::Console { 84 } else { 58 };
    theme::texture(renderer, (width, 30).into(), |context| {
        theme::card(context, f64::from(width), 30.0, 10.0)?;
        theme::font(context, 12.0, FontWeight::Bold);
        theme::ink(context, theme::TEXT, 1.0);
        let text = context.text_extents(app.name())?;
        context.move_to((f64::from(width) - text.x_advance()) / 2.0, 19.5);
        context.show_text(app.name())
    })
}

fn load_cursor(
    renderer: &mut GlesRenderer,
) -> Result<GlesTexture, smithay::backend::renderer::gles::GlesError> {
    let pixels = [
        "#             ",
        "##            ",
        "#+#           ",
        "#++#          ",
        "#+++#         ",
        "#++++#        ",
        "#+++++#       ",
        "#++++++#      ",
        "#+++++++#     ",
        "#++++++++#    ",
        "#+++++++++#   ",
        "#++++++#####  ",
        "#++#++#       ",
        "#+# #++#      ",
        "##  #++#      ",
        "#    #++#     ",
        "     #++#     ",
        "      ##      ",
    ];
    let rgba: Vec<_> = pixels
        .iter()
        .flat_map(|row| row.bytes())
        .flat_map(|pixel| match pixel {
            b'#' => [18, 22, 28, 255],
            b'+' => [245, 248, 252, 255],
            _ => [0, 0, 0, 0],
        })
        .collect();
    renderer.import_memory(&rgba, Fourcc::Abgr8888, (14, 18).into(), false)
}
