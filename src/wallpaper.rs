use std::time::{Duration, Instant};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Frame, ImportMem, Offscreen, Renderer, Texture,
            gles::{
                GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform,
                UniformName, UniformType,
            },
        },
    },
    utils::{Buffer, Physical, Rectangle, Size, Transform},
};

use crate::{
    palette::{Color, Mode, Theme, mix},
    shell::timeline::{Ease, Transition},
};

#[derive(Clone, Copy, PartialEq)]
struct Grade {
    lift: Color,
    luma: Color,
    light: f64,
}

impl Grade {
    fn for_theme(theme: Theme) -> Self {
        let tint = theme.dark_tint();
        Self {
            lift: mix((0.007, 0.009, 0.010), tint, 0.072),
            luma: mix((0.20, 0.20, 0.20), tint, 0.86),
            light: f64::from(theme.mode == Mode::Light),
        }
    }

    fn blend(self, target: Self, amount: f64) -> Self {
        if amount >= 1.0 {
            return target;
        }
        Self {
            lift: mix(self.lift, target.lift, amount),
            luma: mix(self.luma, target.luma, amount),
            light: self.light + (target.light - self.light) * amount,
        }
    }

    fn names() -> [UniformName<'static>; 2] {
        [
            UniformName::new("tone_lift", UniformType::_3f),
            UniformName::new("tone_luma", UniformType::_3f),
        ]
    }

    fn uniforms(self) -> [Uniform<'static>; 2] {
        [
            Uniform::new(
                "tone_lift",
                (self.lift.0 as f32, self.lift.1 as f32, self.lift.2 as f32),
            ),
            Uniform::new(
                "tone_luma",
                (self.luma.0 as f32, self.luma.1 as f32, self.luma.2 as f32),
            ),
        ]
    }
}

pub struct Wallpaper {
    contours: GlesTexture,
    opal: GlesTexture,
    shader: GlesTexProgram,
    current: Option<(Theme, Grade, Transition)>,
    cached: Option<(Size<i32, Physical>, Grade, GlesTexture)>,
}

impl Wallpaper {
    pub fn new(renderer: &mut GlesRenderer) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            contours: load(renderer, Mode::Dark)?,
            opal: load(renderer, Mode::Light)?,
            shader: renderer
                .compile_custom_texture_shader(include_str!("wallpaper.frag"), &Grade::names())?,
            current: None,
            cached: None,
        })
    }

    fn grade(&mut self, theme: Theme, now: Instant, reduce_motion: bool) -> Grade {
        let target = Grade::for_theme(theme);
        let from = match &self.current {
            Some((previous, from, motion)) if *previous == theme && !reduce_motion => {
                return from.blend(target, motion.value(now));
            }
            Some((previous, from, motion)) if !reduce_motion => {
                from.blend(Grade::for_theme(*previous), motion.value(now))
            }
            _ => target,
        };
        let duration = if self.current.is_none() || reduce_motion {
            Duration::ZERO
        } else {
            Duration::from_millis(420)
        };
        self.current = Some((
            theme,
            from,
            Transition::new(0.0, 1.0, now, duration, Ease::Smooth),
        ));
        from.blend(
            target,
            self.current
                .as_ref()
                .expect("wallpaper tone set")
                .2
                .value(now),
        )
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.current
            .as_ref()
            .is_some_and(|(_, _, motion)| !motion.finished(now))
    }

    pub fn transition_amount(&self, now: Instant) -> f32 {
        self.current
            .as_ref()
            .map_or(1.0, |(_, _, motion)| motion.value(now) as f32)
    }

    pub fn texture(
        &mut self,
        renderer: &mut GlesRenderer,
        size: Size<i32, Physical>,
        theme: Theme,
        now: Instant,
        reduce_motion: bool,
    ) -> Result<GlesTexture, GlesError> {
        let grade = self.grade(theme, now, reduce_motion);
        if grade.light >= 1.0 {
            return Ok(self.opal.clone());
        }
        if self
            .cached
            .as_ref()
            .is_none_or(|(old_size, _, _)| *old_size != size)
        {
            let texture = renderer.create_buffer(Fourcc::Abgr8888, (size.w, size.h).into())?;
            self.cached = Some((
                size,
                Grade {
                    light: -1.0,
                    ..grade
                },
                texture,
            ));
        }
        let (_, cached_grade, texture) = self.cached.as_mut().expect("wallpaper texture allocated");
        if grade != *cached_grade {
            let bounds = Rectangle::from_size(size);
            let mut target = renderer.bind(texture)?;
            let mut frame = renderer.render(&mut target, size, Transform::Normal)?;
            frame.render_texture_from_to(
                &self.contours,
                cover(self.contours.size(), size),
                bounds,
                &[bounds],
                &[],
                Transform::Normal,
                1.0,
                Some(&self.shader),
                &grade.uniforms(),
            )?;
            if grade.light > 0.0 {
                draw(&mut frame, &self.opal, bounds, grade.light as f32)?;
            }
            let _ = frame.finish()?;
            *cached_grade = grade;
        }
        Ok(texture.clone())
    }
}

pub fn draw(
    frame: &mut GlesFrame<'_, '_>,
    texture: &GlesTexture,
    target: Rectangle<i32, Physical>,
    alpha: f32,
) -> Result<(), GlesError> {
    Frame::render_texture_from_to(
        frame,
        texture,
        cover(texture.size(), target.size),
        target,
        &[Rectangle::from_size(target.size)],
        &[],
        Transform::Normal,
        alpha,
    )
}

pub fn art(mode: Mode) -> &'static [u8] {
    match mode {
        Mode::Dark => include_bytes!("../assets/contours.png"),
        Mode::Light => include_bytes!("../assets/opal.png"),
    }
}

fn load(
    renderer: &mut GlesRenderer,
    mode: Mode,
) -> Result<GlesTexture, Box<dyn std::error::Error>> {
    let mut image =
        image::load_from_memory_with_format(art(mode), image::ImageFormat::Png)?.into_rgba8();
    // Use the same navy shadows and warm highlights as the Pi 4 shell.
    if mode == Mode::Dark {
        for pixel in image.pixels_mut() {
            let [r, g, b, _] = pixel.0.map(u32::from);
            let cool = [(3 * r + g) / 8, (3 * g + b) / 5, b];
            let warm = [r, (3 * g + r) / 4, 3 * b / 4];
            let warmth = ((r.saturating_sub(b)) * 6).min(256);
            for (index, channel) in pixel.0[..3].iter_mut().enumerate() {
                *channel = ((cool[index] * (256 - warmth) + warm[index] * warmth) / 256) as u8;
            }
        }
    }
    let size = (image.width().try_into()?, image.height().try_into()?).into();
    Ok(renderer.import_memory(image.as_raw(), Fourcc::Abgr8888, size, false)?)
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
