use std::{error::Error, fs};

use smithay::{
    backend::renderer::gles::{GlesRenderer, GlesTexture},
    utils::{Logical, Physical, Point, Rectangle, Size},
};
use vte4::gtk::{
    cairo::{Context, Error as CairoError, FontWeight, LineCap, LinearGradient},
    glib::DateTime,
};

use super::theme::{self, AMBER, BLUE, GLASS, INK, MUTED, TEXT};

pub const BAR_HEIGHT: f64 = 42.0;
const PANEL: (i32, i32) = (294, 174);

#[derive(Clone, PartialEq, Eq)]
pub struct Status {
    clock: String,
    date: String,
    network_up: bool,
    wired: bool,
    usb_storage: bool,
}

impl Status {
    pub fn read() -> Self {
        let now = DateTime::now_local().ok();
        let clock = now
            .as_ref()
            .and_then(|time| time.format("%H:%M").ok())
            .map_or_else(|| "--:--".into(), Into::into);
        let date = now
            .and_then(|time| time.format("%a, %b %e").ok())
            .map_or_else(String::new, |date| {
                date.split_whitespace().collect::<Vec<_>>().join(" ")
            });
        let mut network_up = false;
        let mut wired = false;
        if let Ok(interfaces) = fs::read_dir("/sys/class/net") {
            for interface in interfaces
                .flatten()
                .filter(|entry| entry.file_name() != "lo")
            {
                let path = interface.path();
                if fs::read_to_string(path.join("operstate"))
                    .is_ok_and(|state| state.trim() == "up")
                {
                    network_up = true;
                    wired |= !path.join("wireless").is_dir();
                }
            }
        }
        let usb_storage = fs::read_dir("/sys/block").is_ok_and(|devices| {
            devices.flatten().any(|device| {
                fs::canonicalize(device.path().join("device")).is_ok_and(|path| {
                    path.components().any(|part| {
                        part.as_os_str().to_str().is_some_and(|name| {
                            name.strip_prefix("usb").is_some_and(|bus| {
                                !bus.is_empty() && bus.bytes().all(|byte| byte.is_ascii_digit())
                            })
                        })
                    })
                })
            })
        });
        Self {
            clock,
            date,
            network_up,
            wired,
            usb_storage,
        }
    }
}

#[derive(Default)]
pub struct StatusBar {
    rendered: Option<(Size<i32, Physical>, Status, bool, GlesTexture)>,
    panel: Option<(Status, usize, GlesTexture)>,
}

impl StatusBar {
    pub fn texture(
        &mut self,
        renderer: &mut GlesRenderer,
        size: Size<i32, Physical>,
        status: &Status,
        expanded: bool,
    ) -> Result<&GlesTexture, Box<dyn Error>> {
        if !self
            .rendered
            .as_ref()
            .is_some_and(|(old_size, old_status, old_expanded, _)| {
                *old_size == size && old_status == status && *old_expanded == expanded
            })
        {
            let texture = theme::texture(renderer, size, |context| {
                paint(context, size, status, expanded)
            })?;
            self.rendered = Some((size, status.clone(), expanded, texture));
        }
        Ok(&self.rendered.as_ref().expect("status bar was rendered").3)
    }

    pub fn panel(
        &mut self,
        renderer: &mut GlesRenderer,
        status: &Status,
        workspace: usize,
    ) -> Result<&GlesTexture, Box<dyn Error>> {
        if !self
            .panel
            .as_ref()
            .is_some_and(|(old_status, old_workspace, _)| {
                old_status == status && *old_workspace == workspace
            })
        {
            let texture = theme::texture(renderer, PANEL.into(), |context| {
                paint_panel(context, status, workspace)
            })?;
            self.panel = Some((status.clone(), workspace, texture));
        }
        Ok(&self.panel.as_ref().expect("status panel was rendered").2)
    }
}

pub fn bounds(width: i32, scale: f64) -> Rectangle<i32, Physical> {
    let height = (BAR_HEIGHT * scale).round().clamp(32.0, 84.0) as i32;
    Rectangle::from_size((width, height).into())
}

pub fn panel_bounds(
    output: Size<i32, Physical>,
    bar: Rectangle<i32, Physical>,
) -> Rectangle<i32, Physical> {
    let scale = f64::from(bar.size.h) / BAR_HEIGHT;
    let width = ((f64::from(PANEL.0) * scale).round() as i32)
        .min(output.w - 16)
        .max(1);
    let height = ((f64::from(PANEL.1) * scale).round() as i32)
        .min(output.h - bar.size.h - 16)
        .max(1);
    Rectangle::new(
        (output.w - width - 8, bar.size.h + 8).into(),
        (width, height).into(),
    )
}

pub fn toggle_contains(bar: Rectangle<i32, Physical>, point: Point<f64, Logical>) -> bool {
    let scale = f64::from(bar.size.h) / BAR_HEIGHT;
    point.y >= 4.0 * scale
        && point.y < 38.0 * scale
        && point.x >= f64::from(bar.size.w) - 101.0 * scale
}

