use std::{error::Error, path::Path};

use smithay::{
    backend::renderer::gles::{GlesRenderer, GlesTexture},
    utils::{Logical, Physical, Point, Rectangle, Size},
};
use vte4::gtk::cairo::{Context, Error as CairoError, FontWeight, Format, ImageSurface, LineCap};

use super::{status::Status, theme};
use crate::palette::{Mode, Theme, mix};

type ClockKey = (String, String, i32);
const SIGN_WIDTH: i32 = 388;
const SIGN_HEIGHT: i32 = 250;
const FIELD: (f64, f64, f64, f64) = (34.0, 160.0, 320.0, 56.0);
const SUBMIT_X: f64 = 325.0;
const SUBMIT_Y: f64 = 188.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SignInStatus {
    #[default]
    Ready,
    Checking,
    Incorrect,
    Unavailable,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SignIn<'a> {
    pub user: &'a str,
    pub dots: usize,
    pub status: SignInStatus,
    pub caps_lock: bool,
}

#[derive(PartialEq, Eq)]
struct SignKey {
    user: String,
    dots: usize,
    status: SignInStatus,
    caps_lock: bool,
    colors: Theme,
    width: i32,
    solid: bool,
}

#[derive(Default)]
pub struct LockScreen {
    clock: Option<(ClockKey, GlesTexture)>,
    hint: Option<(Theme, GlesTexture)>,
    sign: Option<(SignKey, GlesTexture)>,
}

impl LockScreen {
    pub fn clock(
        &mut self,
        renderer: &mut GlesRenderer,
        output: Size<i32, Physical>,
        status: &Status,
    ) -> Result<&GlesTexture, Box<dyn Error>> {
        let preferred = (output.h * 9 / 10).clamp(1, 1440);
        let width = (output.w - 24).clamp(1, preferred);
        let key = (status.clock.clone(), status.date.clone(), width);
        if self.clock.as_ref().is_none_or(|(old, _)| *old != key) {
            let texture =
                theme::texture(renderer, (width, (width * 7 / 20).max(1)).into(), |ctx| {
                    let scale = f64::from(width) / 560.0;
                    ctx.scale(scale, scale);
                    theme::display_font(ctx, 146.0);
                    theme::ink(ctx, (0.04, 0.035, 0.055), 0.43);
                    centered(ctx, &status.clock, 280.0, 132.0)?;
                    theme::ink(ctx, (0.98, 0.98, 1.0), 1.0);
                    centered(ctx, &status.clock, 280.0, 128.0)?;
                    theme::font(ctx, 19.0, FontWeight::Bold);
                    theme::ink(ctx, (0.96, 0.96, 1.0), 1.0);
                    centered(ctx, &status.date, 280.0, 171.0)
                })?;
            self.clock = Some((key, texture));
        }
        Ok(&self.clock.as_ref().expect("lock clock rendered").1)
    }

    pub fn hint(
        &mut self,
        renderer: &mut GlesRenderer,
        colors: Theme,
    ) -> Result<&GlesTexture, Box<dyn Error>> {
        if self.hint.as_ref().is_none_or(|(old, _)| *old != colors) {
            let tint = colors.colors().surface;
            let texture = theme::texture(renderer, (184, 44).into(), |ctx| {
                theme::glass_surface(ctx, (2.0, 2.0, 180.0, 40.0), 20.0, tint, 0.72)?;
                theme::font(ctx, 13.0, FontWeight::Bold);
                theme::ink(ctx, (1.0, 1.0, 1.0), 1.0);
                centered(ctx, "Press a key to unlock", 96.0, 27.0)?;
                ctx.set_line_width(1.4);
                ctx.set_line_cap(LineCap::Round);
                ctx.move_to(22.0, 24.0);
                ctx.line_to(26.0, 20.0);
                ctx.line_to(30.0, 24.0);
                ctx.stroke()
            })?;
            self.hint = Some((colors, texture));
        }
        Ok(&self.hint.as_ref().expect("lock hint rendered").1)
    }

    pub fn sign_in(
        &mut self,
        renderer: &mut GlesRenderer,
        width: i32,
        colors: Theme,
        sign: SignIn<'_>,
        solid: bool,
    ) -> Result<&GlesTexture, Box<dyn Error>> {
        let key = SignKey {
            user: sign.user.to_owned(),
            dots: sign.dots,
            status: sign.status,
            caps_lock: sign.caps_lock,
            colors,
            width,
            solid,
        };
        if self.sign.as_ref().is_none_or(|(old, _)| *old != key) {
            let height = sign_height(width);
            let texture = theme::texture(renderer, (width, height).into(), |ctx| {
                let scale = f64::from(width) / f64::from(SIGN_WIDTH);
                ctx.scale(scale, scale);
                paint_sign_in(ctx, colors, sign, solid)
            })?;
            self.sign = Some((key, texture));
        }
        Ok(&self.sign.as_ref().expect("sign-in rendered").1)
    }
}

fn sign_height(width: i32) -> i32 {
    (width * SIGN_HEIGHT / SIGN_WIDTH).max(1)
}

pub fn sign_bounds(width: i32, height: i32) -> Rectangle<i32, Physical> {
    let w = (height * SIGN_WIDTH / 800)
        .clamp(300, 940)
        .min((width - 24).max(1));
    let h = sign_height(w);
    Rectangle::new(((width - w) / 2, (height - h) / 2).into(), (w, h).into())
}

pub fn submit_contains(width: i32, height: i32, point: Point<f64, Logical>) -> bool {
    let bounds = sign_bounds(width, height);
    let scale = f64::from(bounds.size.w) / f64::from(SIGN_WIDTH);
    let x = (point.x - f64::from(bounds.loc.x)) / scale;
    let y = (point.y - f64::from(bounds.loc.y)) / scale;
    (x - SUBMIT_X).hypot(y - SUBMIT_Y) <= 23.0
}

