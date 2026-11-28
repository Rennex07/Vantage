use std::{error::Error, fs};

use smithay::{
    backend::renderer::gles::{GlesRenderer, GlesTexture},
    utils::{Logical, Physical, Point, Rectangle, Size},
};
use vte4::gtk::{
    cairo::{Context, Error as CairoError, FontWeight, LineCap, LinearGradient},
    glib::DateTime,
};

use crate::desktop::Tool;

use super::{theme, workspaces::WORKSPACE_COUNT};

pub const BAR_HEIGHT: f64 = 42.0;
const PANEL_WIDTH: i32 = 312;
const PANEL_RADIUS: f32 = 22.0;
const FIRST_ROW: f64 = 43.0;
const ROW_HEIGHT: f64 = 43.0;
const WORKSPACE_X: f64 = 177.0;
const WORKSPACE_STEP: f64 = 35.0;
const SETTINGS_WIDTH: f64 = 110.0;
const OVERVIEW_X: f64 = 291.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Toggle,
    Overview,
    Workspace(usize),
    Launch(Tool),
    Power,
    Lock,
    Shutdown,
    Logout,
    Cancel,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Panel {
    #[default]
    Closed,
    Settings,
    Power,
    Shutdown,
    Logout,
}

impl Panel {
    pub fn open(self) -> bool {
        self != Self::Closed
    }

