use smithay::{
    backend::{
        input::{
            AbsolutePositionEvent, ButtonState, InputEvent, KeyState, KeyboardKeyEvent,
            PointerButtonEvent,
        },
        renderer::{Frame, Renderer, gles::GlesRenderer},
        winit::{self, WinitEvent},
    },
    reexports::winit::{
        raw_window_handle::{HasWindowHandle, RawWindowHandle},
        window::Window,
    },
    utils::{Rectangle, Transform},
};
use std::{error::Error, thread, time::Duration};
use x11rb::{
    CURRENT_TIME,
    protocol::xproto::{ConnectionExt, InputFocus},
};

const CURSOR: [&str; 19] = [
    "#...............",
    "##..............",
    "#W#.............",
    "#WW#............",
    "#WWW#...........",
    "#WWWW#..........",
    "#WWWWW#.........",
    "#WWWWWW#........",
    "#WWWWWWW#.......",
    "#WWWWWWWW#......",
    "#WWWWWWWWW#.....",
    "#WWWWWWWWWW#....",
    "#WWWWWW#####....",
    "#WWW#WW#........",
    "#WW#.#WW#.......",
    "#W#..#WW#.......",
    "##....#WW#......",
    "#.....#WW#......",
    ".......##.......",
];

fn main() -> Result<(), Box<dyn Error>> {
    let window = Window::default_attributes()
        .with_title("Vantage input test")
        .with_decorations(false);
    let (mut backend, mut events) = winit::init_from_attributes::<GlesRenderer>(window)?;
    let window = backend.window();
    let monitor = window
        .primary_monitor()
        .ok_or("No display for the input test")?;
    window.set_outer_position(monitor.position());
    let _ = window.request_inner_size(monitor.size());
    window.set_cursor_visible(false);
    if let RawWindowHandle::Xlib(handle) = window.window_handle()?.as_raw() {
        let (connection, _) = x11rb::connect(None)?;
        connection
            .set_input_focus(InputFocus::POINTER_ROOT, handle.window as u32, CURRENT_TIME)?
            .check()?;
    }
    let mut pointer = (100, 100);
    let mut buttons = [false; 3];
    let mut closed = false;
    loop {
        events.dispatch_new_events(|event| match event {
            WinitEvent::CloseRequested => closed = true,
            WinitEvent::Focus(false) => buttons.fill(false),
            WinitEvent::Input(InputEvent::Keyboard { event }) => {
                if event.key_code().raw() == 9 && event.state() == KeyState::Pressed {
                    closed = true;
                }
            }
            WinitEvent::Input(InputEvent::PointerMotionAbsolute { event }) => {
                pointer = (event.x() as i32, event.y() as i32);
            }
            WinitEvent::Input(InputEvent::PointerButton { event }) => {
                let index = match event.button_code() {
                    0x110 => Some(0),
                    0x111 => Some(1),
                    0x112 => Some(2),
                    _ => None,
                };
                if let Some(index) = index {
                    buttons[index] = event.state() == ButtonState::Pressed;
                }
            }
            _ => {}
        });
        if closed {
            break;
        }
        let size = backend.window_size();
        if size.w == 0 || size.h == 0 {
            continue;
        }
        let bounds = Rectangle::from_size(size);
        {
            let (renderer, mut framebuffer) = backend.bind()?;
            let mut frame = renderer.render(&mut framebuffer, size, Transform::Flipped180)?;
            frame.clear([0.0, 0.0, 0.0, 1.0].into(), &[bounds])?;
            for y in -24_i32..=24 {
                let width = if buttons[0] {
                    ((24 * 24 - y * y) as f64).sqrt() as i32
                } else if buttons[1] && y.abs() <= 19 {
                    19
                } else if buttons[2] && (-22..=20).contains(&y) {
                    (y + 22) / 2
                } else {
                    continue;
                };
                let rect = Rectangle::new(
                    (pointer.0 - width, pointer.1 + y).into(),
                    (width * 2 + 1, 1).into(),
                );
                if let Some(visible) = rect.intersection(bounds) {
                    frame.clear([0.15, 0.55, 1.0, 1.0].into(), &[visible])?;
                }
            }
            for (y, row) in CURSOR.iter().enumerate() {
                for (x, pixel) in row.chars().enumerate() {
                    let color = match pixel {
                        '#' => [0.0, 0.0, 0.0, 1.0],
                        'W' => [1.0, 1.0, 1.0, 1.0],
                        _ => continue,
                    };
                    let rect = Rectangle::new(
                        (pointer.0 + x as i32 * 2, pointer.1 + y as i32 * 2).into(),
                        (2, 2).into(),
                    );
                    if let Some(visible) = rect.intersection(bounds) {
                        frame.clear(color.into(), &[visible])?;
                    }
                }
            }
            let _ = frame.finish()?;
        }
        backend.submit(Some(&[bounds]))?;
        thread::sleep(Duration::from_millis(16));
    }
    Ok(())
}
