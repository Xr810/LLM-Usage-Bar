use std::sync::Mutex;

/// Tracks long-lived workers. Close registration before stopping handles so a
/// late startup continuation cannot create orphan workers during shutdown.
pub(super) struct RuntimeTasks<H> {
    handles: Mutex<Option<Vec<H>>>,
}

impl<H> Default for RuntimeTasks<H> {
    fn default() -> Self {
        Self {
            handles: Mutex::new(Some(Vec::new())),
        }
    }
}

impl<H> RuntimeTasks<H> {
    pub(super) fn spawn(&self, spawn: impl FnOnce() -> H) {
        let mut handles = self.handles.lock().unwrap();
        if let Some(handles) = handles.as_mut() {
            handles.push(spawn());
        }
    }

    pub(super) fn close(&self) -> Vec<H> {
        self.handles.lock().unwrap().take().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closing_takes_every_handle_and_rejects_late_registration() {
        let tasks = RuntimeTasks::default();
        tasks.spawn(|| 7);
        tasks.spawn(|| 19);
        assert_eq!(tasks.close(), vec![7, 19]);
        tasks.spawn(|| panic!("must not spawn after shutdown"));
        assert!(tasks.close().is_empty());
    }
}
