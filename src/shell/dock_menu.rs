use super::{launch::Target, theme};
use crate::palette::Theme;
use smithay::{
    backend::renderer::gles::{GlesRenderer, GlesTexture},
    desktop::Window,
    utils::{Logical, Physical, Point, Rectangle, Size},
};
use std::error::Error;
use vte4::gtk::cairo::FontWeight;

pub struct Menu {
    pub target: Target,
    pub windows: Vec<(Window, String)>,
    pub bounds: Rectangle<i32, Physical>,
}

impl Menu {
    pub fn new(
        target: Target,
        windows: Vec<(Window, String)>,
        anchor: Point<f64, Logical>,
        top: f64,
        output: Size<i32, Physical>,
    ) -> Option<Self> {
        let width = 280.min(output.w - 16);
        let height = 16 + 34 * (windows.len() as i32 + 2);
        if width < 120 || height + 16 > output.h {
            return None;
        }
        let x = (anchor.x.round() as i32 - width / 2).clamp(8, output.w - width - 8);
        let y = (top.round() as i32 - height - 8).clamp(8, output.h - height - 8);
        Some(Self {
            target,
            windows,
            bounds: Rectangle::new((x, y).into(), (width, height).into()),
        })
    }
    pub fn labels(&self) -> Vec<String> {
        self.windows
            .iter()
            .map(|(_, title)| title.clone())
            .chain(["Next window".into(), "New window".into()])
            .collect()
    }
    pub fn hit(&self, point: Point<f64, Logical>) -> Option<usize> {
        if !self.bounds.to_f64().to_logical(1.0).contains(point) {
            return None;
        }
        let x = point.x - f64::from(self.bounds.loc.x);
        let y = point.y - f64::from(self.bounds.loc.y);
        if x < 8.0 || x >= f64::from(self.bounds.size.w - 8) || y < 8.0 {
            return None;
        }
        let row = ((y - 8.0) / 34.0) as usize;
        (row < self.windows.len() + 2).then_some(row)
    }
}

pub fn texture(
    renderer: &mut GlesRenderer,
    colors: Theme,
    size: Size<i32, Physical>,
    labels: &[String],
    hovered: Option<usize>,
) -> Result<GlesTexture, Box<dyn Error>> {
    let colors = colors.colors();
    theme::texture(renderer, size, |context| {
        let w = f64::from(size.w);
        theme::glass_surface(
            context,
            (1.0, 1.0, w - 2.0, f64::from(size.h) - 2.0),
            14.0,
            colors.base,
            1.0,
        )?;
        for (index, label) in labels.iter().enumerate() {
            let y = 8.0 + 34.0 * index as f64;
            if hovered == Some(index) {
                theme::rounded(context, (8.0, y, w - 16.0, 32.0), 7.0);
                theme::ink(context, colors.selected, 1.0);
                context.fill()?;
            }
            theme::ink(context, colors.text, 1.0);
            theme::font(context, 13.0, FontWeight::Normal);
            let mut label = label.clone();
            while context.text_extents(&label)?.x_advance() > w - 32.0 && label.pop().is_some() {}
            context.move_to(16.0, y + 21.0);
            context.show_text(&label)?;
        }
        Ok(())
    })
}
