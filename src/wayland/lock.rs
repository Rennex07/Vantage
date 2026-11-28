use std::{
    io,
    process::{Child, Command},
    time::{Duration, Instant},
};

use smithay::{
    input::{keyboard::keysyms, pointer::MotionEvent},
    utils::SERIAL_COUNTER,
};

use super::State;
use crate::{
    lock_auth::{Password, SERVICE_PATH},
    preferences::Aod,
    shell::{
        lockscreen::{self, SignIn, SignInStatus},
        status::Panel,
        timeline::{Ease, Transition, progress},
    },
};

const DIM_AFTER_LOCK: Duration = Duration::from_secs(5);
const DIM_AFTER_INPUT: Duration = Duration::from_secs(18);
const DIM_DURATION: Duration = Duration::from_millis(850);
const WAKE_DURATION: Duration = Duration::from_millis(360);
const SIGN_DURATION: Duration = Duration::from_millis(310);
const UNLOCK_DURATION: Duration = Duration::from_millis(400);

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

pub(super) struct Lock {
    active: bool,
    covered_at: Option<Instant>,
    unlocked_at: Option<Instant>,
    idle: Idle,
    signing_in: bool,
    sign_transition: Option<Transition>,
    password: Password,
    user: String,
    status: SignInStatus,
    auth_job: Option<Child>,
    poweroff: Option<Child>,
}

impl Lock {
    pub fn new() -> Self {
        Self {
            active: false,
            covered_at: None,
            unlocked_at: None,
            idle: Idle {
                dim_at: Instant::now(),
                waking: None,
                dim_started: false,
            },
            signing_in: false,
            sign_transition: None,
            password: Password::default(),
            user: vte4::gtk::glib::user_name().to_string_lossy().into_owned(),
            status: SignInStatus::Ready,
            auth_job: None,
            poweroff: None,
        }
    }

    fn sign_amount(&self, now: Instant) -> f64 {
        self.sign_transition
            .as_ref()
            .map_or(f64::from(self.signing_in), |sign| sign.value(now))
    }