fn paint_sign_in(
    ctx: &Context,
    colors: Theme,
    sign: SignIn<'_>,
    solid: bool,
) -> Result<(), CairoError> {
    let palette = colors.colors();
    let light = colors.mode == Mode::Light;
    let frost = if light {
        mix(palette.surface, (0.51, 0.43, 0.36), 0.20)
    } else {
        mix(palette.surface, palette.accent, 0.11)
    };
    let contrast = if light {
        (0.17, 0.13, 0.12)
    } else {
        (0.98, 0.98, 1.0)
    };
    let card_opacity = if solid {
        1.0
    } else if light {
        0.83
    } else {
        0.70
    };
    theme::glass_surface(ctx, (2.0, 2.0, 384.0, 246.0), 37.0, frost, card_opacity)?;

    let center = f64::from(SIGN_WIDTH) / 2.0;
    let badge = mix(frost, palette.accent, if light { 0.28 } else { 0.24 });
    theme::glass_surface(ctx, (center - 33.0, 27.0, 66.0, 66.0), 30.0, badge, 0.79)?;
    theme::ink(ctx, contrast, 0.95);
    ctx.set_line_width(2.0);
    theme::dot(ctx, center, 55.0, 6.0)?;
    ctx.move_to(center - 12.0, 75.0);
    ctx.curve_to(
        center - 11.0,
        64.0,
        center + 11.0,
        64.0,
        center + 12.0,
        75.0,
    );
    ctx.set_line_cap(LineCap::Round);
    ctx.stroke()?;

    theme::font(ctx, 28.0, FontWeight::Bold);
    theme::ink(ctx, contrast, 1.0);
    ctx.save()?;
    ctx.rectangle(24.0, 98.0, 340.0, 48.0);
    ctx.clip();
    centered(ctx, sign.user, center, 130.0)?;
    ctx.restore()?;

    let field_color = mix(frost, contrast, if light { 0.10 } else { 0.05 });
    theme::glass_surface(ctx, FIELD, 28.0, field_color, 0.94)?;
    if sign.dots == 0 {
        theme::font(ctx, 16.0, FontWeight::Normal);
        theme::ink(ctx, contrast, 0.80);
        ctx.move_to(56.0, 194.0);
        ctx.show_text("Password")?;
    } else {
        theme::ink(ctx, contrast, 1.0);
        for index in 0..sign.dots {
            theme::dot(ctx, 58.0 + 8.3 * index as f64, 188.0, 2.6)?;
            ctx.fill()?;
        }
    }

    let button = mix(frost, palette.accent, if light { 0.38 } else { 0.29 });
    theme::glass_surface(
        ctx,
        (SUBMIT_X - 20.0, SUBMIT_Y - 20.0, 40.0, 40.0),
        20.0,
        button,
        1.0,
    )?;
    theme::ink(
        ctx,
        contrast,
        if sign.status == SignInStatus::Checking {
            0.45
        } else {
            0.95
        },
    );
    ctx.set_line_width(1.8);
    ctx.move_to(SUBMIT_X - 7.0, SUBMIT_Y);
    ctx.line_to(SUBMIT_X + 6.5, SUBMIT_Y);
    ctx.move_to(SUBMIT_X + 0.5, SUBMIT_Y - 6.0);
    ctx.line_to(SUBMIT_X + 6.5, SUBMIT_Y);
    ctx.line_to(SUBMIT_X + 0.5, SUBMIT_Y + 6.0);
    ctx.stroke()?;

    let (label, ink) = match sign.status {
        SignInStatus::Incorrect => (
            "Incorrect password",
            if light {
                (0.46, 0.07, 0.11)
            } else {
                (1.0, 0.72, 0.74)
            },
        ),
        SignInStatus::Unavailable => ("Could not verify the password", contrast),
        SignInStatus::Checking => ("Checking…", contrast),
        SignInStatus::Ready if sign.caps_lock => ("Caps Lock is on", contrast),
        SignInStatus::Ready => return Ok(()),
    };
    theme::font(ctx, 12.0, FontWeight::Bold);
    theme::ink(ctx, ink, 1.0);
    centered(ctx, label, center, 235.0)
}

fn centered(ctx: &Context, text: &str, x: f64, y: f64) -> Result<(), CairoError> {
    let extents = ctx.text_extents(text)?;
    ctx.move_to(x - extents.x_bearing() - extents.width() / 2.0, y);
    ctx.show_text(text)
}

pub fn boot_wordmark(path: &Path) -> Result<(), Box<dyn Error>> {
    const WIDTH: u32 = 420;
    const HEIGHT: u32 = 106;
    let mut surface = ImageSurface::create(Format::ARgb32, WIDTH as i32, HEIGHT as i32)?;
    {
        let ctx = Context::new(&surface)?;
        theme::display_font(&ctx, 72.0);
        theme::ink(&ctx, (0.91, 0.95, 1.0), 1.0);
        centered(&ctx, "VANTAGE", f64::from(WIDTH) / 2.0, 78.0)?;
    }
    let pixels = surface.data()?;
    let rgba = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| {
            let argb = u32::from_ne_bytes(*pixel);
            let [a, r, g, b] = argb.to_be_bytes();
            let straight = |value: u8| (u16::from(value) * 255 / u16::from(a.max(1))) as u8;
            [straight(r), straight(g), straight(b), a]
        })
        .collect::<Vec<_>>();
    image::save_buffer(path, &rgba, WIDTH, HEIGHT, image::ColorType::Rgba8)?;
    Ok(())
}
