use std::sync::Mutex;

/// Owns one restartable task. Construct the handle only after winning the slot;
/// detach it before awaiting shutdown so no mutex guard crosses an await.
pub(crate) struct TaskSlot<H> {
    name: &'static str,
    // Outer None permanently closes registration; inner None permits restart.
    handle: Mutex<Option<Option<H>>>,
}

impl<H> TaskSlot<H> {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            name,
            handle: Mutex::new(Some(None)),
        }
    }

    pub(crate) fn start(&self, start: impl FnOnce() -> H) -> bool {
        let Ok(mut handle) = self.handle.lock() else {
            log::error!("{} lock is poisoned", self.name);
            return false;
        };
        let Some(handle) = handle.as_mut() else {
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
            Ok(mut handle) => handle.as_mut().and_then(Option::take),
            Err(_) => {
                log::error!("{} lock is poisoned", self.name);
                None
            }
        }
    }

    pub(super) fn close(&self) -> Option<H> {
        match self.handle.lock() {
            Ok(mut handle) => handle.take().flatten(),
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

    #[test]
    fn close_rejects_start_even_after_take_or_repeated_close() {
        for occupied in [false, true] {
            let slot = TaskSlot::new("test");
            if occupied {
                assert!(slot.start(|| 31));
            }
            assert_eq!(slot.close(), occupied.then_some(31));
            assert_eq!(slot.take(), None);
            assert_eq!(slot.close(), None);
            assert!(!slot.start(|| panic!("closed slot must not construct a worker")));
        }
    }
}
