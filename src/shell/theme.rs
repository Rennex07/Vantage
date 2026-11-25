use std::{
    error::Error,
    f64::consts::{FRAC_PI_2, PI, TAU},
};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ImportMem,
            gles::{GlesRenderer, GlesTexture},
        },
    },
    utils::{Physical, Size},
};
use vte4::gtk::cairo::{Context, Error as CairoError, FontSlant, FontWeight, Format, ImageSurface};

pub const INK: (f64, f64, f64) = (0.051, 0.086, 0.161);
pub const GLASS: (f64, f64, f64) = (0.106, 0.173, 0.302);
pub const BLUE: (f64, f64, f64) = (0.569, 0.749, 0.980);
pub const AMBER: (f64, f64, f64) = (0.925, 0.718, 0.486);
pub const TEXT: (f64, f64, f64) = (0.933, 0.957, 1.0);
pub const MUTED: (f64, f64, f64) = (0.682, 0.753, 0.855);

pub fn ink(context: &Context, (r, g, b): (f64, f64, f64), alpha: f64) {
    context.set_source_rgba(r, g, b, alpha);
}

pub fn rounded(context: &Context, (x, y, w, h): (f64, f64, f64, f64), radius: f64) {
    let r = radius.min(w / 2.0).min(h / 2.0);
    context.new_sub_path();
    context.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    context.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    context.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    context.arc(x + r, y + r, r, PI, PI + FRAC_PI_2);
    context.close_path();
}

pub fn card(context: &Context, width: f64, height: f64, radius: f64) -> Result<(), CairoError> {
    rounded(context, (1.0, 1.0, width - 2.0, height - 2.0), radius);
    ink(context, INK, 0.98);
    context.fill_preserve()?;
    ink(context, BLUE, 0.40);
    context.set_line_width(1.0);
    context.stroke()
}

pub fn font(context: &Context, size: f64, weight: FontWeight) {
    context.select_font_face("sans-serif", FontSlant::Normal, weight);
    context.set_font_size(size);
}

pub fn dot(context: &Context, x: f64, y: f64, radius: f64) -> Result<(), CairoError> {
    context.new_sub_path();
    context.arc(x, y, radius, 0.0, TAU);
    context.fill()
}

pub fn texture(
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
    paint: impl FnOnce(&Context) -> Result<(), CairoError>,
) -> Result<GlesTexture, Box<dyn Error>> {
    let mut image = ImageSurface::create(Format::ARgb32, size.w, size.h)?;
    paint(&Context::new(&image)?)?;
    Ok(renderer.import_memory(
        &image.data()?,
        Fourcc::Argb8888,
        (size.w, size.h).into(),
        false,
    )?)
}
