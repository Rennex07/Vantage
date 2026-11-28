pub(super) struct Fullscreen<Geometry, Serial> {
    pub requested: bool,
    pub saved: Option<Geometry>,
    pub exit: Option<Serial>,
}

impl<Geometry, Serial> Default for Fullscreen<Geometry, Serial> {
    fn default() -> Self {
        Self {
            requested: false,
            saved: None,
            exit: None,
        }
    }
}

impl<Geometry, Serial: Copy + PartialOrd> Fullscreen<Geometry, Serial> {
    pub fn enter(&mut self, current: Geometry) {
        self.requested = true;
        self.exit = None;
        self.saved.get_or_insert(current);
    }

    pub fn restore(&mut self, committed: Option<Serial>) -> Option<Geometry> {
        if self.requested
            || !self
                .exit
                .zip(committed)
                .is_some_and(|(exit, serial)| serial >= exit)
        {
            return None;
        }
        self.exit = None;
        self.saved.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_waits_for_the_exit_configure_commit() {
        let mut fullscreen = Fullscreen::default();
        let window = ((52, 73), (800, 450));
        fullscreen.enter(window);
        fullscreen.requested = false;
        fullscreen.exit = Some(8);
        assert_eq!(fullscreen.restore(None), None);
        assert_eq!(fullscreen.restore(Some(7)), None);
        assert_eq!(fullscreen.restore(Some(9)), Some(window));
        assert_eq!(fullscreen.restore(Some(10)), None);
    }

    #[test]
    fn reentry_before_exit_commit_preserves_the_original_geometry() {
        let mut fullscreen = Fullscreen::default();
        fullscreen.enter(((52, 73), (800, 450)));
        fullscreen.requested = false;
        fullscreen.exit = Some(8);
        fullscreen.enter(((0, 0), (1920, 1080)));
        assert_eq!(fullscreen.restore(Some(8)), None);
        fullscreen.requested = false;
        fullscreen.exit = Some(12);
        assert_eq!(fullscreen.restore(Some(12)), Some(((52, 73), (800, 450))));
    }
}
