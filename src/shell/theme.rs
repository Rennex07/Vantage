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

pub const INK: (f64, f64, f64) = (0.025, 0.044, 0.102);
pub const BLUE: (f64, f64, f64) = (0.569, 0.749, 0.980);
pub const AMBER: (f64, f64, f64) = (0.925, 0.718, 0.486);
pub const TEXT: (f64, f64, f64) = (0.933, 0.957, 1.0);

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
    draw_card(context, width, height, radius, 1.0)
}

pub fn draw_card(
    context: &Context,
    width: f64,
    height: f64,
    radius: f64,
    opacity: f64,
) -> Result<(), CairoError> {
    paint_card(context, width, height, radius, opacity, false)
}

pub fn glass_surface(
    context: &Context,
    bounds: (f64, f64, f64, f64),
    radius: f64,
    tint: crate::palette::Color,
    opacity: f64,
) -> Result<(), CairoError> {
    let (x, y, w, h) = bounds;
    let rim = crate::palette::mix(tint, (1.0, 0.99, 0.98), 0.70);
    let shoulder = crate::palette::mix(tint, (1.0, 1.0, 1.0), 0.18);
    let body = vte4::gtk::cairo::LinearGradient::new(x, y, x, y + h);
    body.add_color_stop_rgba(0.0, rim.0, rim.1, rim.2, opacity);
    body.add_color_stop_rgba(
        (6.0 / h).min(0.20),
        shoulder.0,
        shoulder.1,
        shoulder.2,
        opacity,
    );
    body.add_color_stop_rgba(0.27, tint.0, tint.1, tint.2, opacity);
    body.add_color_stop_rgba(1.0, tint.0, tint.1, tint.2, opacity);
    rounded(context, bounds, radius);
    context.set_source(body)?;
    context.fill_preserve()?;
    let edge = vte4::gtk::cairo::LinearGradient::new(x, y, x + w * 0.55, y + h);
    edge.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.91);
    edge.add_color_stop_rgba(0.48, 1.0, 1.0, 1.0, 0.33);
    edge.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.53);
    context.set_source(edge)?;
    context.set_line_width(1.0);
    context.stroke()
}

fn paint_card(
    context: &Context,
    width: f64,
    height: f64,
    radius: f64,
    opacity: f64,
    solid: bool,
) -> Result<(), CairoError> {
    rounded(context, (1.0, 1.0, width - 2.0, height - 2.0), radius);
    let body = vte4::gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, height);
    body.add_color_stop_rgba(
        0.0,
        0.078,
        0.135,
        0.244,
        if solid { 1.0 } else { 0.98 * opacity },
    );
    body.add_color_stop_rgba(
        0.27,
        INK.0,
        INK.1,
        INK.2,
        if solid { 1.0 } else { 0.96 * opacity },
    );
    body.add_color_stop_rgba(
        1.0,
        0.015,
        0.027,
        0.067,
        if solid { 1.0 } else { 0.98 * opacity },
    );
    context.set_source(body)?;
    context.fill_preserve()?;
    let rim = vte4::gtk::cairo::LinearGradient::new(0.0, 0.0, width, height);
    rim.add_color_stop_rgba(0.0, 0.75, 0.90, 1.0, 0.72);
    rim.add_color_stop_rgba(0.45, BLUE.0, BLUE.1, BLUE.2, 0.27);
    rim.add_color_stop_rgba(1.0, BLUE.0, BLUE.1, BLUE.2, 0.44);
    context.set_source(rim)?;
    context.set_line_width(1.0);
    context.stroke()
}

pub fn font(context: &Context, size: f64, weight: FontWeight) {
    context.select_font_face("Roboto", FontSlant::Normal, weight);
    context.set_font_size(size);
}

pub fn display_font(context: &Context, size: f64) {
    context.select_font_face("Roboto Black", FontSlant::Normal, FontWeight::Bold);
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
