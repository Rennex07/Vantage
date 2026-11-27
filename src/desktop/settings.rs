use std::{
    cell::{Cell, RefCell},
    ffi::OsStr,
    rc::Rc,
    time::Duration,
};

use serde_json::Value;
use vte4::gtk::{self, gdk, gio, glib, prelude::*};

use super::Tool;

mod appearance;

pub(super) fn show(app: &gtk::Application, sound_direct: bool) {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_icon_theme_name(Some("Adwaita"));
    }
    let pages = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(160)
        .build();
    let switcher = gtk::StackSwitcher::builder()
        .stack(&pages)
        .halign(gtk::Align::Center)
        .build();
    let header = gtk::HeaderBar::builder()
        .title_widget(&switcher)
        .decoration_layout(":minimize,maximize,close")
        .build();
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Settings")
        .titlebar(&header)
        .default_width(580)
        .default_height(450)
        .child(&pages)
        .build();
    window.add_css_class("vantage-settings");
    let connectivity = page("Connectivity");
    let issue = error_label();
    connectivity.append(&tool_button(
        "Network & Wi-Fi",
        "network-wireless-symbolic",
        Tool::Network,
        &issue,
    ));
    connectivity.append(&tool_button(
        "Bluetooth devices",
        "bluetooth-active-symbolic",
        Tool::Bluetooth,
        &issue,
    ));
    connectivity.append(&issue);
    pages.add_titled(&connectivity, Some("connectivity"), "Connectivity");
    let audio = Sound::page();
    pages.add_titled(&audio, Some("sound"), "Sound");
    pages.add_titled(&display_page(&window), Some("display"), "Display");
    pages.add_titled(&appearance::page(&window), Some("appearance"), "Appearance");
    if sound_direct {
        pages.set_visible_child_name("sound");
    }
    super::install_style(&window);
    window.present();
}

fn page(title: &str) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 14);
    page.set_margin_top(24);
    page.set_margin_bottom(22);
    page.set_margin_start(24);
    page.set_margin_end(24);
    let heading = label(title);
    heading.add_css_class("title-1");
    page.append(&heading);
    page
}

fn label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build()
}

fn error_label() -> gtk::Label {
    let label = label("");
    label.add_css_class("error");
    label.set_visible(false);
    label
}

fn set_error(label: &gtk::Label, error: Option<&str>) {
    label.set_label(error.unwrap_or_default());
    label.set_visible(error.is_some());
}

fn tool_button(title: &str, icon: &str, tool: Tool, error: &gtk::Label) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    content.append(&gtk::Image::from_icon_name(icon));
    let title = label(title);
    title.set_hexpand(true);
    content.append(&title);
    content.append(&gtk::Image::from_icon_name("go-next-symbolic"));
    let button = gtk::Button::builder()
        .child(&content)
        .css_classes(["settings-row"])
        .build();
    let error = error.clone();
    button.connect_clicked(move |_| {
        let launched = std::env::var_os("WAYLAND_DISPLAY")
            .ok_or_else(|| std::io::Error::other("Wayland display is unavailable"))
            .and_then(|socket| tool.open(&socket));
        set_error(
            &error,
            launched.err().map(|reason| reason.to_string()).as_deref(),
        );
    });
    button
}

fn slider(low: f64) -> gtk::Scale {
    let slider = gtk::Scale::with_range(gtk::Orientation::Horizontal, low, 100.0, 1.0);
    slider.set_draw_value(true);
    slider.set_format_value_func(|_, percent| format!("{percent:.0}%"));
    slider
}

fn display_page(window: &gtk::ApplicationWindow) -> gtk::Box {
    let page = page("Display");
    let details = label("");
    details.add_css_class("dim-label");
    let monitors = WidgetExt::display(window).monitors();
    update_monitor_details(&monitors, &details);
    let details_ref = details.downgrade();
    monitors.connect_items_changed(move |monitors, _, _, _| {
        if let Some(details) = details_ref.upgrade() {
            update_monitor_details(monitors, &details);
        }
    });
    page.append(&details);
    let brightness_label = label("Desktop brightness");
    let control = slider(35.0);
    brightness_label.set_mnemonic_widget(Some(&control));
    control.update_property(&[gtk::accessible::Property::Label("Desktop brightness")]);
    control.set_value(f64::from(crate::preferences::brightness()));
    let description =
        label("Dims the desktop in software. Your monitor's backlight uses its own controls.");
    description.add_css_class("dim-label");
    let error = error_label();
    let error_ref = error.clone();
    control.connect_value_changed(move |control| {
        let failure = crate::preferences::set_brightness(control.value().round() as u8).err();
        set_error(
            &error_ref,
            failure
                .map(|reason| format!("Could not save brightness. {reason}"))
                .as_deref(),
        );
    });
    page.append(&brightness_label);
    page.append(&control);
    page.append(&description);
    page.append(&error);
    page
}

