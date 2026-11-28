use std::{
    error::Error,
    path::Path,
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ImportMem, Texture,
            gles::{
                GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform,
                UniformName, UniformType,
            },
        },
    },
    utils::{Logical, Physical, Point, Rectangle, Transform},
};

use vte4::gtk::{cairo::FontWeight, gdk_pixbuf::Pixbuf};

use crate::{palette::Theme, preferences::DockPin};

use super::{
    launch::{self, App, Target},
    theme,
    timeline::{Ease, progress},
};

const REVEAL: Duration = Duration::from_millis(336);
pub const DOCK_RESERVE: i32 = 86;
const BASE_WIDTH: f64 = 146.0;
const HEIGHT: f64 = 72.0;
const SLOT_WIDTH: f64 = 62.0;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Launch(Target),
    Add,
}

pub struct DockHits {
    panel: Rectangle<f64, Logical>,
    width: f64,
    actions: Vec<Action>,
}

impl DockHits {
    pub fn contains(&self, point: smithay::utils::Point<f64, Logical>) -> bool {
        self.panel.contains(point)
    }

    pub fn anchor(&self, slot: usize) -> Point<f64, Logical> {
        self.panel.loc
            + Point::from((
                (42.0 + SLOT_WIDTH * slot as f64) * self.panel.size.w / self.width,
                35.0 * self.panel.size.h / HEIGHT,
            ))
    }

    pub fn slot_at(&self, point: Point<f64, Logical>) -> Option<usize> {
        let x = (point.x - self.panel.loc.x) * self.width / self.panel.size.w;
        let y = (point.y - self.panel.loc.y) * HEIGHT / self.panel.size.h;
        if (9.0..63.0).contains(&y) {
            for slot in 0..self.actions.len() {
                let center = 42.0 + SLOT_WIDTH * slot as f64;
                if (center - 28.0..center + 28.0).contains(&x) {
                    return Some(slot);
                }
            }
        }
        None
    }

    pub fn action_at(&self, point: Point<f64, Logical>) -> Option<&Action> {
        self.slot_at(point).map(|slot| &self.actions[slot])
    }
}

pub struct DockFrame<'a> {
    pub scale: f64,
    pub now: Instant,
    pub pressed: Option<&'a Action>,
    pub pointer: Point<f64, Logical>,
    pub launching: (f32, f32),
    pub solid: bool,
    pub reduce_motion: bool,
    pub theme: Theme,
}

pub struct Dock {
    shader: GlesTexProgram,
    started: Option<Instant>,
    pins: Vec<DockPin>,
    icons: Vec<GlesTexture>,
    hints: Vec<GlesTexture>,
    add_hint: GlesTexture,
}

impl Dock {
    pub fn new(renderer: &mut GlesRenderer) -> Result<Self, Box<dyn Error>> {
        let shader = renderer.compile_custom_texture_shader(
            include_str!("dock.frag"),
            &[
                UniformName::new("wallpaper_origin", UniformType::_2f),
                UniformName::new("wallpaper_step", UniformType::_2f),
                UniformName::new("panel_width", UniformType::_1f),
                UniformName::new("surface_color", UniformType::_3f),
                UniformName::new("glass_tint", UniformType::_3f),
                UniformName::new("pressed", UniformType::_1f),
                UniformName::new("hovered", UniformType::_1f),
                UniformName::new("launching", UniformType::_2f),
                UniformName::new("solid", UniformType::_1f),
                UniformName::new("reduce_motion", UniformType::_1f),
            ],
        )?;
        Ok(Self {
            shader,
            started: Some(Instant::now()),
            pins: Vec::new(),
            icons: Vec::new(),
            hints: vec![hint(renderer, "Console")?, hint(renderer, "Files")?],
            add_hint: hint(renderer, "Add apps")?,
        })
    }

    pub fn sync_pins(
        &mut self,
        renderer: &mut GlesRenderer,
        pins: &[DockPin],
    ) -> Result<(), Box<dyn Error>> {
        if self.pins == pins {
            return Ok(());
        }
        let mut icons = Vec::new();
        let mut hints = Vec::new();
        for pin in pins {
            let name = launch::desktop_name(&pin.id);
            let icon = pin
                .icon
                .as_deref()
                .and_then(|path| load_icon(renderer, path));
            icons.push(match icon {
                Some(texture) => texture,
                None => fallback_icon(renderer, &name)?,
            });
            hints.push(hint(renderer, &name)?);
        }
        self.pins = pins.to_vec();
        self.icons = icons;
        self.hints.truncate(App::ALL.len());
        self.hints.extend(hints);
        Ok(())
    }

    pub fn hint(&self, slot: usize) -> &GlesTexture {
        self.hints.get(slot).unwrap_or(&self.add_hint)
    }

    pub fn is_animating(&self) -> bool {
        self.started.is_some()
    }

