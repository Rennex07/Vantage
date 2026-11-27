use std::{
    io,
    os::{fd::OwnedFd, unix::net::UnixStream},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

use smithay::{
    delegate_session_lock,
    desktop::utils::send_frames_surface_tree,
    input::pointer::MotionEvent,
    reexports::wayland_server::protocol::{wl_output::WlOutput, wl_surface::WlSurface},
    utils::SERIAL_COUNTER,
    wayland::session_lock::{
        LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
    },
};

use super::{ClientState, State};
use crate::{
    preferences::Aod,
    shell::{
        status::Panel,
        timeline::{Ease, Transition, progress},
    },
};

const DIM_AFTER_LOCK: Duration = Duration::from_secs(6);
const DIM_AFTER_INPUT: Duration = Duration::from_secs(18);
const DIM_DURATION: Duration = Duration::from_millis(750);
const WAKE_DURATION: Duration = Duration::from_millis(310);
const UNLOCK_DURATION: Duration = Duration::from_millis(360);

struct Idle {
    dim_at: Instant,
    waking: Option<Transition>,
    dim_started: bool,
}

impl Idle {
    fn amount(&self, now: Instant, reduced: bool) -> f64 {
        if now >= self.dim_at {
            return Ease::Smooth.sample(progress(
                now,
                self.dim_at,
                if reduced {
                    Duration::ZERO
                } else {
                    DIM_DURATION
                },
            ));
        }
        if reduced {
            return 0.0;
        }
        self.waking.as_ref().map_or(0.0, |wake| wake.value(now))
    }

    fn wake(&mut self, now: Instant, reduced: bool) {
        let from = self.amount(now, reduced);
        self.dim_at = now + DIM_AFTER_INPUT;
        self.dim_started = false;
        self.waking = (from > 0.0 && !reduced)
            .then(|| Transition::new(from, 0.0, now, WAKE_DURATION, Ease::Out));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LockPhase {
    Desktop,
    Covering,
    Covered,
}

pub(super) struct Lock {
    manager: SessionLockManagerState,
    phase: LockPhase,
    pub(super) suspend_pending: bool,
    suspend_job: Option<Child>,
    pending: Option<SessionLocker>,
    surface: Option<LockSurface>,
    child: Option<Child>,
    poweroff: Option<Child>,
    idle: Idle,
    covered_at: Option<Instant>,
    unlocked_at: Option<Instant>,
}

impl Lock {
    pub fn new(display: &smithay::reexports::wayland_server::DisplayHandle) -> Self {
        Self {
            manager: SessionLockManagerState::new::<State, _>(display, |client| {
                client
                    .get_data::<ClientState>()
                    .is_some_and(|data| data.locker)
            }),
            phase: LockPhase::Desktop,
            suspend_pending: false,
            suspend_job: None,
            pending: None,
            surface: None,
            child: None,
            poweroff: None,
            idle: Idle {
                dim_at: Instant::now(),
                waking: None,
                dim_started: false,
            },
            covered_at: None,
            unlocked_at: None,
        }
    }
}

pub(super) fn local_password_missing() -> bool {
    let Ok(result) = Command::new("passwd").arg("-S").env("LC_ALL", "C").output() else {
        return false;
    };
    result.status.success()
        && String::from_utf8_lossy(&result.stdout)
            .split_whitespace()
            .nth(1)
            .is_some_and(|status| matches!(status, "L" | "NP"))
}

impl State {
    pub(super) fn start_locker(&mut self) -> io::Result<()> {
        self.check_locker();
        if self.lock.child.is_some() {
            return Ok(());
        }
        let (server, client) = UnixStream::pair()?;
        // Regular applications cannot bind the lock global. Pass swaylock its own connection.
        let fd: OwnedFd = client.into();
        let mut child = Command::new("swaylock")
            .env("WAYLAND_SOCKET", "0")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::from(fd))
            .args([
                "--config",
                "/dev/null",
                "--ignore-empty-password",
                "--indicator-caps-lock",
                "--font",
                "Roboto",
                "--indicator-thickness",
                "5",
                "--indicator-radius",
                "47",
                "--color",
                "00000000",
                "--inside-color",
                "0c152be6",
                "--inside-wrong-color",
                "391928e6",
                "--inside-ver-color",
                "133550e6",
                "--ring-color",
                "81bdf1b8",
                "--ring-wrong-color",
                "ffa6a6",
                "--ring-ver-color",
                "8cd7f5",
                "--line-uses-inside",
                "--separator-color",
                "00000000",
                "--key-hl-color",
                "e6f7ff",
                "--bs-hl-color",
                "efbb83",
                "--text-color",
                "eaf3ff",
                "--text-wrong-color",
                "ffffff",
                "--text-ver-color",
                "ffffff",
            ])
            .spawn()?;
        if let Err(error) = self.display.insert_client(
            server,
            Arc::new(ClientState {
                locker: true,
                ..Default::default()
            }),
        ) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        self.lock.child = Some(child);
        if !self.lock_active() {
            self.cover_desktop();
        }
        Ok(())
    }

    pub(super) fn check_locker(&mut self) {
        if self
            .lock
            .child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(Some(_))))
        {
            self.lock.child = None;
            self.lock.pending = None;
            self.lock.surface = None;
            if self.lock_active() {
                self.update_lock_focus();
            }
            if self.lock.suspend_pending {
                self.lock.suspend_pending = false;
                self.show_suspend_error();
            }
            self.redraw |= self.lock_active();
        }
        if let Some(job) = &mut self.lock.suspend_job
            && let Ok(Some(status)) = job.try_wait()
        {
            self.lock.suspend_job = None;
            if !status.success() {
                self.show_suspend_error();
            }
        }
        if let Some(job) = &mut self.lock.poweroff
            && let Ok(Some(status)) = job.try_wait()
        {
            self.lock.poweroff = None;
            if !status.success() {
                self.poweroff_failed();
            }
        }
        if self.lock_active() && self.lock.covered_at.is_some() && self.appearance.aod != Aod::Off {
            let now = Instant::now();
            if now >= self.lock.idle.dim_at && !self.lock.idle.dim_started {
                self.lock.idle.dim_started = true;
                self.redraw = true;
            }
        }
    }

    pub(super) fn shutdown(&mut self) {
        if self.lock.poweroff.is_some() {
            return;
        }
        self.status_panel = Panel::Closed;
        match Command::new("systemctl")
            .args(["--no-ask-password", "--check-inhibitors=yes", "poweroff"])
            .spawn()
        {
            Ok(job) => self.lock.poweroff = Some(job),
            Err(error) => {
                eprintln!("Could not shut down: {error}");
                self.poweroff_failed();
            }
        }
    }

    fn poweroff_failed(&mut self) {
        self.status_error = Some("Shutdown was blocked or denied.");
        if !self.lock_active() {
            self.status_panel = Panel::Power;
        }
        self.redraw = true;
    }

    fn show_suspend_error(&mut self) {
        self.status_error = Some("Could not suspend this system.");
        if !self.lock_active() {
            self.status_panel = Panel::Power;
        }
        self.redraw = true;
    }

    pub fn lock_active(&self) -> bool {
        self.lock.phase != LockPhase::Desktop
    }

    pub fn lock_amounts(&self, now: Instant) -> (f32, f32) {
        let reduced = self.appearance.reduce_motion;
        let reveal = self.lock.covered_at.map_or(0.0, |start| {
            Ease::Out.sample(progress(
                now,
                start,
                if reduced {
                    Duration::ZERO
                } else {
                    WAKE_DURATION
                },
            ))
        });
        let dim = if self.appearance.aod == Aod::Off {
            0.0
        } else {
            self.lock.idle.amount(now, reduced)
        };
        (reveal as f32, dim as f32)
    }

    pub fn lock_animating(&self, now: Instant) -> bool {
        let (reveal, dim) = self.lock_amounts(now);
        self.lock.covered_at.is_some()
            && ((reveal < 1.0 && !self.appearance.reduce_motion) || (dim > 0.0 && dim < 1.0))
    }

    pub fn unlock_amount(&self, now: Instant) -> f32 {
        if self.appearance.reduce_motion {
            return 0.0;
        }
        self.lock.unlocked_at.map_or(0.0, |start| {
            (1.0 - Ease::Out.sample(progress(now, start, UNLOCK_DURATION))) as f32
        })
    }

    pub(super) fn wake_lock(&mut self) {
        self.lock
            .idle
            .wake(Instant::now(), self.appearance.reduce_motion);
        self.redraw = true;
    }

    pub fn lock_surface(&self) -> Option<&WlSurface> {
        self.lock
            .surface
            .as_ref()
            .filter(|surface| surface.alive())
            .map(LockSurface::wl_surface)
    }

    pub(super) fn update_lock_focus(&mut self) {
        let focus = self.lock_surface().filter(|_| self.host_focused).cloned();
        let keyboard = self.keyboard.clone();
        if keyboard.current_focus() != focus {
            keyboard.set_focus(self, focus, SERIAL_COUNTER.next_serial());
        }
    }

    pub(super) fn configure_locker(&self) {
        if let Some(surface) = self.lock.surface.as_ref().filter(|surface| surface.alive())
            && let Some(mode) = self.output.current_mode()
        {
            surface.with_pending_state(|state| {
                state.size = Some((mode.size.w as u32, mode.size.h as u32).into())
            });
            surface.send_configure();
        }
    }

    pub(super) fn lock_frame_presented(&mut self, stamp: Duration) {
        self.lock.phase = LockPhase::Covered;
        if self.lock.covered_at.is_none() {
            let now = Instant::now();
            self.lock.covered_at = Some(now);
            self.lock.idle = Idle {
                dim_at: now + DIM_AFTER_LOCK,
                waking: None,
                dim_started: false,
            };
            self.redraw = true;
        }
        if let Some(pending) = self.lock.pending.take() {
            pending.lock();
        }
        if self.lock_surface().is_some() && std::mem::take(&mut self.lock.suspend_pending) {
            match Command::new("systemctl")
                .args(["--no-ask-password", "suspend"])
                .spawn()
            {
                Ok(job) => self.lock.suspend_job = Some(job),
                Err(error) => {
                    eprintln!("Could not suspend: {error}");
                    self.show_suspend_error();
                }
            }
        }
        if let Some(surface) = self.lock_surface() {
            send_frames_surface_tree(
                surface,
                &self.output,
                stamp,
                Some(Duration::ZERO),
                |_, _| Some(self.output.clone()),
            );
        }
    }

    fn cover_desktop(&mut self) {
        self.lock.phase = LockPhase::Covering;
        self.lock.covered_at = None;
        self.lock.unlocked_at = None;
        if self.overview.open {
            self.toggle_overview();
        }
        self.status_panel = Panel::Closed;
        self.dock_pressed = None;
        self.status_pressed = None;
        self.dragged_window = None;
        self.finish_resize();
        let keyboard = self.keyboard.clone();
        keyboard.unset_grab(self);
        self.focus_window(None);
        let pointer = self.pointer.clone();
        let serial = SERIAL_COUNTER.next_serial();
        pointer.motion(
            self,
            None,
            &MotionEvent {
                location: pointer.current_location(),
                serial,
                time: 0,
            },
        );
        pointer.unset_grab(self, serial, 0);
        pointer.frame(self);
        self.redraw = true;
    }
}

impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.lock.manager
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        if self.native_session
            && self.lock_active()
            && self.lock.child.is_some()
            && self.lock_surface().is_none()
            && self.lock.pending.is_none()
        {
            self.lock.phase = LockPhase::Covering;
            self.lock.pending = Some(confirmation);
            self.redraw = true;
        }
    }

    fn unlock(&mut self) {
        if self.lock.phase == LockPhase::Covered && self.lock.pending.is_none() {
            self.lock.phase = LockPhase::Desktop;
            self.lock.unlocked_at = Some(Instant::now());
            self.lock.surface = None;
            self.lock.suspend_pending = false;
            if self.status_error.is_some() {
                self.status_panel = Panel::Settings;
            }
            self.focus_last_window();
            self.redraw = true;
        }
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        if self.output.owns(&output) && self.lock_active() {
            if let Some(mode) = self.output.current_mode() {
                // Smithay sends the initial configure after this hook returns.
                surface.with_pending_state(|state| {
                    state.size = Some((mode.size.w as u32, mode.size.h as u32).into())
                });
            }
            self.lock.surface = Some(surface);
            self.redraw = true;
        }
    }
}

delegate_session_lock!(State);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waking_reverses_the_current_dim_and_resets_the_deadline() {
        let dim_at = Instant::now();
        let mut idle = Idle {
            dim_at,
            waking: None,
            dim_started: true,
        };
        let interrupted_at = dim_at + DIM_DURATION / 2;
        assert_eq!(idle.amount(interrupted_at, false), 0.5);
        idle.wake(interrupted_at, false);
        assert_eq!(idle.amount(interrupted_at, false), 0.5);
        assert_eq!(idle.amount(interrupted_at + WAKE_DURATION, false), 0.0);
        assert_eq!(idle.amount(interrupted_at + DIM_AFTER_INPUT, true), 1.0);
        assert!(!idle.dim_started);
    }
}