fn update_monitor_details(monitors: &gio::ListModel, label: &gtk::Label) {
    let details = monitors
        .iter::<gdk::Monitor>()
        .flatten()
        .map(|monitor| {
            let area = monitor.geometry();
            let scale = monitor.scale_factor();
            let name = monitor.model().unwrap_or_else(|| "Display".into());
            let refresh = f64::from(monitor.refresh_rate()) / 1000.0;
            format!(
                "{name}   |   {} x {}   |   {refresh:.2} Hz",
                area.width() * scale,
                area.height() * scale
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    label.set_label(&details);
}

struct Sink {
    name: String,
    description: String,
    volume: f64,
    muted: bool,
}

struct Sound {
    outputs: gtk::DropDown,
    volume: gtk::Scale,
    mute: gtk::CheckButton,
    error: gtk::Label,
    sinks: RefCell<Vec<Sink>>,
    updating: Cell<bool>,
    busy: Cell<bool>,
    change: RefCell<Option<glib::SourceId>>,
}

impl Sound {
    fn page() -> gtk::Box {
        let page = page("Sound");
        let outputs = gtk::DropDown::from_strings(&[]);
        let volume = slider(0.0);
        let mute = gtk::CheckButton::with_label("Mute");
        let refresh = gtk::Button::with_label("Refresh devices");
        refresh.add_css_class("refresh");
        let error = error_label();
        let panel = Rc::new(Self {
            outputs,
            volume,
            mute,
            error,
            sinks: RefCell::default(),
            updating: Cell::new(false),
            busy: Cell::new(false),
            change: RefCell::default(),
        });
        panel
            .outputs
            .update_property(&[gtk::accessible::Property::Label("Output device")]);
        panel
            .volume
            .update_property(&[gtk::accessible::Property::Label("Volume")]);
        page.append(&label("Output device"));
        page.append(&panel.outputs);
        page.append(&label("Volume"));
        page.append(&panel.volume);
        page.append(&panel.mute);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        footer.append(&refresh);
        footer.append(&tool_button(
            "Advanced mixer",
            "audio-volume-high-symbolic",
            Tool::Mixer,
            &panel.error,
        ));
        page.append(&footer);
        page.append(&panel.error);
        // Keep the controller for the page lifetime; individual widgets hold weak references.
        page.connect_map({
            let panel = panel.clone();
            move |_| panel.refresh()
        });
        refresh.connect_clicked({
            let panel = Rc::downgrade(&panel);
            move |_| {
                if let Some(panel) = panel.upgrade() {
                    panel.refresh();
                }
            }
        });
        panel.outputs.connect_selected_notify({
            let panel = Rc::downgrade(&panel);
            move |_| {
                if let Some(panel) = panel.upgrade()
                    && !panel.updating.get()
                    && let Some(name) = panel.sink_name()
                {
                    panel.cancel_volume_change();
                    panel.run_change(vec!["set-default-sink".into(), name], true);
                }
            }
        });
        panel.mute.connect_toggled({
            let panel = Rc::downgrade(&panel);
            move |mute| {
                if let Some(panel) = panel.upgrade()
                    && !panel.updating.get()
                    && let Some(name) = panel.sink_name()
                {
                    panel.run_change(
                        vec![
                            "set-sink-mute".into(),
                            name,
                            u8::from(mute.is_active()).to_string(),
                        ],
                        false,
                    );
                }
            }
        });
        panel.volume.connect_value_changed({
            let panel = Rc::downgrade(&panel);
            move |_| {
                if let Some(panel) = panel.upgrade()
                    && !panel.updating.get()
                {
                    panel.schedule_volume_change();
                }
            }
        });
        panel.enable_controls(false);
        page
    }

    fn sink_name(&self) -> Option<String> {
        self.sinks
            .borrow()
            .get(self.outputs.selected() as usize)
            .map(|sink| sink.name.clone())
    }

    fn enable_controls(&self, enabled: bool) {
        self.outputs.set_sensitive(enabled);
        self.volume.set_sensitive(enabled);
        self.mute.set_sensitive(enabled);
    }

    fn cancel_volume_change(&self) {
        if let Some(previous) = self.change.borrow_mut().take() {
            previous.remove();
        }
    }

    fn schedule_volume_change(self: &Rc<Self>) {
        self.cancel_volume_change();
        let Some(name) = self.sink_name() else { return };
        let command = vec![
            "set-sink-volume".into(),
            name,
            format!("{:.0}%", self.volume.value()),
        ];
        let panel = Rc::downgrade(self);
        let update = glib::timeout_add_local_once(Duration::from_millis(110), move || {
            if let Some(panel) = panel.upgrade() {
                panel.change.borrow_mut().take();
                panel.run_change(command, false);
            }
        });
        self.change.replace(Some(update));
    }

    fn refresh(self: &Rc<Self>) {
        if self.busy.replace(true) {
            return;
        }
        self.enable_controls(false);
        let panel = self.clone();
        glib::spawn_future_local(async move {
            panel.reload().await;
        });
    }

    async fn reload(&self) {
        let result = read_sinks().await;
        self.updating.set(true);
        match result {
            Ok((sinks, selected)) => {
                let names: Vec<_> = sinks.iter().map(|sink| sink.description.as_str()).collect();
                self.outputs.set_model(Some(&gtk::StringList::new(&names)));
                if let Some(sink) = sinks.get(selected) {
                    self.outputs.set_selected(selected as u32);
                    self.volume.set_value(sink.volume);
                    self.mute.set_active(sink.muted);
                    self.enable_controls(true);
                } else {
                    self.outputs.set_selected(gtk::INVALID_LIST_POSITION);
                }
                set_error(
                    &self.error,
                    sinks.is_empty().then_some("No audio output is available."),
                );
                self.sinks.replace(sinks);
            }
            Err(reason) => set_error(&self.error, Some(&reason)),
        }
        self.updating.set(false);
        self.busy.set(false);
    }

    fn run_change(self: &Rc<Self>, args: Vec<String>, reload: bool) {
        if reload {
            if self.busy.replace(true) {
                return;
            }
            self.enable_controls(false);
        }
        let panel = self.clone();
        glib::spawn_future_local(async move {
            let result = pactl(&args.iter().map(String::as_str).collect::<Vec<_>>()).await;
            if reload {
                panel.reload().await;
            }
            if let Err(reason) = result {
                set_error(&panel.error, Some(&reason));
            } else if !reload {
                set_error(&panel.error, None);
            }
        });
    }
}

async fn pactl(args: &[&str]) -> Result<String, String> {
    let command: Vec<_> = ["pactl", "-f", "json", "--"]
        .iter()
        .copied()
        .chain(args.iter().copied())
        .map(OsStr::new)
        .collect();
    let process = gio::Subprocess::newv(
        &command,
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE,
    )
    .map_err(|reason| format!("Could not open audio controls. {reason}"))?;
    let (out, err) = process
        .communicate_utf8_future(None)
        .await
        .map_err(|reason| reason.to_string())?;
    if !process.is_successful() {
        let message = err
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .unwrap_or("Audio service is unavailable.");
        return Err(message.to_string());
    }
    Ok(out.unwrap_or_default().into())
}

async fn read_sinks() -> Result<(Vec<Sink>, usize), String> {
    let preferred = pactl(&["get-default-sink"]).await?;
    // The default name is plain text even with the JSON output option.
    let preferred = preferred.trim();
    let data = pactl(&["list", "sinks"]).await?;
    let data: Vec<Value> = serde_json::from_str(&data).map_err(|reason| reason.to_string())?;
    let sinks: Vec<_> = data
        .iter()
        .filter_map(|sink| {
            let name = sink.get("name")?.as_str()?;
            let description = sink
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or(name);
            let volume = sink
                .get("volume")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|channels| channels.values())
                .filter_map(|channel| {
                    channel
                        .get("value_percent")?
                        .as_str()?
                        .trim_end_matches('%')
                        .parse::<u32>()
                        .ok()
                })
                .max()
                .unwrap_or(0);
            Some(Sink {
                name: name.into(),
                description: description.into(),
                volume: f64::from(volume.min(100)),
                muted: sink
                    .get("mute")
                    .and_then(Value::as_bool)
                    .unwrap_or_default(),
            })
        })
        .collect();
    let selected = sinks
        .iter()
        .position(|sink| sink.name == preferred)
        .unwrap_or(0);
    Ok((sinks, selected))
}
