use std::{error::Error, path::Path};

use smithay::{
    backend::renderer::gles::{GlesRenderer, GlesTexture},
    utils::{Physical, Size},
};
use vte4::gtk::cairo::{Context, Error as CairoError, FontWeight, Format, ImageSurface};

use super::{status::Status, theme};

type ClockKey = (String, String, i32);

#[derive(Default)]
pub struct LockScreen {
    clock: Option<(ClockKey, GlesTexture)>,
    hint: Option<GlesTexture>,
}

impl LockScreen {
    pub fn clock(
        &mut self,
        renderer: &mut GlesRenderer,
        output: Size<i32, Physical>,
        status: &Status,
    ) -> Result<&GlesTexture, Box<dyn Error>> {
        let width = (output.w.min(output.h * 3 / 2) - 24).clamp(1, 560);
        let key = (status.clock.clone(), status.date.clone(), width);
        if self.clock.as_ref().is_none_or(|(old, _)| *old != key) {
            let texture =
                theme::texture(renderer, (width, (width * 7 / 20).max(1)).into(), |ctx| {
                    let width = f64::from(width);
                    let scale = width / 560.0;
                    ctx.scale(scale, scale);
                    theme::display_font(ctx, 142.0);
                    // A low shadow keeps the same clock legible in the light and dark scenes.
                    theme::ink(ctx, (0.01, 0.025, 0.07), 0.65);
                    centered(ctx, &status.clock, 280.0, 132.0)?;
                    theme::ink(ctx, (0.93, 0.96, 1.0), 1.0);
                    centered(ctx, &status.clock, 280.0, 128.0)?;
                    theme::font(ctx, 23.0, FontWeight::Bold);
                    theme::ink(ctx, (0.87, 0.92, 0.98), 1.0);
                    centered(ctx, &status.date, 280.0, 174.0)
                })?;
            self.clock = Some((key, texture));
        }
        Ok(&self.clock.as_ref().expect("lock clock rendered").1)
    }

    pub fn hint(&mut self, renderer: &mut GlesRenderer) -> Result<&GlesTexture, Box<dyn Error>> {
        if self.hint.is_none() {
            self.hint = Some(theme::texture(renderer, (250, 42).into(), |ctx| {
                theme::rounded(ctx, (1.0, 1.0, 248.0, 40.0), 20.0);
                let frost = vte4::gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, 42.0);
                frost.add_color_stop_rgba(0.0, 0.78, 0.89, 1.0, 0.58);
                frost.add_color_stop_rgba(0.22, 0.16, 0.24, 0.35, 0.94);
                frost.add_color_stop_rgba(1.0, 0.05, 0.10, 0.19, 0.89);
                ctx.set_source(frost)?;
                ctx.fill_preserve()?;
                theme::ink(ctx, (0.91, 0.96, 1.0), 0.67);
                ctx.stroke()?;
                theme::font(ctx, 14.0, FontWeight::Bold);
                theme::ink(ctx, (0.95, 0.97, 1.0), 1.0);
                centered(ctx, "Type your password to unlock", 125.0, 26.0)
            })?);
        }
        Ok(self.hint.as_ref().expect("lock hint rendered"))
    }
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