    fn size(self) -> (i32, i32) {
        (PANEL_WIDTH, if self == Self::Settings { 294 } else { 202 })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Status {
    pub clock: String,
    pub date: String,
    network: &'static str,
    bluetooth: &'static str,
    usb_storage: bool,
    battery: Option<(u8, bool)>,
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

        let mut wireless = false;
        let mut wireless_up = false;
        let mut wired_up = false;
        for interface in fs::read_dir("/sys/class/net")
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = interface.path();
            let is_wifi = path.join("wireless").is_dir() || path.join("phy80211").exists();
            wireless |= is_wifi;
            if interface.file_name() != "lo"
                && path.join("device").exists()
                && fs::read_to_string(path.join("operstate"))
                    .is_ok_and(|state| state.trim() == "up")
            {
                wireless_up |= is_wifi;
                wired_up |= !is_wifi;
            }
        }
        let network = if wireless_up {
            "Wi-Fi connected"
        } else if wired_up {
            "Wired connection"
        } else if wireless {
            "Wi-Fi disconnected"
        } else {
            "Disconnected"
        };

        let bluetooth = if fs::read_dir("/sys/class/bluetooth")
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| {
                entry.file_name().to_str().is_some_and(|name| {
                    name.strip_prefix("hci").is_some_and(|number| {
                        !number.is_empty() && number.bytes().all(|n| n.is_ascii_digit())
                    })
                })
            }) {
            "Manage devices"
        } else {
            "No adapter detected"
        };
        let usb_storage = fs::read_dir("/sys/block")
            .into_iter()
            .flatten()
            .flatten()
            .any(|device| {
                fs::canonicalize(device.path().join("device")).is_ok_and(|path| {
                    path.components().any(|part| {
                        part.as_os_str().to_str().is_some_and(|name| {
                            name.strip_prefix("usb").is_some_and(|bus| {
                                !bus.is_empty() && bus.bytes().all(|n| n.is_ascii_digit())
                            })
                        })
                    })
                })
            });
        let battery = fs::read_dir("/sys/class/power_supply")
            .into_iter()
            .flatten()
            .flatten()
            .find_map(|supply| {
                let path = supply.path();
                let kind = fs::read_to_string(path.join("type")).ok()?;
                let scope = fs::read_to_string(path.join("scope")).unwrap_or_default();
                if kind.trim() != "Battery" || scope.trim() == "Device" {
                    return None;
                }
                let capacity: u8 = fs::read_to_string(path.join("capacity"))
                    .ok()?
                    .trim()
                    .parse()
                    .ok()?;
                let charging = fs::read_to_string(path.join("status"))
                    .is_ok_and(|status| status.trim() == "Charging");
                Some((capacity.min(100), charging))
            });
        Self {
            clock,
            date,
            network,
            bluetooth,
            usb_storage,
            battery,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct View {
    pub theme: crate::palette::Theme,
    pub status: Status,
    pub panel: Panel,
    pub workspace: usize,
    pub hovered: Option<Action>,
    pub overview: bool,
    pub native: bool,
    pub error: Option<&'static str>,
    pub solid: bool,
}

#[derive(Default)]
pub struct StatusBar {
    rendered: Option<(Size<i32, Physical>, View, GlesTexture)>,
    panel: Option<(View, GlesTexture)>,
}

impl StatusBar {
    pub fn texture(
        &mut self,
        renderer: &mut GlesRenderer,
        size: Size<i32, Physical>,
        view: &View,
    ) -> Result<&GlesTexture, Box<dyn Error>> {
        if !self
            .rendered
            .as_ref()
            .is_some_and(|(old_size, old_view, _)| *old_size == size && old_view == view)
        {
            let texture = theme::texture(renderer, size, |context| paint(context, size, view))?;
            self.rendered = Some((size, view.clone(), texture));
        }
        Ok(&self.rendered.as_ref().expect("status bar was rendered").2)
    }

    pub fn panel(
        &mut self,
        renderer: &mut GlesRenderer,
        view: &View,
    ) -> Result<&GlesTexture, Box<dyn Error>> {
        if !self
            .panel
            .as_ref()
            .is_some_and(|(old_view, _)| old_view == view)
        {
            let texture = theme::texture(renderer, view.panel.size().into(), |context| {
                paint_panel(context, view)
            })?;
            self.panel = Some((view.clone(), texture));
        }
        Ok(&self.panel.as_ref().expect("settings panel was rendered").1)
    }
}

pub fn bounds(width: i32, scale: f64) -> Rectangle<i32, Physical> {
    let height = (BAR_HEIGHT * scale).round().clamp(32.0, 84.0) as i32;
    Rectangle::from_size((width, height).into())
}

pub fn panel_bounds(
    output: Size<i32, Physical>,
    bar: Rectangle<i32, Physical>,
    panel: Panel,
) -> Rectangle<i32, Physical> {
    let (panel_w, panel_h) = panel.size();
    let scale = (f64::from(bar.size.h) / BAR_HEIGHT)
        .min(f64::from(output.w - 16) / f64::from(panel_w))
        .min(f64::from(output.h - bar.size.h - 16) / f64::from(panel_h));
    let width = ((f64::from(panel_w) * scale).round() as i32).max(1);
    let height = ((f64::from(panel_h) * scale).round() as i32).max(1);
    Rectangle::new(
        (output.w - width - 8, bar.size.h + 8).into(),
        (width, height).into(),
    )
}

pub fn panel_radius(bounds: Rectangle<i32, Physical>) -> f32 {
    PANEL_RADIUS * bounds.size.w as f32 / PANEL_WIDTH as f32
}

fn workspaces_fit(width: f64) -> bool {
    width >= 400.0
}

pub fn action_at(
    hits: (Rectangle<i32, Physical>, Rectangle<i32, Physical>),
    point: Point<f64, Logical>,
    panel: Panel,
    native: bool,
) -> Option<Action> {
    let (bar, popup) = hits;
    if bar.to_f64().to_logical(1.0).contains(point) {
        let scale = f64::from(bar.size.h) / BAR_HEIGHT;
        let width = f64::from(bar.size.w) / scale;
        let x = point.x / scale;
        let y = point.y / scale;
        if x >= width - SETTINGS_WIDTH {
            return Some(Action::Toggle);
        }
        if x < 158.0 {
            return Some(Action::Launch(Tool::Apps));
        }
        if width >= 450.0
            && (OVERVIEW_X..OVERVIEW_X + 36.0).contains(&x)
            && (5.0..37.0).contains(&y)
        {
            return Some(Action::Overview);
        }
        if workspaces_fit(width) && (6.0..36.0).contains(&y) {
            return (0..WORKSPACE_COUNT)
                .find(|index| {
                    let left = WORKSPACE_X + *index as f64 * WORKSPACE_STEP;
                    (left..left + 30.0).contains(&x)
                })
                .map(Action::Workspace);
        }
    } else if panel.open() && popup.to_f64().to_logical(1.0).contains(point) {
        let (panel_w, panel_h) = panel.size();
        let x = (point.x - f64::from(popup.loc.x)) * f64::from(panel_w) / f64::from(popup.size.w);
        let y = (point.y - f64::from(popup.loc.y)) * f64::from(panel_h) / f64::from(popup.size.h);
        if (12.0..300.0).contains(&x) {
            let offset = y - FIRST_ROW;
            if matches!(panel, Panel::Logout | Panel::Shutdown) {
                return match y {
                    y if (105.0..142.0).contains(&y) => Some(if panel == Panel::Logout {
                        Action::Logout
                    } else {
                        Action::Shutdown
                    }),
                    y if (151.0..188.0).contains(&y) => Some(Action::Cancel),
                    _ => None,
                };
            }
            if offset >= 0.0 && offset % ROW_HEIGHT < 37.0 {
                return match (panel, (offset / ROW_HEIGHT) as usize) {
                    (Panel::Power, 0) if native => Some(Action::Lock),
                    (Panel::Power, 1) if native => Some(Action::Shutdown),
                    (Panel::Power, 2) if native => Some(Action::Cancel),
                    (Panel::Settings, 0) => Some(Action::Launch(Tool::Network)),
                    (Panel::Settings, 1) => Some(Action::Launch(Tool::Bluetooth)),
                    (Panel::Settings, 2) => Some(Action::Launch(Tool::Sound)),
                    (Panel::Settings, 3) => Some(Action::Launch(Tool::Settings)),
                    (Panel::Settings, 4) if native => Some(Action::Power),
                    _ => None,
                };
            }
        }
    }
    None
}

fn paint(context: &Context, size: Size<i32, Physical>, view: &View) -> Result<(), CairoError> {
    let colors = view.theme.colors();
    let scale = f64::from(size.h) / BAR_HEIGHT;
    let width = f64::from(size.w) / scale;
    context.scale(scale, scale);
    let shade = LinearGradient::new(0.0, 0.0, 0.0, BAR_HEIGHT);
    let tint = crate::palette::mix(colors.surface, colors.accent, 0.17);
    let opacity = if view.solid { 1.0 } else { 0.91 };
    shade.add_color_stop_rgba(0.0, tint.0, tint.1, tint.2, opacity);
    shade.add_color_stop_rgba(1.0, colors.base.0, colors.base.1, colors.base.2, opacity);
    context.set_source(shade)?;
    context.paint()?;
    theme::ink(context, colors.accent, 0.26);
    context.rectangle(0.0, BAR_HEIGHT - 1.0, width, 1.0);
    context.fill()?;
    if view.hovered == Some(Action::Launch(Tool::Apps)) {
        theme::rounded(context, (6.0, 5.0, 150.0, 32.0), 8.0);
        theme::ink(context, colors.accent, 0.14);
        context.fill()?;
    }
    draw_brand(context, colors)?;

    if workspaces_fit(width) {
        for index in 0..WORKSPACE_COUNT {
            let x = WORKSPACE_X + index as f64 * WORKSPACE_STEP;
            let active = index == view.workspace;
            let hovered = view.hovered == Some(Action::Workspace(index));
            theme::rounded(context, (x, 6.0, 30.0, 30.0), 9.0);
            theme::ink(
                context,
                colors.accent,
                if active {
                    0.32
                } else if hovered {
                    0.22
                } else {
                    0.10
                },
            );
            context.fill_preserve()?;
            theme::ink(context, colors.accent, if active { 0.57 } else { 0.0 });
            context.set_line_width(1.0);
            context.stroke()?;
            if active {
                theme::rounded(context, (x + 10.0, 32.0, 10.0, 2.5), 1.2);
                theme::ink(context, colors.warning, 1.0);
                context.fill()?;
            }
            theme::ink(
                context,
                if active { colors.text } else { colors.muted },
                1.0,
            );
            theme::font(context, 17.0, FontWeight::Normal);
            centered(context, &(index + 1).to_string(), x + 15.0, 27.0)?;
        }
    }
    if width >= 450.0 {
        let hovered = view.hovered == Some(Action::Overview);
        if hovered || view.overview {
            theme::rounded(context, (OVERVIEW_X, 6.0, 36.0, 30.0), 9.0);
            theme::ink(
                context,
                colors.accent,
                if view.overview { 0.30 } else { 0.16 },
            );
            context.fill()?;
        }
        theme::ink(
            context,
            if view.overview {
                colors.text
            } else {
                colors.muted
            },
            1.0,
        );
        context.set_line_width(1.5);
        for (x, y, w, h) in [
            (299.0, 14.0, 8.0, 7.0),
            (311.0, 14.0, 8.0, 7.0),
            (299.0, 24.0, 20.0, 4.0),
        ] {
            theme::rounded(context, (x, y, w, h), 1.5);
        }
        context.stroke()?;
    }
    if width >= 820.0 {
        theme::ink(context, colors.muted, 1.0);
        theme::font(context, 14.0, FontWeight::Normal);
        centered(
            context,
            &format!("{}   ·   {}", view.status.date, view.status.clock),
            width / 2.0,
            26.0,
        )?;
    }

    if view.panel.open() || view.hovered == Some(Action::Toggle) {
        theme::rounded(
            context,
            (width - SETTINGS_WIDTH, 5.0, SETTINGS_WIDTH - 6.0, 32.0),
            8.0,
        );
        theme::ink(context, colors.accent, 0.16);
        context.fill()?;
    }
    context.set_line_cap(LineCap::Round);
    context.set_line_width(1.7);
    let usb = width - 91.0;
    theme::ink(
        context,
        if view.status.usb_storage {
            colors.warning
        } else {
            colors.muted
        },
        1.0,
    );
    context.move_to(usb, 28.0);
    context.line_to(usb, 13.0);
    context.rel_line_to(-3.0, 3.0);
    context.move_to(usb, 13.0);
    context.rel_line_to(3.0, 3.0);
    context.move_to(usb, 24.0);
    context.rel_line_to(-5.0, -3.0);
    context.rel_line_to(0.0, -3.0);
    context.move_to(usb, 24.0);
    context.rel_line_to(5.0, -3.0);
    context.rel_line_to(0.0, -3.0);
    context.stroke()?;

    let bluetooth = width - 68.0;
    theme::ink(
        context,
        if view.status.bluetooth == "Manage devices" {
            colors.accent
        } else {
            colors.muted
        },
        0.89,
    );
    context.move_to(bluetooth, 28.0);
    context.line_to(bluetooth, 14.0);
    context.rel_line_to(4.3, 3.8);
    context.rel_line_to(-8.6, 7.0);
    context.move_to(bluetooth - 4.3, 17.5);
    context.rel_line_to(8.6, 7.0);
    context.line_to(bluetooth, 28.0);
    context.stroke()?;

    let net = width - 42.0;
    theme::ink(
        context,
        if matches!(view.status.network, "Wi-Fi connected" | "Wired connection") {
            colors.accent
        } else {
            colors.muted
        },
        1.0,
    );
    if view.status.network == "Wired connection" {
        theme::rounded(context, (net - 7.0, 15.0, 14.0, 10.0), 2.0);
        context.move_to(net - 3.0, 28.0);
        context.line_to(net + 3.0, 28.0);
        context.stroke()?;
    } else {
        for radius in [5.0, 9.0, 13.0] {
            context.new_sub_path();
            context.arc(net, 27.0, radius, -2.27, -0.87);
        }
        context.stroke()?;
        theme::dot(context, net, 27.0, 1.1)?;
    }
    let arrow = width - 15.0;
    let tilt = if view.panel.open() { -1.0 } else { 1.0 };
    theme::ink(context, colors.text, 0.90);
    context.move_to(arrow - 4.5, 21.0 - 2.0 * tilt);
    context.line_to(arrow, 21.0 + 2.5 * tilt);
    context.line_to(arrow + 4.5, 21.0 - 2.0 * tilt);
    context.stroke()
}

fn centered(context: &Context, text: &str, x: f64, baseline: f64) -> Result<(), CairoError> {
    let extent = context.text_extents(text)?;
    context.move_to(x - extent.width() / 2.0 - extent.x_bearing(), baseline);
    context.show_text(text)
}

fn draw_brand(context: &Context, colors: crate::palette::Palette) -> Result<(), CairoError> {
    // Stroke coordinates from Vantage-OS kernel/ui/canvas.c (GPL-3.0).
    const WORDMARK: [(u8, u8, u8, u8); 24] = [
        (4, 8, 11, 24),
        (11, 24, 18, 8),
        (24, 24, 31, 8),
        (31, 8, 38, 24),
        (27, 18, 35, 18),
        (44, 24, 44, 8),
        (44, 8, 58, 24),
        (58, 24, 58, 8),
        (64, 8, 78, 8),
        (71, 8, 71, 24),
        (84, 24, 91, 8),
        (91, 8, 98, 24),
        (87, 18, 95, 18),
        (118, 8, 108, 8),
        (108, 8, 104, 12),
        (104, 12, 104, 20),
        (104, 20, 108, 24),
        (108, 24, 118, 24),
        (118, 24, 118, 16),
        (118, 16, 112, 16),
        (137, 8, 124, 8),
        (124, 8, 124, 24),
        (124, 16, 134, 16),
        (124, 24, 137, 24),
    ];
    context.set_line_cap(LineCap::Round);
    context.set_line_join(vte4::gtk::cairo::LineJoin::Round);
    context.set_line_width(2.0);
    theme::ink(context, colors.accent, 1.0);
    context.move_to(15.0, 27.0);
    for (x, y) in [(23.0, 15.0), (29.0, 23.0), (32.0, 18.0), (38.0, 27.0)] {
        context.line_to(x, y);
    }
    context.close_path();
    context.stroke()?;
    theme::ink(context, colors.text, 1.0);
    context.set_line_width(2.45);
    for (x, y, end_x, end_y) in WORDMARK {
        context.move_to(43.0 + f64::from(x) * 0.75, 9.0 + f64::from(y) * 0.75);
        context.line_to(
            43.0 + f64::from(end_x) * 0.75,
            9.0 + f64::from(end_y) * 0.75,
        );
    }
    context.stroke()
}

fn panel_row(
    context: &Context,
    view: &View,
    y: f64,
    action: Action,
    label: &str,
    detail: &str,
) -> Result<(), CairoError> {
    let colors = view.theme.colors();
    theme::glass_surface(
        context,
        (12.0, y, 288.0, 37.0),
        14.0,
        if view.hovered == Some(action) {
            colors.raised
        } else {
            colors.surface
        },
        if view.solid {
            0.96
        } else if view.hovered == Some(action) {
            0.77
        } else {
            0.65
        },
    )?;
    theme::font(context, 12.0, FontWeight::Bold);
    theme::ink(context, colors.text, 1.0);
    context.move_to(24.0, y + 23.0);
    context.show_text(label)?;
    theme::font(context, 11.0, FontWeight::Normal);
    theme::ink(context, colors.muted, 1.0);
    context.move_to(286.0 - context.text_extents(detail)?.x_advance(), y + 23.0);
    context.show_text(detail)
}

fn paint_panel(context: &Context, view: &View) -> Result<(), CairoError> {
    let colors = view.theme.colors();
    let (width, height) = view.panel.size();
    theme::glass_surface(
        context,
        (1.0, 1.0, f64::from(width - 2), f64::from(height - 2)),
        f64::from(PANEL_RADIUS),
        colors.base,
        if view.solid { 1.0 } else { 0.16 },
    )?;
    theme::font(context, 16.0, FontWeight::Bold);
    theme::ink(context, colors.text, 1.0);
    context.move_to(17.0, 29.0);
    if matches!(view.panel, Panel::Logout | Panel::Shutdown) {
        let shutdown = view.panel == Panel::Shutdown;
        context.show_text(if shutdown {
            "Shut down this computer?"
        } else {
            "Log out of Vantage?"
        })?;
        theme::font(context, 12.0, FontWeight::Normal);
        theme::ink(context, colors.muted, 1.0);
        context.move_to(17.0, 63.0);
        context.show_text("Save your work before ending this session.")?;
        panel_row(
            context,
            view,
            105.0,
            if shutdown {
                Action::Shutdown
            } else {
                Action::Logout
            },
            if shutdown { "Shut down" } else { "Log out" },
            if shutdown {
                "Power off"
            } else {
                "Return to sign-in"
            },
        )?;
        return panel_row(context, view, 151.0, Action::Cancel, "Cancel", "");
    }
    if view.panel == Panel::Power {
        context.show_text("Power")?;
        for (index, (action, label, detail)) in [
            (Action::Lock, "Lock screen", "F9"),
            (Action::Shutdown, "Shut down", "Power off"),
            (Action::Cancel, "Back", "Quick settings"),
        ]
        .into_iter()
        .enumerate()
        {
            panel_row(
                context,
                view,
                FIRST_ROW + index as f64 * ROW_HEIGHT,
                action,
                label,
                detail,
            )?;
        }
        if let Some(error) = view.error {
            theme::font(context, 11.0, FontWeight::Normal);
            theme::ink(context, colors.warning, 1.0);
            context.move_to(17.0, 190.0);
            context.show_text(error)?;
        }
        return Ok(());
    }
    context.show_text("Quick settings")?;
    for (index, (tool, label, detail)) in [
        (Tool::Network, "Network & Wi-Fi", view.status.network),
        (Tool::Bluetooth, "Bluetooth", view.status.bluetooth),
        (Tool::Sound, "Sound", "Output and input"),
        (Tool::Settings, "Settings", "Open all settings"),
    ]
    .into_iter()
    .enumerate()
    {
        panel_row(
            context,
            view,
            FIRST_ROW + index as f64 * ROW_HEIGHT,
            Action::Launch(tool),
            label,
            detail,
        )?;
    }
    if view.native {
        panel_row(
            context,
            view,
            FIRST_ROW + 4.0 * ROW_HEIGHT,
            Action::Power,
            "Power",
            "Lock or shut down",
        )?;
    }
    theme::font(context, 11.0, FontWeight::Normal);
    theme::ink(
        context,
        if view.error.is_some() {
            colors.warning
        } else {
            colors.muted
        },
        1.0,
    );
    context.move_to(17.0, 278.0);
    let usb = if view.status.usb_storage {
        "USB storage connected"
    } else {
        "No USB storage"
    };
    let detail = view.status.battery.map_or_else(
        || usb.to_owned(),
        |(level, charging)| {
            let battery = if charging { "Charging" } else { "Battery" };
            format!("{battery} {level}%    ·    {usb}")
        },
    );
    context.show_text(view.error.unwrap_or(&detail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_panel_and_shutdown_confirmation_use_visible_rows() {
        let bar = bounds(1280, 1.0);
        let popup = panel_bounds((1280, 720).into(), bar, Panel::Power);
        let hits = (bar, popup);
        let point = |y: f64| {
            (
                f64::from(popup.loc.x) + 20.0 * f64::from(popup.size.w) / f64::from(PANEL_WIDTH),
                f64::from(popup.loc.y)
                    + y * f64::from(popup.size.h) / f64::from(Panel::Power.size().1),
            )
                .into()
        };
        assert_eq!(
            action_at(hits, point(50.0), Panel::Power, true),
            Some(Action::Lock)
        );
        assert_eq!(
            action_at(hits, point(93.0), Panel::Power, true),
            Some(Action::Shutdown)
        );
        assert_eq!(
            action_at(hits, point(136.0), Panel::Power, true),
            Some(Action::Cancel)
        );
        assert_eq!(action_at(hits, point(179.0), Panel::Power, true), None);
        assert_eq!(
            action_at(hits, point(110.0), Panel::Shutdown, true),
            Some(Action::Shutdown)
        );
        assert_eq!(
            action_at(hits, point(160.0), Panel::Shutdown, true),
            Some(Action::Cancel)
        );
        assert_eq!(action_at(hits, point(50.0), Panel::Power, false), None);
    }

    #[test]
    fn top_bar_workspaces_and_quick_settings_use_visible_bounds() {
        let output = (1280, 720).into();
        let bar = bounds(1280, 1.5);
        let popup = panel_bounds(output, bar, Panel::Settings);
        let hits = (bar, popup);
        let network = (f64::from(popup.loc.x) + 30.0, f64::from(popup.loc.y) + 82.0).into();
        let power = (
            f64::from(popup.loc.x) + 30.0,
            f64::from(popup.loc.y) + 334.0,
        )
            .into();
        assert_eq!(
            action_at(hits, (335.0, 29.0).into(), Panel::Closed, true),
            Some(Action::Workspace(1))
        );
        assert_eq!(action_at(hits, network, Panel::Closed, true), None);
        assert_eq!(
            action_at(hits, network, Panel::Settings, true),
            Some(Action::Launch(Tool::Network))
        );
        assert_eq!(action_at(hits, power, Panel::Settings, false), None);
        assert_eq!(
            action_at(hits, power, Panel::Settings, true),
            Some(Action::Power)
        );
    }
}
