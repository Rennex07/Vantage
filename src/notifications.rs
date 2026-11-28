use std::{
    collections::{HashSet, VecDeque},
    time::{Duration, Instant},
};

const HISTORY_LIMIT: usize = 24;
pub const VISIBLE_ROWS: usize = 4;

#[derive(Clone, Debug)]
pub struct Action {
    pub key: String,
    pub label: String,
}

pub struct Request {
    pub replaces: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<Action>,
    pub timeout_ms: i32,
}

pub struct Entry {
    pub id: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<Action>,
    owner: String,
    open: bool,
    deadline: Option<Instant>,
}

pub enum Event {
    Closed { owner: String, id: u32, reason: u32 },
    Invoked { owner: String, id: u32, key: String },
}

#[derive(Default)]
pub struct Center {
    entries: VecDeque<Entry>,
    next_id: u32,
    revision: u64,
    private: bool,
    quiet: bool,
    muted_apps: HashSet<String>,
    banner: Option<(u32, Instant)>,
    events: Vec<Event>,
}

impl Center {
    pub fn notify(&mut self, owner: &str, request: Request, now: Instant) -> u32 {
        let replacement = self
            .entries
            .iter()
            .position(|entry| entry.id == request.replaces && entry.open && entry.owner == owner);
        let id = replacement.map_or_else(
            || loop {
                self.next_id = self.next_id.wrapping_add(1).max(1);
                if self.entries.iter().all(|entry| entry.id != self.next_id) {
                    break self.next_id;
                }
            },
            |index| self.entries[index].id,
        );
        let timeout = match request.timeout_ms {
            0 => None,
            ms if ms > 0 => Some(Duration::from_millis(ms as u64)),
            _ => Some(Duration::from_secs(5)),
        };
        let entry = Entry {
            id,
            owner: owner.into(),
            app: plain_text(&request.app, 40),
            summary: plain_text(&request.summary, 96),
            body: plain_text(&request.body, 200),
            actions: request
                .actions
                .into_iter()
                .filter(|action| !action.key.is_empty() && action.key.len() <= 128)
                .take(8)
                .map(|action| Action {
                    label: plain_text(&action.label, 30),
                    ..action
                })
                .collect(),
            open: true,
            deadline: timeout.map(|duration| now + duration),
        };
        if let Some(index) = replacement {
            self.entries[index] = entry;
        } else {
            if self.entries.len() == HISTORY_LIMIT
                && let Some(old) = self.entries.pop_back().filter(|entry| entry.open)
            {
                self.events.push(Event::Closed {
                    owner: old.owner,
                    id: old.id,
                    reason: 4,
                });
            }
            self.entries.push_front(entry);
        }
        if !self.private
            && !self.quiet
            && !self
                .entries
                .iter()
                .any(|entry| entry.id == id && self.app_muted(&entry.app))
            && (replacement.is_none() || self.banner.is_some_and(|(shown, _)| shown == id))
        {
            let visible_for = timeout
                .unwrap_or(Duration::from_secs(5))
                .min(Duration::from_secs(8));
            self.banner = Some((id, now + visible_for));
        } else if self.banner.is_some_and(|(shown, _)| shown == id) {
            self.banner = None;
        }
        self.changed();
        id
    }

