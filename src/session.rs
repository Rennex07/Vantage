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
            element::{
                Kind,
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            },
            gles::{
                GlesFrame, GlesPixelProgram, GlesRenderer, GlesTarget, GlesTexture, Uniform,
                UniformName, UniformType,
            },
            sync::SyncPoint,
            utils::draw_render_elements,
        },
    },
    input::pointer::{CursorImageStatus, CursorImageSurfaceData},
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
    wayland::{compositor::with_states, socket::ListeningSocketSource},
};

use crate::{
    palette::Mode as ColorMode,
    preferences::{Aod, Effects},
    shell::{
        dock::{Dock, DockFrame},
        glass::{FrostedWindow, Glass, GlassStyle},
        launch::{self, App, Target},
        lockscreen::LockScreen,
        notifications as notification_ui,
        status::{self, Panel, StatusBar},
        theme,
        timeline::{Animation, Ease, progress},
    },
    wallpaper::{self, Wallpaper},
    wayland::{ClientState, State, overview},
};

type OverviewStyle = (crate::palette::Theme, usize, [usize; 3]);

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
        let preferences_interval = Duration::from_millis(220);
        handle.insert_source(
            Timer::from_duration(preferences_interval),
            move |_, _, data| {
                data.as_mut().refresh_preferences();
                TimeoutAction::ToDuration(preferences_interval)
            },
        )?;
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

#[derive(Default)]
struct PanelMotion {
    visible: Panel,
    motion: Animation,
}

impl PanelMotion {
    fn amount(&mut self, target: Panel, now: Instant, reduce_motion: bool) -> f64 {
        if target.open() {
            self.visible = target;
        }
        let duration = if reduce_motion {
            Duration::ZERO
        } else {
            Duration::from_millis(180)
        };
        self.motion
            .retarget(f64::from(target.open()), now, duration, Ease::Out);
        self.motion.finish_frame(now);
        self.motion.value(now)
    }
}

