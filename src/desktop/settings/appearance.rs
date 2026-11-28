use vte4::gtk::{self, gdk, glib, prelude::*};

use super::{error_label, label, set_error};
use crate::palette::{DARK_COLORS, Mode, Theme};
use crate::preferences::{self, Aod, Effects};

pub(super) fn page(parent: &gtk::ApplicationWindow) -> gtk::ScrolledWindow {
    install_preview_style(parent);
    let content = super::page("Appearance");
    let effects = gtk::DropDown::from_strings(&["Full", "Balanced", "Reduced"]);
    effects.update_property(&[gtk::accessible::Property::Label("Effects")]);
    let current = preferences::appearance();
    effects.set_selected(match current.effects {
        Effects::Full => 0,
        Effects::Balanced => 1,
        Effects::Reduced => 2,
    });
    let transparency = gtk::CheckButton::with_label("Reduce transparency");
    transparency.set_active(current.reduce_transparency);
    let motion = gtk::CheckButton::with_label("Reduce motion");
    motion.set_active(current.reduce_motion);
    let error = error_label();
    let mode = gtk::DropDown::from_strings(&["Dark", "Light"]);
    mode.update_property(&[gtk::accessible::Property::Label("Color mode")]);
    mode.set_selected(u32::from(current.theme.mode == Mode::Light));
    let dark_colors = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .activate_on_single_click(true)
        .homogeneous(true)
        .min_children_per_line(3)
        .max_children_per_line(4)
        .column_spacing(8)
        .row_spacing(8)
        .build();
    dark_colors.add_css_class("color-grid");
    dark_colors.update_property(&[gtk::accessible::Property::Label("Dark color")]);
    populate_dark_colors(&dark_colors);
    select_dark_color(&dark_colors, current.theme);
    let dark_options = gtk::Box::new(gtk::Orientation::Vertical, 8);
    dark_options.append(&label("Dark color"));
    dark_options.append(&dark_colors);
    dark_options.set_visible(current.theme.mode == Mode::Dark);
    mode.connect_selected_notify({
        let dark_options = dark_options.clone();
        let dark_colors = dark_colors.clone();
        let error = error.clone();
        move |control| {
            let next = if control.selected() == 1 {
                Mode::Light
            } else {
                Mode::Dark
            };
            let mut value = preferences::appearance();
            if value.theme.mode == next {
                return;
            }
            value.theme = preferences::saved_theme(next);
            if save(value, &error) {
                dark_options.set_visible(next == Mode::Dark);
                select_dark_color(&dark_colors, value.theme);
            } else {
                control.set_selected(u32::from(next == Mode::Dark));
            }
        }
    });
    dark_colors.connect_child_activated({
        let error = error.clone();
        move |control, child| {
            let mut value = preferences::appearance();
            if value.theme.mode == Mode::Light {
                return;
            }
            let previous = value.theme;
            value.theme = Theme::dark(child.index() as usize);
            if save(value, &error) {
                control.select_child(child);
            } else {
                select_dark_color(control, previous);
            }
        }
    });
    content.append(&label("Color mode"));
    content.append(&mode);
    content.append(&dark_options);
    content.append(&error);
    effects.connect_selected_notify({
        let error = error.clone();
        move |control| {
            let mut value = preferences::appearance();
            value.effects = match control.selected() {
                0 => Effects::Full,
                1 => Effects::Balanced,
                _ => Effects::Reduced,
            };
            save(value, &error);
        }
    });
    transparency.connect_toggled({
        let error = error.clone();
        move |control| {
            let mut value = preferences::appearance();
            value.reduce_transparency = control.is_active();
            save(value, &error);
        }
    });
    motion.connect_toggled({
        let error = error.clone();
        move |control| {
            let mut value = preferences::appearance();
            value.reduce_motion = control.is_active();
            save(value, &error);
        }
    });
    content.append(&label("Effects"));
    content.append(&effects);
    content.append(&transparency);
    content.append(&motion);

    let title = label("Always-on display");
    title.add_css_class("title-2");
    content.append(&title);
    let aod = gtk::DropDown::from_strings(&["Off", "Clock only", "Full-screen wallpaper"]);
    aod.update_property(&[gtk::accessible::Property::Label("Always-on display")]);
    aod.set_selected(match current.aod {
        Aod::Off => 0,
        Aod::Classic => 1,
        Aod::Wallpaper => 2,
    });
    aod.connect_selected_notify({
        let error = error.clone();
        move |control| {
            let mut value = preferences::appearance();
            value.aod = match control.selected() {
                0 => Aod::Off,
                1 => Aod::Classic,
                _ => Aod::Wallpaper,
            };
            save(value, &error);
        }
    });
    content.append(&aod);
    let description = label("Choose what stays visible after the lock screen dims.");
    description.add_css_class("dim-label");
    content.append(&description);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    for (title, wallpaper) in [("Classic", false), ("Wallpaper", true)] {
        let button = gtk::Button::with_label(title);
        button.add_css_class("settings-row");
        button.set_hexpand(true);
        let parent = parent.downgrade();
        button.connect_clicked(move |_| {
            if let Some(parent) = parent.upgrade() {
                preview(&parent, wallpaper);
            }
        });
        buttons.append(&button);
    }
    content.append(&buttons);
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&content)
        .build()
}

