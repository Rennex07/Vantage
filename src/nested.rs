use std::{
    error::Error,
    time::{Duration, Instant},
};

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
    frame_metrics::FrameMetrics,
    session::{Scene, Server},
    shell::launch,
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
        WinitEvent::Resized { .. } => {
            state.redraw = true;
        }
        WinitEvent::Focus(focused) => state.host_focus(focused),
        WinitEvent::Input(event) => state.input(
            event,
            (
                |app, started| launch::open(app, started, &client_socket, &app_exits, None),
                |tool: crate::desktop::Tool| tool.open(&client_socket),
            ),
            None,
        ),
    })?;

    println!("WAYLAND_DISPLAY={}", server.socket.to_string_lossy());
    let mut metrics = FrameMetrics::new("nested");
    let mut next_frame: Option<Instant> = None;
    while !state.closed {
        let now = Instant::now();
        if next_frame.is_some_and(|deadline| now >= deadline) {
            state.redraw = true;
            next_frame = None;
        }
        server.dispatch(&mut state)?;
        let refresh = host_refresh(backend.window());
        state.resize_output(backend.window_size(), refresh);
        if std::mem::take(&mut state.redraw) {
            let size = backend.window_size();
            if size.w == 0 || size.h == 0 {
                state.dock_hits = None;
                state.status_hits = None;
                next_frame = None;
            } else {
                let start = metrics.start();
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
                let submit_start = metrics.start();
                backend.submit(Some(&[Rectangle::from_size(size)]))?;
                metrics.submitted(start, submit_start);
                state.frame_presented(Instant::now(), false);
                next_frame = scene
                    .is_animating(&state)
                    .then_some(now + frame_interval(refresh));
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

fn host_refresh(window: &Window) -> i32 {
    window
        .current_monitor()
        .and_then(|monitor| monitor.refresh_rate_millihertz())
        .and_then(|refresh| i32::try_from(refresh).ok())
        .filter(|&refresh| refresh > 0)
        .unwrap_or(60_000)
}

fn frame_interval(refresh: i32) -> Duration {
    Duration::from_nanos(1_000_000_000_000 / refresh as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_interval_tracks_host_refresh() {
        assert_eq!(frame_interval(60_000), Duration::from_nanos(16_666_666));
        assert_eq!(frame_interval(144_000), Duration::from_nanos(6_944_444));
    }
}
