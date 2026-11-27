use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Frame, Offscreen, Renderer, Texture,
            gles::{
                GlesError, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName,
                UniformType,
            },
            utils::draw_render_elements,
        },
    },
    utils::{Physical, Rectangle, Size, Transform},
};

use crate::{
    palette::{Mode, Theme},
    wallpaper,
    wayland::WindowScene,
};

// FBO pixels are sampled as a top-down texture by subsequent Smithay passes.
const SURFACE_TRANSFORM: Transform = Transform::Normal;

pub struct FrostedWindow {
    pub behind: usize,
    pub bounds: Rectangle<i32, Physical>,
    pub opacity: f32,
    pub theme: Theme,
}

pub struct GlassStyle {
    pub overview: f32,
    pub full_effects: bool,
}

pub struct Glass {
    gaussian: GlesTexProgram,
    frost: GlesTexProgram,
    textures: Option<Buffers>,
}

struct Buffers {
    scene: GlesTexture,
    horizontal: GlesTexture,
    blurred: GlesTexture,
    downsample: i32,
}

impl Glass {
    pub fn new(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        Ok(Self {
            gaussian: renderer.compile_custom_texture_shader(
                include_str!("gaussian.frag"),
                &[
                    UniformName::new("direction", UniformType::_2f),
                    UniformName::new("texel", UniformType::_2f),
                ],
            )?,
            frost: renderer.compile_custom_texture_shader(
                include_str!("frost.frag"),
                &[
                    UniformName::new("panel_origin", UniformType::_2f),
                    UniformName::new("output_size", UniformType::_2f),
                    UniformName::new("panel_size", UniformType::_2f),
                    UniformName::new("surface_color", UniformType::_3f),
                    UniformName::new("daylight", UniformType::_1f),
                ],
            )?,
            textures: None,
        })
    }

    pub fn compose(
        &mut self,
        renderer: &mut GlesRenderer,
        wallpaper: &GlesTexture,
        size: Size<i32, Physical>,
        windows: &WindowScene,
        style: GlassStyle,
    ) -> Result<&GlesTexture, GlesError> {
        let downsample = if style.full_effects { 4 } else { 8 };
        if !self.textures.as_ref().is_some_and(|buffers| {
            buffers.scene.size().w == size.w
                && buffers.scene.size().h == size.h
                && buffers.downsample == downsample
        }) {
            let full = (size.w, size.h).into();
            let reduced = (
                (size.w + downsample - 1) / downsample,
                (size.h + downsample - 1) / downsample,
            )
                .into();
            self.textures = Some(Buffers {
                scene: renderer.create_buffer(Fourcc::Abgr8888, full)?,
                horizontal: renderer.create_buffer(Fourcc::Abgr8888, reduced)?,
                blurred: renderer.create_buffer(Fourcc::Abgr8888, reduced)?,
                downsample,
            });
        }
        let buffers = self.textures.as_mut().expect("glass textures allocated");
        let bounds = Rectangle::from_size(size);
        let mut scene = renderer.bind(&mut buffers.scene)?;
        {
            let mut frame = renderer.render(&mut scene, size, SURFACE_TRANSFORM)?;
            wallpaper::draw(&mut frame, wallpaper, bounds, 1.0)?;
            crate::session::dim_wallpaper(&mut frame, bounds, style.overview)?;
            let _ = frame.finish()?;
        }

        // The element list runs front to back. Frost only what has already been drawn
        // behind each Files window, then add that window with its text at full detail.
        let mut remaining = windows.elements.len();
        for panel in windows.glass.iter().rev() {
            let Some(visible) = panel.bounds.intersection(bounds) else {
                continue;
            };
            {
                let mut frame = renderer.render(&mut scene, size, SURFACE_TRANSFORM)?;
                draw_render_elements(
                    &mut frame,
                    1.0,
                    &windows.elements[panel.behind..remaining],
                    &[bounds],
                )?;
                let _ = frame.finish()?;
            }
            remaining = panel.behind;
            // The framebuffer binding retains a reference to the scene texture.
            drop(scene);
            let low_size = buffers.horizontal.size();
            blur_pass(
                renderer,
                &self.gaussian,
                &buffers.scene,
                &mut buffers.blurred,
                (0.0, 0.0),
            )?;
            blur_pass(
                renderer,
                &self.gaussian,
                &buffers.blurred,
                &mut buffers.horizontal,
                (1.0 / low_size.w as f32, 0.0),
            )?;
            blur_pass(
                renderer,
                &self.gaussian,
                &buffers.horizontal,
                &mut buffers.blurred,
                (0.0, 1.0 / low_size.h as f32),
            )?;
            scene = renderer.bind(&mut buffers.scene)?;
            let mut frame = renderer.render(&mut scene, size, SURFACE_TRANSFORM)?;
            let x = f64::from(panel.bounds.loc.x) / f64::from(size.w);
            let y = f64::from(panel.bounds.loc.y) / f64::from(size.h);
            let (r, g, b) = panel.theme.colors().surface;
            let daylight = f32::from(panel.theme.mode == Mode::Light);
            let source = Rectangle::new(
                (x * f64::from(low_size.w), y * f64::from(low_size.h)).into(),
                (
                    (f64::from(panel.bounds.size.w) / f64::from(size.w)) * f64::from(low_size.w),
                    (f64::from(panel.bounds.size.h) / f64::from(size.h)) * f64::from(low_size.h),
                )
                    .into(),
            );
            frame.render_texture_from_to(
                &buffers.blurred,
                source,
                panel.bounds,
                &[Rectangle::new(visible.loc - panel.bounds.loc, visible.size)],
                &[],
                Transform::Normal,
                panel.opacity,
                Some(&self.frost),
                &[
                    Uniform::new("panel_origin", (x as f32, y as f32)),
                    Uniform::new("output_size", (size.w as f32, size.h as f32)),
                    Uniform::new("surface_color", (r as f32, g as f32, b as f32)),
                    Uniform::new("daylight", daylight),
                    Uniform::new(
                        "panel_size",
                        (panel.bounds.size.w as f32, panel.bounds.size.h as f32),
                    ),
                ],
            )?;
            let _ = frame.finish()?;
        }
        {
            let mut frame = renderer.render(&mut scene, size, SURFACE_TRANSFORM)?;
            draw_render_elements(&mut frame, 1.0, &windows.elements[..remaining], &[bounds])?;
            let _ = frame.finish()?;
        }
        drop(scene);
        Ok(&buffers.scene)
    }
}

fn blur_pass(
    renderer: &mut GlesRenderer,
    shader: &GlesTexProgram,
    source: &GlesTexture,
    destination: &mut GlesTexture,
    direction: (f32, f32),
) -> Result<(), GlesError> {
    let dimensions = destination.size();
    let size = (dimensions.w, dimensions.h).into();
    let bounds = Rectangle::from_size(size);
    let mut target = renderer.bind(destination)?;
    let mut frame = renderer.render(&mut target, size, SURFACE_TRANSFORM)?;
    frame.render_texture_from_to(
        source,
        Rectangle::from_size(source.size().to_f64()),
        bounds,
        &[bounds],
        &[],
        Transform::Normal,
        1.0,
        Some(shader),
        &[
            Uniform::new("direction", direction),
            Uniform::new(
                "texel",
                (
                    1.0_f32 / source.size().w as f32,
                    1.0_f32 / source.size().h as f32,
                ),
            ),
        ],
    )?;
    let _ = frame.finish()?;
    Ok(())
}