    pub fn close(&mut self, owner: &str, id: u32) -> bool {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.id == id && entry.open && entry.owner == owner)
        else {
            return false;
        };
        self.remove(index, 3);
        true
    }

    pub fn dismiss(&mut self, id: u32) {
        if !self.private
            && let Some(index) = self.entries.iter().position(|entry| entry.id == id)
        {
            self.remove(index, 2);
        }
    }

    pub fn clear(&mut self) {
        if !self.private {
            while !self.entries.is_empty() {
                self.remove(0, 2);
            }
        }
    }

    pub fn invoke(&mut self, id: u32, action: usize) {
        if self.private {
            return;
        }
        if let Some((index, entry)) = self
            .entries
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.id == id && entry.open)
            && let Some(action) = entry.actions.get(action)
        {
            self.events.push(Event::Invoked {
                owner: entry.owner.clone(),
                id,
                key: action.key.clone(),
            });
            self.remove(index, 2);
        }
    }

    fn remove(&mut self, index: usize, reason: u32) {
        let entry = self
            .entries
            .remove(index)
            .expect("notification index was checked");
        if entry.open {
            self.events.push(Event::Closed {
                owner: entry.owner,
                id: entry.id,
                reason,
            });
        }
        if self.banner.is_some_and(|(id, _)| id == entry.id) {
            self.banner = None;
        }
        self.changed();
    }

    pub fn tick(&mut self, now: Instant) {
        let mut changed = false;
        for entry in &mut self.entries {
            if entry.open && entry.deadline.is_some_and(|deadline| now >= deadline) {
                entry.open = false;
                entry.actions.clear();
                self.events.push(Event::Closed {
                    owner: entry.owner.clone(),
                    id: entry.id,
                    reason: 1,
                });
                changed = true;
            }
        }
        if self.banner.is_some_and(|(_, until)| now >= until) {
            self.banner = None;
            changed = true;
        }
        if changed {
            self.changed();
        }
    }

    pub fn set_private(&mut self, private: bool) -> bool {
        if private != self.private {
            self.private = private;
            self.banner = None;
            self.changed();
            return true;
        }
        false
    }

    pub fn toggle_quiet(&mut self) {
        if !self.private {
            self.quiet = !self.quiet;
            self.banner = None;
            self.changed();
        }
    }

    pub fn quiet(&self) -> bool {
        self.quiet
    }

    // Client-supplied display labels are not authenticated identities.
    // Muting is a session-only presentation preference.
    pub fn app_muted(&self, app: &str) -> bool {
        self.muted_apps.contains(app)
    }
    pub fn any_muted(&self) -> bool {
        !self.muted_apps.is_empty()
    }
    pub fn toggle_app(&mut self, id: u32) {
        if self.private {
            return;
        }
        let Some(app) = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.app.clone())
        else {
            return;
        };
        if app.is_empty() {
            return;
        }
        if !self.muted_apps.remove(&app) {
            if self.muted_apps.len() >= HISTORY_LIMIT {
                return;
            }
            self.muted_apps.insert(app.clone());
            if self.banner().is_some_and(|entry| entry.app == app) {
                self.banner = None;
            }
        }
        self.changed();
    }
    pub fn unmute_all(&mut self) {
        if !self.private && self.any_muted() {
            self.muted_apps.clear();
            self.changed();
        }
    }

    pub fn pages(&self) -> usize {
        self.entries.len().div_ceil(VISIBLE_ROWS).max(1)
    }

    pub fn visible(&self, page: usize) -> impl Iterator<Item = &Entry> {
        let mut grouped = Vec::new();
        if !self.private {
            let mut seen = HashSet::new();
            for entry in &self.entries {
                if entry.app.is_empty() {
                    grouped.push(entry);
                } else if seen.insert(entry.app.as_str()) {
                    grouped.extend(self.entries.iter().filter(|other| other.app == entry.app));
                }
            }
        }
        grouped
            .into_iter()
            .skip(page * VISIBLE_ROWS)
            .take(VISIBLE_ROWS)
    }

    pub fn banner(&self) -> Option<&Entry> {
        let (id, _) = self.banner?;
        (!self.private)
            .then(|| self.entries.iter().find(|entry| entry.id == id))
            .flatten()
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

fn plain_text(text: &str, max: usize) -> String {
    let text: String = text
        .chars()
        .take(max)
        .map(|ch| {
            if ch.is_control() || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                ' '
            } else {
                ch
            }
        })
        .collect();
    text.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(replaces: u32, timeout_ms: i32) -> Request {
        Request {
            replaces,
            app: "Test".into(),
            summary: "First\nline".into(),
            body: "Private text".into(),
            actions: vec![Action {
                key: "open".into(),
                label: "Open".into(),
            }],
            timeout_ms,
        }
    }

    #[test]
    fn replacement_and_close_are_sender_scoped() {
        let now = Instant::now();
        let mut center = Center::default();
        let first = center.notify(":1.1", request(0, 0), now);
        assert_eq!(center.notify(":1.1", request(first, 0), now), first);
        assert_ne!(center.notify(":1.2", request(first, 0), now), first);
        assert!(!center.close(":1.2", first));
        assert!(center.close(":1.1", first));
        assert!(
            matches!(&center.take_events()[0], Event::Closed { owner, reason: 3, .. } if owner == ":1.1")
        );
    }

    #[test]
    fn privacy_hides_content_and_does_not_replay_locked_banners() {
        let now = Instant::now();
        let mut center = Center::default();
        center.notify(":1.1", request(0, 0), now);
        assert!(center.banner().is_some());
        center.set_private(true);
        let id = center.notify(":1.1", request(0, 0), now);
        center.invoke(id, 0);
        assert!(center.banner().is_none());
        assert_eq!(center.visible(0).count(), 0);
        assert!(center.take_events().is_empty());
        center.set_private(false);
        assert_eq!(center.visible(0).count(), 2);
        assert!(center.banner().is_none());
    }

    #[test]
    fn grouping_and_session_mute_preserve_history_and_privacy() {
        let now = Instant::now();
        let mut center = Center::default();
        let first = center.notify(":1.1", request(0, 0), now);
        let mut other = request(0, 0);
        other.app = "Other".into();
        let second = center.notify(":1.2", other, now);
        let third = center.notify(":1.1", request(0, 0), now);
        assert_eq!(
            center.visible(0).map(|e| e.id).collect::<Vec<_>>(),
            vec![third, first, second]
        );
        center.toggle_app(third);
        assert!(center.app_muted("Test"));
        assert!(center.banner().is_none());
        let fourth = center.notify(":1.1", request(0, 0), now);
        assert!(center.banner().is_none());
        assert_eq!(center.visible(0).next().unwrap().id, fourth);
        center.set_private(true);
        center.unmute_all();
        center.toggle_app(third);
        assert!(center.app_muted("Test"));
        center.set_private(false);
        center.unmute_all();
        assert!(!center.any_muted());
        center.notify(":1.1", request(0, 0), now);
        assert!(center.banner().is_some());
    }

    #[test]
    fn expiration_keeps_history_and_actions_send_directed_events() {
        let now = Instant::now();
        let mut center = Center::default();
        let expired = center.notify(":1.1", request(0, 10), now);
        center.tick(now + Duration::from_millis(10));
        assert!(center.banner().is_none());
        assert_eq!(center.visible(0).next().unwrap().summary, "First line");
        center.invoke(expired, 0);
        let active = center.notify(":1.2", request(0, 0), now);
        center.invoke(active, 0);
        assert!(matches!(center.take_events().as_slice(), [
            Event::Closed { reason: 1, .. },
            Event::Invoked { id, key, .. },
            Event::Closed { reason: 2, .. },
        ] if *id == active && key == "open"));
    }
}