fn select_dark_color(grid: &gtk::FlowBox, theme: Theme) {
    if theme.mode == Mode::Dark
        && let Some(child) = grid.child_at_index(theme.dark_index() as i32)
    {
        grid.select_child(&child);
    }
}

fn populate_dark_colors(grid: &gtk::FlowBox) {
    for (index, choice) in DARK_COLORS.iter().enumerate() {
        let theme = Theme::dark(index);
        let tint = theme.dark_tint();
        let accent = theme.colors().accent;
        let card = gtk::Box::new(gtk::Orientation::Horizontal, 9);
        let chip = gtk::DrawingArea::builder()
            .content_width(24)
            .content_height(24)
            .valign(gtk::Align::Center)
            .build();
        chip.set_draw_func(move |_, context, w, h| {
            use crate::shell::theme;
            theme::rounded(
                context,
                (1.0, 1.0, f64::from(w) - 2.0, f64::from(h) - 2.0),
                12.0,
            );
            let glaze = gtk::cairo::LinearGradient::new(0.0, 0.0, f64::from(w), f64::from(h));
            glaze.add_color_stop_rgb(0.0, accent.0, accent.1, accent.2);
            glaze.add_color_stop_rgb(1.0, tint.0, tint.1, tint.2);
            let _ = context.set_source(glaze);
            let _ = context.fill_preserve();
            theme::ink(context, accent, 0.85);
            context.set_line_width(1.0);
            let _ = context.stroke();
        });
        card.append(&chip);
        card.append(&label(choice.name));
        let entry = gtk::FlowBoxChild::new();
        entry.update_property(&[gtk::accessible::Property::Label(choice.name)]);
        entry.set_child(Some(&card));
        grid.append(&entry);
    }
}

fn save(value: preferences::Appearance, error: &gtk::Label) -> bool {
    let failure = preferences::set_appearance(value).err();
    let success = failure.is_none();
    set_error(
        error,
        failure
            .map(|reason| format!("Could not save appearance. {reason}"))
            .as_deref(),
    );
    success
}

