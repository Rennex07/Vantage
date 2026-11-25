use std::time::{Duration, Instant};

use smithay::{
    backend::renderer::{
        Texture,
        gles::{
            GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName,
            UniformType,
        },
    },
    utils::{Logical, Physical, Point, Rectangle, Transform},
};

use super::launch::App;

const REVEAL: Duration = Duration::from_millis(336);
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const SIZE: (f64, f64) = (300.0, 72.0);
const WORKSPACE_FIRST: f64 = 176.0;
const WORKSPACE_PITCH: f64 = 42.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DockAction {
    App(App),
    Workspace(usize),
}

impl DockAction {
    fn shader_index(self) -> f32 {
        match self {
            Self::App(app) => app.index() as f32,
            Self::Workspace(index) => (index + App::ALL.len()) as f32,
        }
    }
}

pub struct DockHits {
    panel: Rectangle<f64, Logical>,
}

impl DockHits {
    pub fn contains(&self, point: smithay::utils::Point<f64, Logical>) -> bool {
        self.panel.contains(point)
    }

    pub fn anchor(&self, app: App) -> Point<f64, Logical> {
        self.panel.loc
            + Point::from((
                (42.0 + 62.0 * app.index() as f64) * self.panel.size.w / SIZE.0,
                35.0 * self.panel.size.h / SIZE.1,
            ))
    }

    pub fn action_at(&self, point: Point<f64, Logical>) -> Option<DockAction> {
        let x = (point.x - self.panel.loc.x) * SIZE.0 / self.panel.size.w;
        let y = (point.y - self.panel.loc.y) * SIZE.1 / self.panel.size.h;
        if (9.0..63.0).contains(&y) {
            for app in App::ALL {
                let center = 42.0 + 62.0 * app.index() as f64;
                if (center - 28.0..center + 28.0).contains(&x) {
                    return Some(DockAction::App(app));
                }
            }
        }
        if (17.0..55.0).contains(&y) {
            for index in 0..super::workspaces::WORKSPACE_COUNT {
                let center = WORKSPACE_FIRST + index as f64 * WORKSPACE_PITCH;
                if (center - 19.0..center + 19.0).contains(&x) {
                    return Some(DockAction::Workspace(index));
                }
            }
        }
        None
    }
}

pub struct DockFrame {
    pub scale: f64,
    pub now: Instant,
    pub pressed: Option<DockAction>,
    pub pointer: Point<f64, Logical>,
    pub launching: (f32, f32),
    pub workspace_position: f64,
}

pub struct Dock {
    shader: GlesTexProgram,
    started: Option<Instant>,
}

impl Dock {
    pub fn new(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let shader = renderer.compile_custom_texture_shader(
            include_str!("dock.frag"),
            &[
                UniformName::new("wallpaper_origin", UniformType::_2f),
                UniformName::new("wallpaper_step", UniformType::_2f),
                UniformName::new("pressed", UniformType::_1f),
                UniformName::new("hovered", UniformType::_1f),
                UniformName::new("launching", UniformType::_2f),
                UniformName::new("workspace_position", UniformType::_1f),
            ],
        )?;
        Ok(Self {
            shader,
            started: Some(Instant::now()),
        })
    }

    pub fn is_animating(&self) -> bool {
        self.started.is_some()
    }

    pub fn reveal(&self, now: Instant) -> f64 {
        self.started.map_or(1.0, |start| {
            let t = super::workspaces::progress(now, start, REVEAL);
            1.0 - (1.0 - t).powi(3)
        })
    }

    pub fn finish_frame(&mut self, now: Instant) {
        if self.reveal(now) >= 1.0 {
            self.started = None;
        }
    }

    pub fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        wallpaper: &GlesTexture,
        bounds: Rectangle<i32, Physical>,
        view: DockFrame,
    ) -> Result<Option<DockHits>, GlesError> {
        let reveal = self.reveal(view.now);
        let scale = view
            .scale
            .min(bounds.size.w as f64 / (SIZE.0 + 24.0))
            .min(bounds.size.h as f64 / (SIZE.1 + 36.0));
        let width = (SIZE.0 * scale).round() as i32;
        let height = (SIZE.1 * scale).round() as i32;
        let bottom = ((12.0 - 30.0 * (1.0 - reveal)) * scale).round() as i32;
        if width == 0 || height == 0 || reveal == 0.0 {
            return Ok(None);
        }
        let panel = Rectangle::new(
            ((bounds.size.w - width) / 2, bounds.size.h - height - bottom).into(),
            (width, height).into(),
        );

        let source = crate::wallpaper::cover(wallpaper.size(), bounds.size);
        let pixels_per_screen_pixel = (
            source.size.w / f64::from(bounds.size.w),
            source.size.h / f64::from(bounds.size.h),
        );
        let region = Rectangle::new(
            (
                source.loc.x + f64::from(panel.loc.x) * pixels_per_screen_pixel.0,
                source.loc.y + f64::from(panel.loc.y) * pixels_per_screen_pixel.1,
            )
                .into(),
            (
                f64::from(panel.size.w) * pixels_per_screen_pixel.0,
                f64::from(panel.size.h) * pixels_per_screen_pixel.1,
            )
                .into(),
        );
        let texture_size = wallpaper.size().to_f64();
        let origin = (
            (region.loc.x / texture_size.w) as f32,
            (region.loc.y / texture_size.h) as f32,
        );
        let step = (
            (region.size.w / SIZE.0 / texture_size.w) as f32,
            (region.size.h / SIZE.1 / texture_size.h) as f32,
        );
        // The entrance can start below the output. Damage is local to the panel.
        let Some(visible) = panel.intersection(bounds) else {
            return Ok(None);
        };
        let hits = DockHits {
            panel: panel.to_f64().to_logical(1.0),
        };
        frame.render_texture_from_to(
            wallpaper,
            region,
            panel,
            &[Rectangle::new(visible.loc - panel.loc, visible.size)],
            &[],
            Transform::Normal,
            reveal as f32,
            Some(&self.shader),
            &[
                Uniform::new("wallpaper_origin", origin),
                Uniform::new("wallpaper_step", step),
                Uniform::new(
                    "pressed",
                    view.pressed.map_or(-1.0, DockAction::shader_index),
                ),
                Uniform::new(
                    "hovered",
                    hits.action_at(view.pointer)
                        .map_or(-1.0, DockAction::shader_index),
                ),
                Uniform::new("launching", view.launching),
                Uniform::new("workspace_position", view.workspace_position as f32),
            ],
        )?;
        Ok(Some(hits))
    }
}