    pub fn reveal(&self, now: Instant) -> f64 {
        self.started
            .map_or(1.0, |start| Ease::Out.sample(progress(now, start, REVEAL)))
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
        view: DockFrame<'_>,
    ) -> Result<Option<DockHits>, GlesError> {
        let reveal = if view.reduce_motion {
            1.0
        } else {
            self.reveal(view.now)
        };
        let panel_width = BASE_WIDTH + (self.pins.len() + 1) as f64 * SLOT_WIDTH;
        let scale = view
            .scale
            .min(bounds.size.w as f64 / (panel_width + 24.0))
            .min(bounds.size.h as f64 / (HEIGHT + 36.0));
        let width = (panel_width * scale).round() as i32;
        let height = (HEIGHT * scale).round() as i32;
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
            (region.size.w / panel_width / texture_size.w) as f32,
            (region.size.h / HEIGHT / texture_size.h) as f32,
        );
        // The entrance can start below the output. Damage is local to the panel.
        let Some(visible) = panel.intersection(bounds) else {
            return Ok(None);
        };
        let hits = DockHits {
            panel: panel.to_f64().to_logical(1.0),
            width: panel_width,
            actions: App::ALL
                .into_iter()
                .map(|app| Action::Launch(Target::BuiltIn(app)))
                .chain(
                    self.pins
                        .iter()
                        .map(|pin| Action::Launch(Target::Desktop(pin.id.clone()))),
                )
                .chain(std::iter::once(Action::Add))
                .collect(),
        };
        let pressed = view
            .pressed
            .and_then(|pressed| hits.actions.iter().position(|action| action == pressed));
        let hovered = hits.slot_at(view.pointer);
        let colors = view.theme.colors();
        let gpu = |(r, g, b)| (r as f32, g as f32, b as f32);
        let glass_tint = crate::palette::mix(colors.surface, colors.accent, 0.15);
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
                Uniform::new("panel_width", panel_width as f32),
                Uniform::new("surface_color", gpu(colors.surface)),
                Uniform::new("glass_tint", gpu(glass_tint)),
                Uniform::new("pressed", pressed.map_or(-1.0, |slot| slot as f32)),
                Uniform::new("hovered", hovered.map_or(-1.0, |slot| slot as f32)),
                Uniform::new("launching", view.launching),
                Uniform::new("solid", if view.solid { 1.0_f32 } else { 0.0_f32 }),
                Uniform::new(
                    "reduce_motion",
                    if view.reduce_motion { 1.0_f32 } else { 0.0_f32 },
                ),
            ],
        )?;
        for (index, icon) in self.icons.iter().enumerate() {
            let slot = App::ALL.len() + index;
            let mut center = hits.anchor(slot);
            let mut icon_scale = scale;
            if !view.reduce_motion {
                if hovered == Some(slot) {
                    center.y -= 2.0 * scale;
                }
                if pressed == Some(slot) {
                    center.y += 2.0 * scale;
                    icon_scale *= 0.9;
                }
            }
            let ratio = icon_scale * 30.0 / f64::from(icon.size().w.max(icon.size().h));
            let size = icon.size().to_f64().upscale(ratio).to_i32_round();
            let destination = Rectangle::new(
                (
                    center.x.round() as i32 - size.w / 2,
                    center.y.round() as i32 - size.h / 2,
                )
                    .into(),
                (size.w, size.h).into(),
            );
            if let Some(visible) = destination.intersection(bounds) {
                frame.render_texture_from_to(
                    icon,
                    Rectangle::from_size(icon.size().to_f64()),
                    destination,
                    &[Rectangle::new(visible.loc - destination.loc, visible.size)],
                    &[],
                    Transform::Normal,
                    reveal as f32,
                    None,
                    &[],
                )?;
            }
        }
        Ok(Some(hits))
    }
}

fn load_icon(renderer: &mut GlesRenderer, path: &Path) -> Option<GlesTexture> {
    if !path.is_file() {
        return None;
    }
    let icon = Pixbuf::from_file_at_scale(path, 96, 96, true).ok()?;
    let (width, height) = (icon.width() as usize, icon.height() as usize);
    let (stride, channels) = (icon.rowstride() as usize, icon.n_channels() as usize);
    let pixels = icon.read_pixel_bytes();
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in (0..height).map(|index| &pixels[index * stride..index * stride + width * channels]) {
        for pixel in row.chunks_exact(channels) {
            let alpha = pixel.get(3).copied().unwrap_or(255);
            rgba.extend(
                pixel[..3]
                    .iter()
                    .map(|value| ((u16::from(*value) * u16::from(alpha) + 127) / 255) as u8),
            );
            rgba.push(alpha);
        }
    }
    renderer
        .import_memory(
            &rgba,
            Fourcc::Abgr8888,
            (icon.width(), icon.height()).into(),
            false,
        )
        .ok()
}

fn fallback_icon(renderer: &mut GlesRenderer, name: &str) -> Result<GlesTexture, Box<dyn Error>> {
    let letter = name
        .chars()
        .find(|ch| ch.is_alphanumeric())
        .map_or_else(|| "?".into(), |ch| ch.to_uppercase().collect::<String>());
    theme::texture(renderer, (32, 32).into(), |context| {
        theme::card(context, 32.0, 32.0, 9.0)?;
        theme::font(context, 16.0, FontWeight::Bold);
        theme::ink(context, theme::TEXT, 1.0);
        let width = context.text_extents(&letter)?.x_advance();
        context.move_to((32.0 - width) / 2.0, 22.0);
        context.show_text(&letter)
    })
}

fn hint(renderer: &mut GlesRenderer, name: &str) -> Result<GlesTexture, Box<dyn Error>> {
    let name: String = name.chars().filter(|ch| !ch.is_control()).collect();
    let width = 18 + 10 * name.chars().count().min(30) as i32;
    theme::texture(renderer, (width, 30).into(), |context| {
        theme::card(context, f64::from(width), 30.0, 10.0)?;
        theme::font(context, 12.0, FontWeight::Bold);
        theme::ink(context, theme::TEXT, 1.0);
        let mut label: String = name.chars().take(27).collect();
        if label.len() != name.len() {
            label.push('…');
        }
        let label_width = context.text_extents(&label)?.x_advance();
        context.move_to(((f64::from(width) - label_width) / 2.0).max(4.0), 19.5);
        context.show_text(&label)
    })
}
