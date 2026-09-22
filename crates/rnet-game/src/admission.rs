use rnet_core::Handle;
use std::collections::HashSet;

mod production_sync {
    pub(crate) use std::sync::atomic::{AtomicBool, Ordering};
    pub(crate) use std::sync::RwLock;
}

#[cfg(all(test, feature = "loom"))]
mod model_sync {
    pub(crate) use loom::sync::atomic::{AtomicBool, Ordering};
    pub(crate) use loom::sync::RwLock;
}

// Production and Loom instantiate the same state-transition implementation with different
// synchronization backends. The public game runtime always uses std synchronization, including
// all-features builds; Loom types exist only inside the bounded model tests below.
macro_rules! define_send_admission {
    ($name:ident, $sync:ident) => {
        /// Serializes ready-session publication/removal with queue admission.
        ///
        /// The caller performs queue cleanup after `remove`/`clear` returns. Any admission that
        /// entered its closure completes before the write lock is acquired; later admissions see
        /// the missing ready marker or stopping flag. This prevents close/stop from leaving a
        /// message in a queue after cleanup.
        pub(crate) struct $name {
            ready: $sync::RwLock<HashSet<Handle>>,
            stopping: $sync::AtomicBool,
        }

        impl $name {
            pub(crate) fn new() -> Self {
                Self {
                    ready: $sync::RwLock::new(HashSet::new()),
                    stopping: $sync::AtomicBool::new(false),
                }
            }

            pub(crate) fn is_stopping(&self) -> bool {
                self.stopping.load($sync::Ordering::Acquire)
            }

            pub(crate) fn begin_stop(&self) {
                self.stopping.store(true, $sync::Ordering::Release);
            }

            pub(crate) fn publish(&self, session: Handle) {
                self.ready
                    .write()
                    .expect("game ready table poisoned")
                    .insert(session);
            }

            pub(crate) fn contains(&self, session: Handle) -> bool {
                self.ready
                    .read()
                    .expect("game ready table poisoned")
                    .contains(&session)
            }

            /// Runs `operation` while session readiness cannot be removed by cleanup.
            pub(crate) fn with_ready<T>(
                &self,
                session: Handle,
                operation: impl FnOnce() -> T,
            ) -> Option<T> {
                let ready = self.ready.read().expect("game ready table poisoned");
                if !ready.contains(&session) || self.is_stopping() {
                    return None;
                }
                Some(operation())
            }

            pub(crate) fn remove(&self, session: Handle) {
                self.ready
                    .write()
                    .expect("game ready table poisoned")
                    .remove(&session);
            }

            pub(crate) fn clear(&self) {
                self.ready
                    .write()
                    .expect("game ready table poisoned")
                    .clear();
            }
        }
    };
}

define_send_admission!(SendAdmission, production_sync);

#[cfg(all(test, feature = "loom"))]
define_send_admission!(LoomSendAdmission, model_sync);

#[cfg(all(test, feature = "loom"))]
mod tests {
    use super::LoomSendAdmission;
    use loom::sync::atomic::{AtomicUsize, Ordering};
    use loom::sync::Arc;
    use loom::thread;

    const SESSION: u64 = 7;

    #[test]
    fn closing_a_session_cannot_leave_a_racing_send_admitted() {
        loom::model(|| {
            let admission = Arc::new(LoomSendAdmission::new());
            let queued = Arc::new(AtomicUsize::new(0));
            admission.publish(SESSION);
            assert!(admission.contains(SESSION));

            let sender = {
                let admission = admission.clone();
                let queued = queued.clone();
                thread::spawn(move || {
                    admission.with_ready(SESSION, || queued.fetch_add(1, Ordering::AcqRel));
                })
            };
            let closer = {
                let admission = admission.clone();
                let queued = queued.clone();
                thread::spawn(move || {
                    admission.remove(SESSION);
                    queued.store(0, Ordering::Release);
                })
            };

            sender.join().unwrap();
            closer.join().unwrap();
            assert_eq!(queued.load(Ordering::Acquire), 0);
        });
    }

    #[test]
    fn stopping_cannot_leave_a_racing_send_admitted_after_cleanup() {
        loom::model(|| {
            let admission = Arc::new(LoomSendAdmission::new());
            let queued = Arc::new(AtomicUsize::new(0));
            admission.publish(SESSION);
            assert!(admission.contains(SESSION));

            let sender = {
                let admission = admission.clone();
                let queued = queued.clone();
                thread::spawn(move || {
                    admission.with_ready(SESSION, || queued.fetch_add(1, Ordering::AcqRel));
                })
            };
            let stopper = {
                let admission = admission.clone();
                let queued = queued.clone();
                thread::spawn(move || {
                    admission.begin_stop();
                    admission.clear();
                    queued.store(0, Ordering::Release);
                })
            };

            sender.join().unwrap();
            stopper.join().unwrap();
            assert_eq!(queued.load(Ordering::Acquire), 0);
        });
    }
}
