use std::{error::Error, time::Instant};

use smithay::{
    backend::{
        renderer::gles::GlesRenderer,
        winit::{self, WinitEvent},
    },
    reexports::{
        calloop::EventLoop,
        winit::{dpi::LogicalSize, window::Window},
    },
    utils::{Rectangle, Transform},
};

use crate::{
    session::{Scene, Server},
    shell::{dock::FRAME_INTERVAL, launch},
    wayland::State,
};

pub fn run() -> Result<(), Box<dyn Error>> {
    let window = Window::default_attributes()
        .with_title("Vantage")
        .with_inner_size(LogicalSize::new(1280, 720))
        .with_min_inner_size(LogicalSize::new(320, 200));
    let (mut backend, events) = winit::init_from_attributes::<GlesRenderer>(window)?;
    let mut scene = {
        let (renderer, _) = backend.bind()?;
        Scene::new(renderer, false)?
    };

    let mut event_loop = EventLoop::<State>::try_new()?;
    let handle = event_loop.handle();
    let mut server = Server::new(&handle)?;
    let mut state = State::new(&server.handle(), backend.window_size())?;
    let client_socket = server.socket.clone();
    let app_exits = server.app_exits.clone();
    handle.insert_source(events, move |event, _, state| match event {
        WinitEvent::CloseRequested => state.closed = true,
        WinitEvent::Redraw => state.redraw = true,
        WinitEvent::Resized { size, .. } => {
            state.resize_output(size);
            state.redraw = true;
        }
        WinitEvent::Focus(focused) => state.host_focus(focused),
        WinitEvent::Input(event) => state.input(
            event,
            |app, started| launch::open(app, started, &client_socket, &app_exits),
            None,
        ),
    })?;

    println!("WAYLAND_DISPLAY={}", server.socket.to_string_lossy());
    let mut next_frame: Option<Instant> = None;
    while !state.closed {
        let now = Instant::now();
        if next_frame.is_some_and(|deadline| now >= deadline) {
            state.redraw = true;
            next_frame = None;
        }
        server.dispatch(&mut state)?;
        if std::mem::take(&mut state.redraw) {
            let size = backend.window_size();
            if size.w == 0 || size.h == 0 {
                state.dock_hits = None;
                state.status_hits = None;
                next_frame = None;
            } else {
                let scale = backend.window().scale_factor();
                {
                    let (renderer, mut target) = backend.bind()?;
                    let _ = scene.draw(
                        &mut state,
                        renderer,
                        &mut target,
                        size,
                        Transform::Flipped180,
                        scale,
                    )?;
                }
                backend.submit(Some(&[Rectangle::from_size(size)]))?;
                state.frame_presented(Instant::now());
                next_frame = scene.is_animating(&state).then_some(now + FRAME_INTERVAL);
            }
        }
        server.flush()?;
        event_loop.dispatch(
            next_frame.map(|deadline| deadline.saturating_duration_since(Instant::now())),
            &mut state,
        )?;
    }
    Ok(())
}