fn preview(parent: &gtk::ApplicationWindow, wallpaper: bool) {
    let window = gtk::Window::builder()
        .title(if wallpaper {
            "Wallpaper AOD preview"
        } else {
            "Classic AOD preview"
        })
        .transient_for(parent)
        .modal(true)
        .default_width(760)
        .default_height(540)
        .build();
    window.add_css_class("aod-preview");
    let scene = gtk::Overlay::new();
    let mut scrim = None;
    if wallpaper {
        let bytes =
            glib::Bytes::from_static(crate::wallpaper::art(preferences::appearance().theme.mode));
        if let Ok(texture) = gdk::Texture::from_bytes(&bytes) {
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_content_fit(gtk::ContentFit::Cover);
            scene.set_child(Some(&picture));
            let shade = gtk::Box::new(gtk::Orientation::Vertical, 0);
            shade.add_css_class("aod-scrim");
            scene.add_overlay(&shade);
            scrim = Some(shade);
        }
    }
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 16);
    layout.set_margin_top(24);
    layout.set_margin_bottom(24);
    layout.set_margin_start(24);
    layout.set_margin_end(24);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let caption = label("Always-on display preview");
    caption.add_css_class("aod-caption");
    caption.set_hexpand(true);
    top.append(&caption);
    let close = gtk::Button::with_label("Close");
    close.connect_clicked({
        let window = window.downgrade();
        move |_| {
            if let Some(window) = window.upgrade() {
                window.close();
            }
        }
    });
    top.append(&close);
    layout.append(&top);
    let center = gtk::Box::new(gtk::Orientation::Vertical, 6);
    center.set_valign(gtk::Align::Center);
    center.set_vexpand(true);
    let clock = gtk::Label::new(Some("10:08"));
    clock.add_css_class("aod-clock");
    let date = gtk::Label::new(Some("Tuesday, September 17"));
    date.add_css_class("aod-date");
    center.append(&clock);
    center.append(&date);
    let wake_content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    wake_content.set_halign(gtk::Align::Center);
    wake_content.set_margin_top(20);
    wake_content.add_css_class("aod-wake");
    let account = gtk::Label::new(Some(&glib::user_name().to_string_lossy()));
    account.add_css_class("aod-account");
    let password = gtk::Box::new(gtk::Orientation::Horizontal, 100);
    password.add_css_class("aod-password");
    password.append(&gtk::Label::new(Some("Password")));
    password.append(&gtk::Image::from_icon_name("go-next-symbolic"));
    wake_content.append(&account);
    wake_content.append(&password);
    let reveal = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::Crossfade)
        .transition_duration(if preferences::appearance().reduce_motion {
            0
        } else {
            250
        })
        .child(&wake_content)
        .build();
    reveal.set_hexpand(true);
    reveal.set_vexpand(true);
    reveal.set_valign(gtk::Align::Center);
    layout.append(&center);
    let wake = gtk::Button::with_label("Preview wake");
    wake.set_halign(gtk::Align::Center);
    let sign_preview = reveal.clone();
    wake.connect_clicked(move |button| {
        let active = !sign_preview.reveals_child();
        sign_preview.set_reveal_child(active);
        center.set_opacity(f64::from(!active));
        if let Some(scrim) = &scrim {
            if active {
                scrim.add_css_class("aod-awake");
            } else {
                scrim.remove_css_class("aod-awake");
            }
        }
        button.set_label(if active {
            "Back to AOD"
        } else {
            "Preview wake"
        });
    });
    layout.append(&wake);
    scene.add_overlay(&reveal);
    scene.add_overlay(&layout);
    window.set_child(Some(&scene));
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let window = window.downgrade();
        move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if let Some(window) = window.upgrade() {
                    window.close();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    });
    window.add_controller(keys);
    window.present();
}

fn install_preview_style(parent: &gtk::ApplicationWindow) {
    let style = gtk::CssProvider::new();
    style.load_from_string(
        ".aod-preview { background: #030509; color: white; }
         .aod-scrim { background: rgba(0, 0, 0, 0.67); }
         .aod-scrim.aod-awake { background: rgba(0, 0, 0, 0.26); }
         .aod-clock { font-family: Roboto, sans-serif; font-size: 76px; font-weight: 900; color: #f5f7ff; }
         .aod-date { font-size: 17px; color: #d7deec; }
         .aod-caption { color: #d7deec; }
         .aod-wake { background: linear-gradient(rgba(90, 102, 128, 0.88), rgba(18, 28, 46, 0.88) 18%);\n                     border: 1px solid alpha(white, 0.40); border-radius: 27px; padding: 20px; }\n         .aod-account { font-size: 20px; font-weight: 700; color: white; }\n         .aod-password { border-radius: 18px; background: rgba(8, 16, 28, 0.82);\n                         padding: 9px 15px; color: white; }",
    );
    gtk::style_context_add_provider_for_display(
        &WidgetExt::display(parent),
        &style,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
