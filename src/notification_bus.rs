use std::{cell::RefCell, error::Error, rc::Rc, time::Instant};

use vte4::gtk::{
    gio,
    glib::{self, variant::ToVariant},
};

use crate::notifications::{Action, Center, Event, Request};

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = r#"
<node>
  <interface name="org.freedesktop.Notifications">
    <method name="GetCapabilities">
      <arg type="as" direction="out"/>
    </method>
    <method name="Notify">
      <arg type="s" direction="in"/>
      <arg type="u" direction="in"/>
      <arg type="s" direction="in"/>
      <arg type="s" direction="in"/>
      <arg type="s" direction="in"/>
      <arg type="as" direction="in"/>
      <arg type="a{sv}" direction="in"/>
      <arg type="i" direction="in"/>
      <arg type="u" direction="out"/>
    </method>
    <method name="CloseNotification">
      <arg type="u" direction="in"/>
    </method>
    <method name="GetServerInformation">
      <arg type="s" direction="out"/>
      <arg type="s" direction="out"/>
      <arg type="s" direction="out"/>
      <arg type="s" direction="out"/>
    </method>
    <signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
    <signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
  </interface>
</node>
"#;

pub struct Service {
    context: glib::MainContext,
    connection: gio::DBusConnection,
    registration: Option<gio::RegistrationId>,
}

impl Service {
    pub fn start(center: Rc<RefCell<Center>>) -> Result<Self, Box<dyn Error>> {
        let context = glib::MainContext::new();
        let (connection, registration) = context.with_thread_default(|| {
            let address =
                gio::dbus_address_get_for_bus_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
            let connection = gio::DBusConnection::for_address_sync(
                &address,
                gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                    | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                None,
                gio::Cancellable::NONE,
            )?;
            connection.set_exit_on_close(false);
            let schema = gio::DBusNodeInfo::for_xml(INTERFACE)?;
            let interface = schema
                .lookup_interface(NAME)
                .ok_or("Missing notification interface")?;
            let registration = connection
                .register_object(PATH, &interface)
                .method_call(move |_, sender, _, _, method, parameters, invocation| {
                    let Some(sender) = sender else {
                        invocation.return_dbus_error(
                            "org.freedesktop.DBus.Error.AccessDenied",
                            "Missing sender",
                        );
                        return;
                    };
                    let mut center = center.borrow_mut();
                    let response = match method {
                        "GetCapabilities" => (vec!["body", "actions"],).to_variant(),
                        "GetServerInformation" => {
                            ("Vantage", "Vantage", env!("CARGO_PKG_VERSION"), "1.3").to_variant()
                        }
                        "Notify" => {
                            let actions = parameters.child_value(5);
                            let Ok(mut actions) = actions.array_iter_str() else {
                                invocation.return_dbus_error(
                                    "org.freedesktop.DBus.Error.InvalidArgs",
                                    "Invalid notification actions",
                                );
                                return;
                            };
                            let actions = std::iter::from_fn(|| {
                                Some(Action {
                                    key: actions.next()?.chars().take(129).collect(),
                                    label: actions.next()?.chars().take(30).collect(),
                                })
                            })
                            .take(8)
                            .collect();
                            let id = center.notify(
                                sender,
                                Request {
                                    replaces: parameters.child_get(1),
                                    app: read_text(&parameters, 0, 40),
                                    summary: read_text(&parameters, 3, 96),
                                    body: read_text(&parameters, 4, 200),
                                    actions,
                                    timeout_ms: parameters.child_get(7),
                                },
                                Instant::now(),
                            );
                            (id,).to_variant()
                        }
                        "CloseNotification" if center.close(sender, parameters.child_get(0)) => {
                            ().to_variant()
                        }
                        "CloseNotification" => {
                            invocation.return_dbus_error(
                                "org.freedesktop.DBus.Error.InvalidArgs",
                                "Unknown active notification",
                            );
                            return;
                        }
                        _ => {
                            invocation.return_dbus_error(
                                "org.freedesktop.DBus.Error.UnknownMethod",
                                "Unknown notification method",
                            );
                            return;
                        }
                    };
                    invocation.return_value(Some(&response));
                })
                .build()?;
            let acquired = connection
                .call_sync(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "RequestName",
                    Some(&(NAME, 4_u32).to_variant()), // DO_NOT_QUEUE
                    None,
                    gio::DBusCallFlags::NONE,
                    1000,
                    gio::Cancellable::NONE,
                )
                .and_then(|value| {
                    value.get::<(u32,)>().ok_or_else(|| {
                        glib::Error::new(gio::IOErrorEnum::InvalidData, "Invalid RequestName reply")
                    })
                });
            match acquired {
                Ok((1 | 4,)) => Ok((connection, Some(registration))),
                result => {
                    let _ = connection.unregister_object(registration);
                    result?;
                    Err::<_, Box<dyn Error>>(
                        "Another service owns org.freedesktop.Notifications".into(),
                    )
                }
            }
        })??;
        Ok(Self {
            context,
            connection,
            registration,
        })
    }

    pub fn dispatch(&self, center: &Rc<RefCell<Center>>, now: Instant) -> bool {
        let before = center.borrow().revision();
        center.borrow_mut().tick(now);
        // Limit GLib dispatch work in each compositor tick.
        for _ in 0..8 {
            if !self.context.iteration(false) {
                break;
            }
        }
        let mut center = center.borrow_mut();
        for event in center.take_events() {
            let (owner, signal, value) = match event {
                Event::Closed { owner, id, reason } => {
                    (owner, "NotificationClosed", (id, reason).to_variant())
                }
                Event::Invoked { owner, id, key } => {
                    (owner, "ActionInvoked", (id, key).to_variant())
                }
            };
            let _ = self
                .connection
                .emit_signal(Some(&owner), PATH, NAME, signal, Some(&value));
        }
        center.revision() != before
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        if let Some(registration) = self.registration.take() {
            let _ = self.connection.unregister_object(registration);
        }
    }
}

fn read_text(parameters: &glib::Variant, index: usize, limit: usize) -> String {
    parameters
        .child_value(index)
        .str()
        .unwrap_or_default()
        .chars()
        .take(limit)
        .collect()
}