    fn show_sign_in(&mut self, visible: bool, now: Instant, reduced: bool) {
        if self.signing_in != visible {
            self.sign_transition = (!reduced).then(|| {
                Transition::new(
                    self.sign_amount(now),
                    f64::from(visible),
                    now,
                    SIGN_DURATION,
                    Ease::Out,
                )
            });
            self.signing_in = visible;
        }
        if !visible {
            self.password.clear();
            self.status = SignInStatus::Ready;
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
        if self.lock_active() {
            return Ok(());
        }
        if !std::path::Path::new(SERVICE_PATH).is_file() {
            return Err(io::Error::new(io::ErrorKind::NotFound, SERVICE_PATH));
        }
        self.lock.active = true;
        self.lock.covered_at = None;
        self.lock.unlocked_at = None;
        self.lock.show_sign_in(false, Instant::now(), true);
        self.lock.sign_transition = None;
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
        pointer.unset_grab(self, serial, 0);
        pointer.motion(
            self,
            None,
            &MotionEvent {
                location: pointer.current_location(),
                serial,
                time: 0,
            },
        );
        pointer.frame(self);
        self.redraw = true;
        Ok(())
    }

    pub(super) fn check_locker(&mut self) {
        if let Some(job) = &mut self.lock.auth_job {
            let result = match job.try_wait() {
                Ok(None) => None,
                Ok(Some(status)) => Some(status.code()),
                Err(error) => {
                    eprintln!("Could not wait for lock authentication: {error}");
                    let _ = job.kill();
                    let _ = job.wait();
                    Some(None)
                }
            };
            if let Some(result) = result {
                self.lock.auth_job = None;
                match result {
                    Some(0) => self.unlock_session(),
                    Some(1) => self.lock.status = SignInStatus::Incorrect,
                    _ => self.lock.status = SignInStatus::Unavailable,
                }
                self.redraw = true;
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
        if self.lock_active() && self.lock.covered_at.is_some() {
            let now = Instant::now();
            if now >= self.lock.idle.dim_at && !self.lock.idle.dim_started {
                self.lock.idle.dim_started = true;
                self.lock
                    .show_sign_in(false, now, self.appearance.reduce_motion);
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

    pub fn lock_active(&self) -> bool {
        self.lock.active
    }

    pub fn lock_amounts(&self, now: Instant) -> (f32, f32, f32) {
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
        let sign = if reduced {
            f64::from(self.lock.signing_in)
        } else {
            self.lock.sign_amount(now)
        };
        (reveal as f32, dim as f32, sign as f32)
    }

    pub fn lock_animating(&self, now: Instant) -> bool {
        let (reveal, dim, sign) = self.lock_amounts(now);
        self.lock.covered_at.is_some()
            && ((reveal < 1.0 && !self.appearance.reduce_motion)
                || (dim > 0.0 && dim < 1.0)
                || (sign > 0.0 && sign < 1.0))
    }

    pub fn unlock_amount(&self, now: Instant) -> f32 {
        if self.appearance.reduce_motion {
            return 0.0;
        }
        self.lock.unlocked_at.map_or(0.0, |start| {
            (1.0 - Ease::Out.sample(progress(now, start, UNLOCK_DURATION))) as f32
        })
    }

    pub fn lock_sign_in(&self) -> SignIn<'_> {
        let keyboard = self.keyboard.clone();
        SignIn {
            user: &self.lock.user,
            dots: self.lock.password.len().min(28),
            status: if self.lock.auth_job.is_some() {
                SignInStatus::Checking
            } else {
                self.lock.status
            },
            caps_lock: keyboard.modifier_state().caps_lock,
        }
    }

    fn show_lock_sign_in(&mut self, now: Instant) {
        self.lock.idle.wake(now, self.appearance.reduce_motion);
        self.lock
            .show_sign_in(true, now, self.appearance.reduce_motion);
        self.redraw = true;
    }

    pub(super) fn lock_key(&mut self, symbol: u32, character: Option<char>) {
        if self.lock.covered_at.is_none() {
            return;
        }
        let now = Instant::now();
        if symbol == keysyms::KEY_Escape && self.lock.auth_job.is_none() {
            self.lock.idle.wake(now, self.appearance.reduce_motion);
            self.lock
                .show_sign_in(false, now, self.appearance.reduce_motion);
            self.redraw = true;
            return;
        }

        let was_signing_in = self.lock.signing_in;
        self.show_lock_sign_in(now);
        if self.lock.auth_job.is_some() {
            return;
        }
        match symbol {
            keysyms::KEY_Return | keysyms::KEY_KP_Enter if was_signing_in => self.submit_password(),
            keysyms::KEY_BackSpace => self.lock.password.pop(),
            _ => {
                if let Some(character) = character.filter(|character| !character.is_control()) {
                    self.lock.status = SignInStatus::Ready;
                    self.lock.password.push(character);
                }
            }
        }
    }

    pub(super) fn lock_click(&mut self) {
        if self.lock.covered_at.is_none() {
            return;
        }
        if self.lock.signing_in
            && let Some(mode) = self.output.current_mode()
            && lockscreen::submit_contains(
                mode.size.w,
                mode.size.h,
                self.pointer.current_location(),
            )
        {
            self.submit_password();
        }
        self.show_lock_sign_in(Instant::now());
    }

    pub(super) fn lock_pointer_moved(&mut self) {
        self.redraw |= self.lock.signing_in;
    }

    fn submit_password(&mut self) {
        if self.lock.auth_job.is_none() && self.lock.password.len() > 0 {
            match self.lock.password.submit() {
                Ok(job) => self.lock.auth_job = Some(job),
                Err(error) => {
                    eprintln!("Could not start lock authentication: {error}");
                    self.lock.status = SignInStatus::Unavailable;
                }
            }
        }
    }

    pub(super) fn lock_frame_presented(&mut self) {
        if self.lock.active && self.lock.covered_at.is_none() {
            let now = Instant::now();
            self.lock.covered_at = Some(now);
            self.lock.idle = Idle {
                dim_at: now + DIM_AFTER_LOCK,
                waking: None,
                dim_started: false,
            };
            self.redraw = true;
        }
    }

    fn unlock_session(&mut self) {
        self.lock.active = false;
        self.lock.unlocked_at = Some(Instant::now());
        self.lock.show_sign_in(false, Instant::now(), true);
        self.focus_last_window();
        self.redraw = true;
    }
}

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
