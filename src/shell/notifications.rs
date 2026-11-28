use std::error::Error;

use smithay::{
    backend::renderer::gles::{GlesRenderer, GlesTexture},
    utils::{Logical, Physical, Point, Rectangle},
};
use vte4::gtk::cairo::{Context, Error as CairoError, FontWeight};

use crate::{
    notifications::{Center, Entry, VISIBLE_ROWS},
    palette::{Palette, Theme},
};

use super::{status::Action, theme};

pub const SIZE: (i32, i32) = (342, 360);
pub const BANNER_SIZE: (i32, i32) = (342, 76);
const FIRST_ROW: f64 = 61.0;
const ROW_HEIGHT: f64 = 64.0;
const QUIET: (f64, f64, f64, f64) = (138.0, 13.0, 116.0, 32.0);
const CLEAR: (f64, f64, f64, f64) = (262.0, 13.0, 65.0, 32.0);
const NEWER: (f64, f64, f64, f64) = (16.0, 322.0, 72.0, 26.0);
const OLDER: (f64, f64, f64, f64) = (254.0, 322.0, 72.0, 26.0);

#[derive(Default)]
pub struct Hits {
    pub banner: Option<Rectangle<i32, Physical>>,
    pub rows: Vec<(u32, usize)>,
    pub revision: u64,
    pub page: usize,
    pub pages: usize,
}

impl Hits {
    pub fn banner_contains(&self, point: Point<f64, Logical>) -> bool {
        self.banner
            .is_some_and(|rect| rect.to_f64().to_logical(1.0).contains(point))
    }

    pub fn action_at(
        &self,
        popup: Rectangle<i32, Physical>,
        point: Point<f64, Logical>,
    ) -> Option<Action> {
        if !popup.to_f64().to_logical(1.0).contains(point) {
            return None;
        }
        let x = (point.x - f64::from(popup.loc.x)) * f64::from(SIZE.0) / f64::from(popup.size.w);
        let y = (point.y - f64::from(popup.loc.y)) * f64::from(SIZE.1) / f64::from(popup.size.h);
        let contains =
            |(left, top, w, h)| (left..left + w).contains(&x) && (top..top + h).contains(&y);
        if contains(QUIET) {
            return Some(Action::NotificationQuiet);
        }
        if contains(CLEAR) {
            return Some(Action::NotificationClear);
        }
        if self.page > 0 && contains(NEWER) {
            return Some(Action::NotificationPage(-1));
        }
        if self.page + 1 < self.pages && contains(OLDER) {
            return Some(Action::NotificationPage(1));
        }
        if y < FIRST_ROW {
            return None;
        }
        let row = ((y - FIRST_ROW) / ROW_HEIGHT) as usize;
        let (id, actions) = *self.rows.get(row)?;
        let y = y - row as f64 * ROW_HEIGHT - FIRST_ROW;
        if (296.0..326.0).contains(&x) && (1.0..32.0).contains(&y) {
            return Some(Action::NotificationDismiss(id));
        }
        if (38.0..60.0).contains(&y) && (18.0..292.0).contains(&x) && actions > 0 {
            let action = usize::from(x >= 156.0 && actions > 1);
            return Some(Action::NotificationInvoke(id, action, self.revision));
        }
        None
    }
}

pub fn panel(
    renderer: &mut GlesRenderer,
    colors: Theme,
    center: &Center,
    page: usize,
) -> Result<GlesTexture, Box<dyn Error>> {
    let colors = colors.colors();
    theme::texture(renderer, SIZE.into(), |context| {
        theme::glass_surface(context, (1.0, 1.0, 340.0, 358.0), 22.0, colors.base, 1.0)?;
        text(context, colors, "Notifications", (16.0, 33.0), 119.0, true)?;
        button(
            context,
            colors,
            QUIET,
            if center.quiet() { "DND on" } else { "DND off" },
        )?;
        button(context, colors, CLEAR, "Clear")?;
        if center.visible(page).next().is_none() {
            text(
                context,
                colors,
                "No notifications",
                (17.0, 88.0),
                300.0,
                false,
            )?;
        }
        for (row, entry) in center.visible(page).take(VISIBLE_ROWS).enumerate() {
            let y = FIRST_ROW + row as f64 * ROW_HEIGHT;
            theme::rounded(context, (12.0, y, 318.0, ROW_HEIGHT - 4.0), 11.0);
            theme::ink(context, colors.surface, 1.0);
            context.fill()?;
            text(
                context,
                colors,
                &entry.summary,
                (19.0, y + 17.0),
                270.0,
                true,
            )?;
            let detail = if entry.app.is_empty() {
                entry.body.clone()
            } else if entry.body.is_empty() {
                entry.app.clone()
            } else {
                format!("{} · {}", entry.app, entry.body)
            };
            text(context, colors, &detail, (19.0, y + 34.0), 302.0, false)?;
            button(context, colors, (296.0, y + 1.0, 30.0, 30.0), "×")?;
            for (index, action) in entry.actions.iter().take(2).enumerate() {
                let x = 18.0 + 138.0 * index as f64;
                button(
                    context,
                    colors,
                    (
                        x,
                        y + 38.0,
                        if entry.actions.len() == 1 {
                            274.0
                        } else {
                            136.0
                        },
                        21.0,
                    ),
                    &action.label,
                )?;
            }
        }
        if page > 0 {
            button(context, colors, NEWER, "Newer")?;
        }
        if page + 1 < center.pages() {
            button(context, colors, OLDER, "Older")?;
        }
        Ok(())
    })
}

pub fn banner(
    renderer: &mut GlesRenderer,
    colors: Theme,
    entry: &Entry,
) -> Result<GlesTexture, Box<dyn Error>> {
    let colors = colors.colors();
    theme::texture(renderer, BANNER_SIZE.into(), |context| {
        theme::glass_surface(context, (1.0, 1.0, 340.0, 74.0), 19.0, colors.base, 1.0)?;
        text(context, colors, &entry.app, (16.0, 22.0), 306.0, false)?;
        text(context, colors, &entry.summary, (16.0, 42.0), 306.0, true)?;
        text(context, colors, &entry.body, (16.0, 61.0), 306.0, false)
    })
}

fn button(
    context: &Context,
    colors: Palette,
    (x, y, w, h): (f64, f64, f64, f64),
    label: &str,
) -> Result<(), CairoError> {
    theme::rounded(context, (x, y, w, h), 7.0);
    theme::ink(context, colors.selected, 1.0);
    context.fill()?;
    text(
        context,
        colors,
        label,
        (x + 7.0, y + h / 2.0 + 4.0),
        w - 13.0,
        false,
    )
}

fn text(
    context: &Context,
    colors: Palette,
    label: &str,
    (x, y): (f64, f64),
    width: f64,
    bold: bool,
) -> Result<(), CairoError> {
    theme::ink(context, if bold { colors.text } else { colors.muted }, 1.0);
    theme::font(
        context,
        if bold { 13.0 } else { 11.0 },
        if bold {
            FontWeight::Bold
        } else {
            FontWeight::Normal
        },
    );
    context.move_to(x, y);
    if context.text_extents(label)?.x_advance() <= width {
        return context.show_text(label);
    }
    let mut label = label.to_owned();
    let ellipsis = context.text_extents("…")?.x_advance();
    while context.text_extents(&label)?.x_advance() + ellipsis > width && label.pop().is_some() {}
    label.push('…');
    context.show_text(&label)
}
