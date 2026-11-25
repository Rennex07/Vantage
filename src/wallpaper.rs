use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Frame, ImportMem, Texture,
            gles::{GlesFrame, GlesRenderer, GlesTexture},
        },
    },
    utils::{Buffer, Physical, Rectangle, Size, Transform},
};

pub fn load(renderer: &mut GlesRenderer) -> Result<GlesTexture, Box<dyn std::error::Error>> {
    let mut image = image::load_from_memory_with_format(
        include_bytes!("../assets/contours.png"),
        image::ImageFormat::Png,
    )?
    .into_rgba8();
    // Use the same navy shadows and warm highlights as the Pi 4 shell.
    for pixel in image.pixels_mut() {
        let [r, g, b, _] = pixel.0.map(u32::from);
        let cool = [(3 * r + g) / 8, (3 * g + b) / 5, b];
        let warm = [r, (3 * g + r) / 4, 3 * b / 4];
        let warmth = ((r.saturating_sub(b)) * 6).min(256);
        for (index, channel) in pixel.0[..3].iter_mut().enumerate() {
            *channel = ((cool[index] * (256 - warmth) + warm[index] * warmth) / 256) as u8;
        }
    }
    let size = (image.width().try_into()?, image.height().try_into()?).into();
    Ok(renderer.import_memory(image.as_raw(), Fourcc::Abgr8888, size, false)?)
}

pub fn draw(
    frame: &mut GlesFrame<'_, '_>,
    texture: &GlesTexture,
    target: Rectangle<i32, Physical>,
) -> Result<(), smithay::backend::renderer::gles::GlesError> {
    Frame::render_texture_from_to(
        frame,
        texture,
        cover(texture.size(), target.size),
        target,
        &[target],
        &[],
        Transform::Normal,
        1.0,
    )
}

pub fn cover(source: Size<i32, Buffer>, target: Size<i32, Physical>) -> Rectangle<f64, Buffer> {
    let source = source.to_f64();
    let target = target.to_f64();
    let scale = (target.w / source.w).max(target.h / source.h);
    let visible = (target.w / scale, target.h / scale);
    Rectangle::new(
        ((source.w - visible.0) / 2.0, (source.h - visible.1) / 2.0).into(),
        visible.into(),
    )
}