fn paint(
    context: &Context,
    size: Size<i32, Physical>,
    status: &Status,
    expanded: bool,
) -> Result<(), CairoError> {
    let scale = f64::from(size.h) / BAR_HEIGHT;
    let width = f64::from(size.w) / scale;
    context.scale(scale, scale);
    let shade = LinearGradient::new(0.0, 0.0, 0.0, BAR_HEIGHT);
    shade.add_color_stop_rgba(0.0, 0.073, 0.110, 0.185, 0.99);
    shade.add_color_stop_rgba(1.0, INK.0, INK.1, INK.2, 0.97);
    context.set_source(shade)?;
    context.paint()?;
    theme::ink(context, BLUE, 0.29);
    context.rectangle(0.0, BAR_HEIGHT - 1.0, width, 1.0);
    context.fill()?;

    theme::rounded(context, (12.0, 8.0, 26.0, 26.0), 8.0);
    theme::ink(context, GLASS, 1.0);
    context.fill()?;
    theme::ink(context, BLUE, 1.0);
    context.set_line_width(1.4);
    context.set_line_join(vte4::gtk::cairo::LineJoin::Round);
    context.move_to(17.5, 25.0);
    context.line_to(23.0, 15.8);
    context.line_to(26.5, 21.0);
    context.line_to(28.5, 18.4);
    context.line_to(32.5, 25.0);
    context.close_path();
    context.stroke()?;

    if width >= 300.0 {
        theme::ink(context, TEXT, 1.0);
        theme::font(context, 13.0, FontWeight::Bold);
        context.move_to(46.0, 25.5);
        context.show_text("Vantage")?;
    }

    let clock = if width >= 460.0 && !status.date.is_empty() {
        format!("{}  ·  {}", status.date, status.clock)
    } else {
        status.clock.clone()
    };
    theme::ink(context, TEXT, 1.0);
    theme::font(context, 13.0, FontWeight::Bold);
    let text = context.text_extents(&clock)?;
    context.move_to((width - text.width()) * 0.5 - text.x_bearing(), 25.5);
    context.show_text(&clock)?;

    theme::rounded(context, (width - 100.0, 7.0, 88.0, 28.0), 10.0);
    theme::ink(context, GLASS, if expanded { 1.0 } else { 0.66 });
    context.fill_preserve()?;
    theme::ink(context, BLUE, if expanded { 0.72 } else { 0.24 });
    context.set_line_width(1.0);
    context.stroke()?;

    context.set_line_cap(LineCap::Round);
    context.set_line_width(1.45);
    let net = width - 77.0;
    theme::ink(context, if status.network_up { BLUE } else { MUTED }, 0.95);
    if status.wired {
        theme::rounded(context, (net - 6.0, 15.0, 12.0, 8.0), 1.6);
        context.move_to(net, 23.0);
        context.line_to(net, 26.0);
        context.move_to(net - 3.0, 26.0);
        context.line_to(net + 3.0, 26.0);
        context.stroke()?;
    } else {
        for radius in [5.0, 9.0, 12.0] {
            context.new_sub_path();
            context.arc(net, 26.0, radius, -2.23, -0.91);
        }
        context.stroke()?;
        theme::dot(context, net, 26.0, 1.15)?;
    }

    let usb = width - 54.0;
    theme::ink(
        context,
        if status.usb_storage { AMBER } else { MUTED },
        0.85,
    );
    theme::rounded(context, (usb - 5.0, 18.0, 10.0, 10.0), 2.5);
    context.move_to(usb - 3.0, 18.0);
    context.line_to(usb - 3.0, 13.8);
    context.line_to(usb + 3.0, 13.8);
    context.line_to(usb + 3.0, 18.0);
    context.stroke()?;

    let arrow = width - 30.0;
    theme::ink(context, TEXT, 0.92);
    let tilt = if expanded { -1.0 } else { 1.0 };
    context.move_to(arrow - 3.5, 21.0 - 1.5 * tilt);
    context.line_to(arrow, 21.0 + 2.0 * tilt);
    context.line_to(arrow + 3.5, 21.0 - 1.5 * tilt);
    context.stroke()
}

fn paint_panel(context: &Context, status: &Status, workspace: usize) -> Result<(), CairoError> {
    theme::card(context, f64::from(PANEL.0), f64::from(PANEL.1), 18.0)?;
    theme::font(context, 15.0, FontWeight::Bold);
    theme::ink(context, TEXT, 1.0);
    context.move_to(17.0, 27.0);
    context.show_text("System status")?;

    let network = match (status.network_up, status.wired) {
        (true, true) => "Wired link active",
        (true, false) => "Wireless link active",
        _ => "Disconnected",
    };
    let usb = if status.usb_storage {
        "Connected"
    } else {
        "None detected"
    };
    let workspace = format!(
        "{} of {}",
        workspace + 1,
        super::workspaces::WORKSPACE_COUNT
    );
    for (index, (label, value, accent)) in [
        (
            "Network",
            network,
            if status.network_up { BLUE } else { MUTED },
        ),
        (
            "USB storage",
            usb,
            if status.usb_storage { AMBER } else { MUTED },
        ),
        ("Workspace", workspace.as_str(), BLUE),
    ]
    .into_iter()
    .enumerate()
    {
        let y = 39.0 + index as f64 * 42.0;
        theme::rounded(context, (12.0, y, f64::from(PANEL.0) - 24.0, 36.0), 9.0);
        theme::ink(context, GLASS, 0.84);
        context.fill()?;
        theme::ink(context, accent, 1.0);
        theme::dot(context, 25.0, y + 18.0, 3.0)?;
        theme::font(context, 12.0, FontWeight::Bold);
        theme::ink(context, TEXT, 1.0);
        context.move_to(37.0, y + 22.0);
        context.show_text(label)?;
        theme::font(context, 12.0, FontWeight::Normal);
        theme::ink(context, MUTED, 1.0);
        let text = context.text_extents(value)?;
        context.move_to(f64::from(PANEL.0) - 24.0 - text.x_advance(), y + 22.0);
        context.show_text(value)?;
    }
    Ok(())
}