pub struct Scene {
    wallpaper: Wallpaper,
    glass: Option<Glass>,
    glass_failed: bool,
    lock_blur_failed: bool,
    dock: Dock,
    status: StatusBar,
    panel_motion: PanelMotion,
    fullscreen_motion: Animation,
    lockscreen: LockScreen,
    started: Instant,
    cursor: Option<GlesTexture>,
    launch_card: GlesPixelProgram,
    notice: Option<(String, &'static str, GlesTexture)>,
    notification_panel: Option<(u64, usize, crate::palette::Theme, GlesTexture)>,
    notification_banner: Option<(u64, crate::palette::Theme, GlesTexture)>,
    label_theme: crate::palette::Theme,
    previous_bar: Option<(Size<i32, Physical>, GlesTexture)>,
    overview_labels: std::collections::HashMap<String, GlesTexture>,
    overview_controls: Option<(OverviewStyle, GlesTexture)>,
}

impl Scene {
    pub fn new(
        renderer: &mut GlesRenderer,
        draw_cursor: bool,
        safe_graphics: bool,
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            wallpaper: Wallpaper::new(renderer)?,
            glass: (!safe_graphics)
                .then(|| Glass::new(renderer))
                .and_then(|result| {
                    result
                        .map_err(|error| {
                            eprintln!(
                                "Using solid surfaces after glass initialization failed: {error}"
                            )
                        })
                        .ok()
                }),
            glass_failed: false,
            lock_blur_failed: false,
            dock: Dock::new(renderer)?,
            status: StatusBar::default(),
            panel_motion: PanelMotion::default(),
            fullscreen_motion: Animation::default(),
            lockscreen: LockScreen::default(),
            started: Instant::now(),
            cursor: draw_cursor.then(|| load_cursor(renderer)).transpose()?,
            launch_card: renderer.compile_custom_pixel_shader(
                include_str!("shell/launch.frag"),
                &[UniformName::new("accent", UniformType::_3f)],
            )?,
            notice: None,
            notification_panel: None,
            notification_banner: None,
            label_theme: Default::default(),
            previous_bar: None,
            overview_labels: Default::default(),
            overview_controls: None,
        })
    }

    pub fn is_animating(&self, state: &State) -> bool {
        let now = Instant::now();
        if state.lock_active() {
            state.lock_animating(now) || self.wallpaper.animating(now)
        } else {
            self.dock.is_animating()
                || state.workspaces.is_animating()
                || state.windows_are_opening()
                || state.shell_is_animating()
                || state.overview_animating()
                || self.panel_motion.motion.is_animating()
                || self.fullscreen_motion.is_animating()
                || self.wallpaper.animating(now)
                || state.unlock_amount(now) > 0.0
                || (!state.appearance.reduce_motion
                    && progress(now, self.started, Duration::from_millis(520)) < 1.0)
        }
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
        let wallpaper = self.wallpaper.texture(
            renderer,
            size,
            state.appearance.theme,
            now,
            state.appearance.reduce_motion,
        )?;
        let bounds = Rectangle::from_size(size);
        let cursor_size = ((14.0 * scale).round() as i32, (18.0 * scale).round() as i32);
        let light = state.appearance.theme.mode == ColorMode::Light;
        let lock_wallpaper = if light { 0.56 } else { 0.94 };
        let unlocking = state.unlock_amount(now);
        let sign_bounds = crate::shell::lockscreen::sign_bounds(size.w, size.h);
        let solid_lock =
            state.appearance.reduce_transparency || state.appearance.effects == Effects::Reduced;
        let (reveal, dim, sign) = state.lock_amounts(now);
        let blur_amount = if state.lock_active() { sign } else { unlocking };
        let lock_blur = if blur_amount > 0.0 && !solid_lock && !self.lock_blur_failed {
            self.glass.as_mut().and_then(|glass| {
                glass
                    .lock_blur(
                        renderer,
                        &wallpaper,
                        size,
                        state.appearance.theme,
                        self.wallpaper.animating(now),
                    )
                    .map_err(|error| {
                        self.lock_blur_failed = true;
                        eprintln!("Using a solid lock prompt after backdrop blur failed: {error}");
                    })
                    .ok()
            })
        } else {
            None
        };
        if state.lock_active() {
            self.panel_motion = PanelMotion::default();
            self.notification_panel = None;
            self.notification_banner = None;
            let ambient = match state.appearance.aod {
                Aod::Wallpaper => {
                    if light {
                        0.26
                    } else {
                        0.40
                    }
                }
                Aod::Classic => 0.0,
                Aod::Off => lock_wallpaper,
            };
            let clock = self
                .lockscreen
                .clock(renderer, size, &state.status)?
                .clone();
            let clock_alpha = reveal * (1.0 - sign);
            let hint_alpha = clock_alpha * (1.0 - dim);
            let sign_alpha = reveal * sign;
            let hint = if hint_alpha > 0.0 {
                Some(
                    self.lockscreen
                        .hint(renderer, state.appearance.theme)?
                        .clone(),
                )
            } else {
                None
            };
            let sign_texture = if sign_alpha > 0.0 {
                Some(
                    self.lockscreen
                        .sign_in(
                            renderer,
                            sign_bounds.size.w,
                            state.appearance.theme,
                            state.lock_sign_in(),
                            solid_lock || lock_blur.is_none(),
                        )?
                        .clone(),
                )
            } else {
                None
            };
            let mut frame = renderer.render(target, size, transform)?;
            wallpaper::draw(&mut frame, &wallpaper, bounds, 1.0)?;
            if let Some(blur) = lock_blur.as_ref() {
                wallpaper::draw(&mut frame, blur, bounds, sign_alpha)?;
            }
            let brightness = lock_wallpaper + (ambient - lock_wallpaper) * dim;
            let scrim = reveal * (1.0 - brightness * (1.0 - 0.20 * sign));
            frame.draw_solid(bounds, &[bounds], [0.0, 0.0, 0.0, scrim].into())?;
            draw_texture_with_opacity(
                &mut frame,
                &clock,
                lock_clock_bounds(&clock, bounds, dim, sign),
                bounds,
                clock_alpha,
            )?;
            if let Some(hint) = hint {
                let width = (f64::from(hint.size().w) * scale).round() as i32;
                let height = (f64::from(hint.size().h) * scale).round() as i32;
                let size_hint = (width, height).into();
                let hx = (size.w - width) / 2;
                let hy = size.h - height - (size.h / 10).max(12);
                draw_texture_with_opacity(
                    &mut frame,
                    &hint,
                    Rectangle::new((hx, hy).into(), size_hint),
                    bounds,
                    hint_alpha,
                )?;
            }
            if let Some(sign_texture) = sign_texture {
                let mut position = sign_bounds;
                position.loc.y += (18.0 * (1.0 - sign)).round() as i32;
                draw_texture_with_opacity(&mut frame, &sign_texture, position, bounds, sign_alpha)?;
                if let Some(cursor) = &self.cursor {
                    let pointer = state.pointer_location().to_i32_round();
                    draw_texture_with_opacity(
                        &mut frame,
                        cursor,
                        Rectangle::new((pointer.x, pointer.y).into(), cursor_size.into()),
                        bounds,
                        sign_alpha,
                    )?;
                }
            }
            let sync = frame.finish()?;
            state.finish_lock_frame(now);
            return Ok(sync);
        }
        state.finish_shell_frame(now);
        let running = state.running_dock_apps();
        let focused = state.focused_dock_app();
        let mut apps = state.dock_pins.clone();
        let mut unpinned: Vec<_> = running
            .iter()
            .filter_map(|target| match target {
                Target::Desktop(id) if !apps.iter().any(|pin| pin.id == *id) => Some(id),
                _ => None,
            })
            .collect();
        unpinned.sort_unstable();
        apps.extend(
            unpinned
                .into_iter()
                .take(6)
                .map(|id| crate::preferences::DockPin {
                    id: id.clone(),
                    icon: None,
                }),
        );
        self.dock.sync_apps(renderer, &apps)?;
        let position = state.workspaces.position(now);
        let fullscreen = state.fullscreen_visible(now);
        let chrome_duration = if state.appearance.reduce_motion {
            Duration::ZERO
        } else {
            Duration::from_millis(180)
        };
        self.fullscreen_motion
            .retarget(f64::from(fullscreen), now, chrome_duration, Ease::Out);
        self.fullscreen_motion.finish_frame(now);
        let chrome_opacity = 1.0 - self.fullscreen_motion.value(now) as f32;
        let top = status::bounds(size.w, scale);
        state.dock_reserve = crate::shell::dock::reserve(size, scale, state.dock_pins.len());
        state.update_overview(now, (size.w, size.h).into(), top.size.h);
        let overview = state.overview_amount(now);
        state.notification_hits = Default::default();
        state
            .notifications
            .borrow_mut()
            .set_private(unlocking > 0.0);
        let panel_amount =
            self.panel_motion
                .amount(state.status_panel, now, state.appearance.reduce_motion);
        let mut panel_bounds = status::panel_bounds(size, top, self.panel_motion.visible);
        panel_bounds.loc.y -= ((1.0 - panel_amount) * 8.0).round() as i32;
        state.status_hits = Some((
            if fullscreen {
                Rectangle::from_size((0, 0).into())
            } else {
                top
            },
            panel_bounds,
        ));
        state.bar_height = Some(top.size.h);
        let solid = state.appearance.reduce_transparency
            || state.appearance.effects == Effects::Reduced
            || self.glass_failed
            || self.glass.is_none();
        let mut status_view = status::View {
            theme: state.appearance.theme,
            solid,
            status: state.status.clone(),
            panel: state.status_panel,
            workspace: state.workspaces.active,
            hovered: state.status_hovered,
            overview: state.overview_open(),
            native: state.native_session,
            notifications: state.notifications_available && unlocking == 0.0,
            error: state.status_error,
        };
        let theme = state.appearance.theme;
        let appearance_amount = self.wallpaper.transition_amount(now);
        if self.label_theme != theme {
            if appearance_amount < 1.0 && !fullscreen {
                let mut previous = status_view.clone();
                previous.theme = self.label_theme;
                let old_bar = self.status.texture(renderer, top.size, &previous)?.clone();
                self.previous_bar = Some((top.size, old_bar));
            }
            self.label_theme = theme;
            self.notice = None;
            self.overview_labels.clear();
        }
        if appearance_amount >= 1.0 {
            self.previous_bar = None;
        }
        let notice = if let Some((name, message, _)) = &state.notice {
            if !self
                .notice
                .as_ref()
                .is_some_and(|(prev, text, _)| prev == name && text == message)
            {
                self.notice = Some((
                    name.clone(),
                    message,
                    notice_texture(renderer, name, message, theme.colors())?,
                ));
            }
            self.notice.as_ref().map(|(_, _, texture)| texture)
        } else {
            None
        };
        let mut windows = state.window_elements(renderer, position, now);
        if panel_amount > 0.0
            && !solid
            && !fullscreen
            && self.panel_motion.visible != Panel::Notifications
        {
            windows.glass.insert(
                0,
                FrostedWindow {
                    behind: 0,
                    bounds: panel_bounds,
                    opacity: panel_amount as f32,
                    radius: status::panel_radius(panel_bounds),
                    theme: state.appearance.theme,
                },
            );
        }
        let glass_scene = if !solid
            && !windows.glass.is_empty()
            && let Some(glass) = &mut self.glass
        {
            match glass.compose(
                renderer,
                &wallpaper,
                size,
                &windows,
                GlassStyle {
                    overview: overview as f32,
                    full_effects: state.appearance.effects == Effects::Full,
                },
            ) {
                Ok(texture) => Some(texture),
                Err(error) => {
                    eprintln!("Could not render frosted surfaces: {error}");
                    self.glass_failed = true;
                    self.previous_bar = None;
                    None
                }
            }
        } else {
            None
        };
        status_view.solid |= self.glass_failed;
        let bar = if chrome_opacity == 0.0 {
            None
        } else {
            Some(
                self.status
                    .texture(renderer, top.size, &status_view)?
                    .clone(),
            )
        };
        let (panel, banner) = {
            let center = state.notifications.borrow();
            let revision = center.revision();
            let panel = if panel_amount <= 0.0 {
                None
            } else if self.panel_motion.visible == Panel::Notifications {
                if unlocking > 0.0 {
                    None
                } else {
                    let pages = center.pages();
                    let page = state.notification_page.min(pages - 1);
                    state.notification_page = page;
                    state.notification_hits.rows = center
                        .visible(page)
                        .map(|entry| (entry.id, entry.actions.len()))
                        .collect();
                    state.notification_hits.revision = revision;
                    state.notification_hits.page = page;
                    state.notification_hits.pages = pages;
                    if self.notification_panel.as_ref().is_none_or(
                        |(old, old_page, old_theme, _)| {
                            (*old, *old_page, *old_theme) != (revision, page, theme)
                        },
                    ) {
                        self.notification_panel = Some((
                            revision,
                            page,
                            theme,
                            notification_ui::panel(renderer, theme, &center, page)?,
                        ));
                    }
                    self.notification_panel
                        .as_ref()
                        .map(|(_, _, _, texture)| texture.clone())
                }
            } else {
                let mut panel_view = status_view.clone();
                panel_view.panel = self.panel_motion.visible;
                Some(self.status.panel(renderer, &panel_view)?.clone())
            };
            let banner = if panel_amount == 0.0
                && !fullscreen
                && overview == 0.0
                && unlocking == 0.0
                && let Some(entry) = center.banner()
            {
                if self
                    .notification_banner
                    .as_ref()
                    .is_none_or(|(old, old_theme, _)| (*old, *old_theme) != (revision, theme))
                {
                    self.notification_banner = Some((
                        revision,
                        theme,
                        notification_ui::banner(renderer, theme, entry)?,
                    ));
                }
                self.notification_banner
                    .as_ref()
                    .map(|(_, _, texture)| texture.clone())
            } else {
                self.notification_banner = None;
                None
            };
            (panel, banner)
        };
        let pointer = state.pointer_location().to_i32_round();
        let cursor_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            if self.cursor.is_some()
                && let CursorImageStatus::Surface(surface) = state.cursor()
            {
                let hotspot = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<CursorImageSurfaceData>()
                        .and_then(|data| data.lock().ok().map(|data| data.hotspot))
                        .unwrap_or_default()
                });
                let origin = pointer - hotspot;
                render_elements_from_surface_tree(
                    renderer,
                    surface,
                    (origin.x, origin.y),
                    1.0,
                    1.0,
                    Kind::Cursor,
                )
            } else {
                Vec::new()
            };
        let (overview_cards, selected) = state.overview_cards();
        if overview > 0.0 {
            let style = (
                state.appearance.theme,
                state.workspaces.active,
                std::array::from_fn(|index| state.workspace_window_count(index)),
            );
            if self
                .overview_controls
                .as_ref()
                .is_none_or(|(cached, _)| *cached != style)
            {
                self.overview_controls = Some((style, overview_controls_texture(renderer, style)?));
            }
        }
        if overview >= 0.92 {
            self.overview_labels
                .retain(|title, _| overview_cards.iter().any(|card| &card.title == title));
            for card in overview_cards {
                if !card.title.is_empty() && !self.overview_labels.contains_key(&card.title) {
                    let texture = overview_label(renderer, &card.title, theme.colors())?;
                    self.overview_labels.insert(card.title.clone(), texture);
                }
            }
            if overview_cards.is_empty() && !self.overview_labels.contains_key("No open windows") {
                let texture = overview_label(renderer, "No open windows", theme.colors())?;
                self.overview_labels
                    .insert("No open windows".into(), texture);
            }
        }
        let unlock_sign = if unlocking > 0.0 {
            Some(
                self.lockscreen
                    .sign_in(
                        renderer,
                        sign_bounds.size.w,
                        state.appearance.theme,
                        state.lock_sign_in(),
                        solid_lock || lock_blur.is_none(),
                    )?
                    .clone(),
            )
        } else {
            None
        };
        let mut frame = renderer.render(target, size, transform)?;
        if fullscreen {
            frame.clear([0.0, 0.0, 0.0, 1.0].into(), &[bounds])?;
            draw_render_elements(&mut frame, 1.0, &windows.elements, &[bounds])?;
        } else if let Some(scene) = glass_scene {
            draw_texture(&mut frame, scene, bounds, bounds)?;
        } else {
            wallpaper::draw(&mut frame, &wallpaper, bounds, 1.0)?;
            dim_wallpaper(&mut frame, bounds, overview as f32)?;
            if status_view.solid {
                let mut remaining = windows.elements.len();
                for surface in windows
                    .glass
                    .iter()
                    .rev()
                    .filter(|surface| surface.behind > 0)
                {
                    let Some(visible) = surface.bounds.intersection(bounds) else {
                        continue;
                    };
                    draw_render_elements(
                        &mut frame,
                        1.0,
                        &windows.elements[surface.behind..remaining],
                        &[bounds],
                    )?;
                    remaining = surface.behind;
                    let alpha = surface.opacity;
                    let (r, g, b) = surface.theme.colors().surface;
                    frame.draw_solid(
                        visible,
                        &[Rectangle::from_size(visible.size)],
                        [r as f32 * alpha, g as f32 * alpha, b as f32 * alpha, alpha].into(),
                    )?;
                }
                draw_render_elements(&mut frame, 1.0, &windows.elements[..remaining], &[bounds])?;
            } else {
                draw_render_elements(&mut frame, 1.0, &windows.elements, &[bounds])?;
            }
        }
        if overview >= 0.92 {
            for (index, card) in overview_cards.iter().enumerate() {
                let thumbnail = card.thumbnail.to_i32_round().to_physical(1);
                if index == selected {
                    let x = thumbnail.loc.x - 3;
                    let y = thumbnail.loc.y - 3;
                    let w = thumbnail.size.w + 6;
                    let h = thumbnail.size.h + 6;
                    for (origin, size) in [
                        ((x, y), (w, 2)),
                        ((x, y + h - 2), (w, 2)),
                        ((x, y + 2), (2, h - 4)),
                        ((x + w - 2, y + 2), (2, h - 4)),
                    ] {
                        let rect = Rectangle::new(origin.into(), size.into());
                        frame.draw_solid(rect, &[Rectangle::from_size(rect.size)], {
                            let (r, g, b) = state.appearance.theme.colors().accent;
                            [r as f32, g as f32, b as f32, 1.0].into()
                        })?;
                    }
                }
                if let Some(label) = self.overview_labels.get(&card.title) {
                    let width = label.size().w.min(thumbnail.size.w);
                    let rect = Rectangle::new(
                        (
                            thumbnail.loc.x + (thumbnail.size.w - width) / 2,
                            thumbnail.loc.y + thumbnail.size.h + 4,
                        )
                            .into(),
                        (width, label.size().h).into(),
                    );
                    draw_texture(&mut frame, label, rect, bounds)?;
                }
            }
            if overview_cards.is_empty()
                && let Some(label) = self.overview_labels.get("No open windows")
            {
                let rect = Rectangle::new(
                    ((size.w - label.size().w) / 2, size.h / 2 - 14).into(),
                    (label.size().w, label.size().h).into(),
                );
                draw_texture(&mut frame, label, rect, bounds)?;
            }
        }
        if overview > 0.0
            && let Some((_, controls)) = &self.overview_controls
        {
            let rect =
                overview::controls_bounds((size.w, size.h).into(), top.size.h).to_physical(1);
            draw_texture_with_opacity(&mut frame, controls, rect, bounds, overview as f32)?;
        }
        for (app, rect, alpha) in state
            .launch_tiles(now)
            .chain(state.opening_tiles(position, now))
            .filter(|_| overview == 0.0 && !fullscreen)
        {
            draw_launch_card(&mut frame, &self.launch_card, bounds, app, rect, alpha)?;
        }
        if let Some(area) = state.snap_preview() {
            frame.draw_solid(
                area,
                &[Rectangle::from_size(area.size)],
                [0.055, 0.12, 0.19, 0.26].into(),
            )?;
            let edges = [
                Rectangle::new((0, 0).into(), (area.size.w, 2).into()),
                Rectangle::new((0, area.size.h - 2).into(), (area.size.w, 2).into()),
                Rectangle::new((0, 0).into(), (2, area.size.h).into()),
                Rectangle::new((area.size.w - 2, 0).into(), (2, area.size.h).into()),
            ];
            frame.draw_solid(area, &edges, [0.35, 0.61, 0.81, 0.90].into())?;
        }
        if let Some(bar) = bar {
            if let Some((old_size, previous)) = &self.previous_bar
                && *old_size == top.size
            {
                draw_texture_with_opacity(&mut frame, previous, top, bounds, chrome_opacity)?;
                draw_texture_with_opacity(
                    &mut frame,
                    &bar,
                    top,
                    bounds,
                    appearance_amount * chrome_opacity,
                )?;
            } else {
                draw_texture_with_opacity(&mut frame, &bar, top, bounds, chrome_opacity)?;
            }
        }
        if let Some(panel) = panel {
            draw_texture_with_opacity(
                &mut frame,
                &panel,
                panel_bounds,
                bounds,
                panel_amount as f32,
            )?;
        }
        if let Some(banner) = banner {
            let width = banner.size().w.min((size.w - 16).max(1));
            let height = (banner.size().h * width / banner.size().w).max(1);
            let position = Rectangle::new(
                ((size.w - width - 8).max(0), top.size.h + 8).into(),
                (width, height).into(),
            );
            draw_texture(&mut frame, &banner, position, bounds)?;
            state.notification_hits.banner = position.intersection(bounds);
        }
        let dock_hits = if chrome_opacity == 0.0 {
            None
        } else {
            self.dock.draw(
                &mut frame,
                &wallpaper,
                bounds,
                DockFrame {
                    scale,
                    now,
                    pressed: state.dock_pressed.as_ref(),
                    pointer: state.pointer_location(),
                    launching: state.dock_launch_intensity(now),
                    running: &running,
                    focused: focused.as_ref(),
                    opacity: chrome_opacity,
                    solid: status_view.solid,
                    reduce_motion: state.appearance.reduce_motion,
                    theme: state.appearance.theme,
                },
            )?
        };
        state.dock_hits = if fullscreen { None } else { dock_hits };
        if let Some(texture) = notice.filter(|_| !fullscreen) {
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
            && let Some(slot) = dock.slot_at(state.pointer_location())
        {
            let texture = self.dock.hint(slot);
            let anchor = dock.anchor(slot);
            let rect = Rectangle::new(
                (
                    (anchor.x.round() as i32 - texture.size().w / 2)
                        .clamp(0, (size.w - texture.size().w).max(0)),
                    dock.top().round() as i32 - texture.size().h - 8,
                )
                    .into(),
                (texture.size().w, texture.size().h).into(),
            );
            draw_texture(&mut frame, texture, rect, bounds)?;
        }
        if let Some(sign) = unlock_sign {
            let shade = unlocking * (1.0 - lock_wallpaper * 0.8);
            wallpaper::draw(
                &mut frame,
                lock_blur.as_ref().unwrap_or(&wallpaper),
                bounds,
                (unlocking - shade) / (1.0 - shade),
            )?;
            frame.draw_solid(bounds, &[bounds], [0.0, 0.0, 0.0, shade].into())?;
            draw_texture_with_opacity(&mut frame, &sign, sign_bounds, bounds, unlocking)?;
        }
        if !state.appearance.reduce_motion {
            let entrance =
                1.0 - Ease::Out.sample(progress(now, self.started, Duration::from_millis(520)));
            if entrance > 0.0 {
                let alpha = 0.88 * entrance as f32;
                frame.draw_solid(
                    bounds,
                    &[bounds],
                    [0.015 * alpha, 0.026 * alpha, 0.051 * alpha, alpha].into(),
                )?;
            }
        }
        if state.display_brightness < 100 {
            let opacity = 1.0 - f32::from(state.display_brightness) / 100.0;
            frame.draw_solid(bounds, &[bounds], [0.0, 0.0, 0.0, opacity].into())?;
        }
        if !cursor_elements.is_empty() {
            draw_render_elements(&mut frame, 1.0, &cursor_elements, &[bounds])?;
        } else if let Some(cursor) = &self.cursor
            && !matches!(state.cursor(), CursorImageStatus::Hidden)
        {
            let target = Rectangle::new((pointer.x, pointer.y).into(), cursor_size.into());
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

fn lock_clock_bounds(
    clock: &GlesTexture,
    bounds: Rectangle<i32, Physical>,
    dim: f32,
    signing_in: f32,
) -> Rectangle<i32, Physical> {
    let scale = (0.86 + 0.14 * dim) * (1.0 - 0.06 * signing_in);
    let w = (clock.size().w as f32 * scale).round().max(1.0) as i32;
    let h = (clock.size().h as f32 * scale).round().max(1.0) as i32;
    let center = bounds.size.h as f32 * (0.33 + 0.085 * dim - 0.035 * signing_in);
    Rectangle::new(
        ((bounds.size.w - w) / 2, center.round() as i32 - h / 2).into(),
        (w, h).into(),
    )
}

pub(crate) fn dim_wallpaper(
    frame: &mut GlesFrame<'_, '_>,
    bounds: Rectangle<i32, Physical>,
    strength: f32,
) -> Result<(), smithay::backend::renderer::gles::GlesError> {
    if strength > 0.0 {
        let alpha = strength * 0.57;
        frame.draw_solid(
            bounds,
            &[bounds],
            [0.03 * alpha, 0.05 * alpha, 0.11 * alpha, alpha].into(),
        )?;
    }
    Ok(())
}

fn overview_label(
    renderer: &mut GlesRenderer,
    title: &str,
    colors: crate::palette::Palette,
) -> Result<GlesTexture, Box<dyn Error>> {
    use vte4::gtk::cairo::FontWeight;
    theme::texture(renderer, (230, 26).into(), |context| {
        theme::glass_surface(context, (1.0, 1.0, 228.0, 24.0), 12.0, colors.base, 0.95)?;
        theme::font(context, 12.0, FontWeight::Bold);
        theme::ink(context, colors.text, 1.0);
        let mut title = title.to_owned();
        if context.text_extents(&title)?.x_advance() > 206.0 {
            while context.text_extents(&title)?.x_advance() > 191.0 && title.pop().is_some() {}
            title.push('…');
        }
        let width = context.text_extents(&title)?.x_advance();
        context.move_to((230.0 - width) / 2.0, 17.5);
        context.show_text(&title)
    })
}

fn draw_texture(
    frame: &mut GlesFrame<'_, '_>,
    texture: &GlesTexture,
    target: Rectangle<i32, Physical>,
    bounds: Rectangle<i32, Physical>,
) -> Result<(), smithay::backend::renderer::gles::GlesError> {
    draw_texture_with_opacity(frame, texture, target, bounds, 1.0)
}

fn draw_texture_with_opacity(
    frame: &mut GlesFrame<'_, '_>,
    texture: &GlesTexture,
    target: Rectangle<i32, Physical>,
    bounds: Rectangle<i32, Physical>,
    opacity: f32,
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
            opacity,
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
    name: &str,
    message: &str,
    colors: crate::palette::Palette,
) -> Result<GlesTexture, Box<dyn Error>> {
    use vte4::gtk::cairo::FontWeight;
    theme::texture(renderer, (376, 68).into(), |context| {
        theme::glass_surface(context, (1.0, 1.0, 374.0, 66.0), 22.0, colors.base, 0.95)?;
        theme::ink(context, colors.warning, 1.0);
        theme::dot(context, 19.0, 24.0, 3.0)?;
        theme::font(context, 14.0, FontWeight::Bold);
        theme::ink(context, colors.text, 1.0);
        context.move_to(31.0, 29.0);
        let name: String = name
            .chars()
            .filter(|ch| !ch.is_control())
            .take(36)
            .collect();
        context.show_text(&format!("Could not open {name}"))?;
        theme::font(context, 12.0, FontWeight::Normal);
        theme::ink(context, colors.muted, 1.0);
        context.move_to(17.0, 51.0);
        context.show_text(message)
    })
}

fn overview_controls_texture(
    renderer: &mut GlesRenderer,
    (theme, active, counts): OverviewStyle,
) -> Result<GlesTexture, Box<dyn Error>> {
    use vte4::gtk::cairo::FontWeight;
    let colors = theme.colors();
    theme::texture(renderer, overview::CONTROL_SIZE.into(), |context| {
        let search = overview::search_control().to_f64();
        theme::rounded(
            context,
            (1.0, 1.0, search.size.w - 2.0, search.size.h - 2.0),
            14.0,
        );
        theme::ink(context, colors.surface, 1.0);
        context.fill_preserve()?;
        theme::ink(context, colors.accent, 0.60);
        context.stroke()?;
        theme::ink(context, colors.text, 1.0);
        theme::font(context, 13.0, FontWeight::Bold);
        context.move_to(15.0, 22.0);
        context.show_text("Search applications")?;
        theme::ink(context, colors.muted, 1.0);
        theme::font(context, 11.0, FontWeight::Normal);
        let shortcut = "Super + S";
        let width = context.text_extents(shortcut)?.x_advance();
        context.move_to(search.size.w - width - 14.0, 22.0);
        context.show_text(shortcut)?;
        for (index, count) in counts.into_iter().enumerate() {
            let slot = overview::workspace_control(index).to_f64();
            let x = slot.loc.x + 1.0;
            let y = slot.loc.y + 1.0;
            let w = slot.size.w - 2.0;
            let h = slot.size.h - 2.0;
            theme::rounded(context, (x, y, w, h), 11.0);
            theme::ink(
                context,
                if index == active {
                    colors.selected
                } else {
                    colors.surface
                },
                1.0,
            );
            context.fill_preserve()?;
            theme::ink(
                context,
                colors.accent,
                if index == active { 1.0 } else { 0.42 },
            );
            context.stroke()?;
            theme::font(context, 13.0, FontWeight::Bold);
            theme::ink(context, colors.text, 1.0);
            context.move_to(x + 12.0, y + 22.0);
            context.show_text(&format!("Workspace {}", index + 1))?;
            theme::font(context, 11.0, FontWeight::Normal);
            theme::ink(context, colors.muted, 1.0);
            context.move_to(x + 12.0, y + 40.0);
            context.show_text(&format!(
                "{count} {}",
                if count == 1 { "window" } else { "windows" }
            ))?;
        }
        Ok(())
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
