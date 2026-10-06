use std::sync::Mutex;

/// Owns one restartable task. Construct the handle only after winning the slot;
/// detach it before awaiting shutdown so no mutex guard crosses an await.
pub(crate) struct TaskSlot<H> {
    name: &'static str,
    handle: Mutex<Option<H>>,
}

impl<H> TaskSlot<H> {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            name,
            handle: Mutex::new(None),
        }
    }

    pub(crate) fn start(&self, start: impl FnOnce() -> H) -> bool {
        let Ok(mut handle) = self.handle.lock() else {
            log::error!("{} lock is poisoned", self.name);
            return false;
        };
        if handle.is_some() {
            return false;
        }
        *handle = Some(start());
        true
    }

    pub(crate) fn take(&self) -> Option<H> {
        match self.handle.lock() {
            Ok(mut handle) => handle.take(),
            Err(_) => {
                log::error!("{} lock is poisoned", self.name);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn concurrent_start_constructs_exactly_one_handle_and_take_allows_restart() {
        let slot = TaskSlot::new("test");
        let starts = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    slot.start(|| starts.fetch_add(1, Ordering::SeqCst));
                });
            }
        });
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(slot.take(), Some(0));
        assert_eq!(slot.take(), None);
        assert!(slot.start(|| 27));
        assert_eq!(slot.take(), Some(27));
    }
}
