use std::{cell::RefCell, ffi::OsStr, rc::Rc};

use vte4::gtk::{self, gio, glib, prelude::*};

use super::{error_label, label, set_error};

pub(super) fn page() -> gtk::Box {
    let content = super::page("Date & Time");
    let current = label("Reading system time zone…");
    let description = label("The system time zone controls the desktop and lock screen clocks.");
    description.add_css_class("dim-label");
    let zone_name = gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        None::<&gtk::Expression>,
        "string",
    );
    let zones = gtk::DropDown::builder()
        .expression(&zone_name)
        .enable_search(true)
        .search_match_mode(gtk::StringFilterMatchMode::Substring)
        .build();
    zones.update_property(&[gtk::accessible::Property::Label("Time zone")]);
    zones.set_sensitive(false);
    let apply = gtk::Button::with_label("Apply time zone");
    apply.set_halign(gtk::Align::Start);
    apply.set_sensitive(false);
    let error = error_label();
    content.append(&current);
    content.append(&description);
    content.append(&label("Time zone"));
    content.append(&zones);
    content.append(&apply);
    content.append(&error);

    let active = Rc::new(RefCell::new(String::new()));
    zones.connect_selected_item_notify({
        let active = Rc::clone(&active);
        let apply = apply.downgrade();
        move |zones| {
            let Some(apply) = apply.upgrade() else { return };
            apply.set_sensitive(
                selected_zone(zones).is_some_and(|zone| zone.as_str() != active.borrow().as_str()),
            )
        }
    });
    apply.connect_clicked({
        let apply = apply.downgrade();
        let zones = zones.clone();
        let current = current.clone();
        let error = error.clone();
        let active = Rc::clone(&active);
        move |_| {
            let Some(zone) = selected_zone(&zones) else {
                return;
            };
            let Some(apply) = apply.upgrade() else { return };
            set_error(&error, None);
            apply.set_sensitive(false);
            zones.set_sensitive(false);
            let (zones, current, error, active) = (
                zones.clone(),
                current.clone(),
                error.clone(),
                Rc::clone(&active),
            );
            glib::spawn_future_local(async move {
                match timedatectl(&["set-timezone", zone.as_str()]).await {
                    Ok(_) => {
                        current.set_label(&format!("Current time zone: {zone}"));
                        active.replace(zone.into());
                    }
                    Err(reason) => set_error(&error, Some(&reason)),
                }
                zones.set_sensitive(true);
                apply.set_sensitive(
                    selected_zone(&zones)
                        .is_some_and(|zone| zone.as_str() != active.borrow().as_str()),
                );
            });
        }
    });

    glib::spawn_future_local(async move {
        match load_zones().await {
            Ok((system_zone, names)) => {
                current.set_label(&format!("Current time zone: {system_zone}"));
                let selected = names.iter().position(|zone| *zone == system_zone);
                active.replace(system_zone);
                let values: Vec<_> = names.iter().map(String::as_str).collect();
                zones.set_model(Some(&gtk::StringList::new(&values)));
                zones.set_selected(
                    selected.map_or(gtk::INVALID_LIST_POSITION, |index| index as u32),
                );
                zones.set_sensitive(!names.is_empty());
                apply.set_sensitive(false);
            }
            Err(reason) => {
                current.set_label("System time zone unavailable");
                set_error(&error, Some(&reason));
            }
        }
    });
    content
}

fn selected_zone(zones: &gtk::DropDown) -> Option<glib::GString> {
    zones
        .selected_item()
        .and_downcast::<gtk::StringObject>()
        .map(|zone| zone.string())
}

async fn load_zones() -> Result<(String, Vec<String>), String> {
    let current = timedatectl(&["show", "--property=Timezone", "--value"]).await?;
    let available = timedatectl(&["list-timezones"]).await?;
    Ok((
        current.trim().to_owned(),
        available.lines().map(str::to_owned).collect(),
    ))
}

async fn timedatectl(args: &[&str]) -> Result<String, String> {
    let command: Vec<_> = ["timedatectl", "--no-pager"]
        .into_iter()
        .chain(args.iter().copied())
        .map(OsStr::new)
        .collect();
    let process = gio::Subprocess::newv(
        &command,
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE,
    )
    .map_err(|reason| reason.to_string())?;
    let (out, err) = process
        .communicate_utf8_future(None)
        .await
        .map_err(|reason| reason.to_string())?;
    if !process.is_successful() {
        return Err(err
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .unwrap_or("The system time service is unavailable.")
            .to_owned());
    }
    Ok(out.unwrap_or_default().into())
}
