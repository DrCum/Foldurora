//! The viewer's own capture intent, recorded from the actual attributes used
//! to create each world window. This is not an OS/DRM confidentiality claim.
//!
//! GetWindowDisplayAffinity is limited to layered/DWM windows. A failed query
//! cannot establish intent; a verified unprotected creation record can. Any
//! positive protected native observation remains denied for that window epoch.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_WINDOW: AtomicU64 = AtomicU64::new(1);

pub(super) struct Creation {
    epoch: u64,
    protected: bool,
}
pub(super) struct Policy<K> {
    window: K,
    epoch: u64,
    protected: bool,
    observed_protected: Cell<bool>,
}

impl Creation {
    pub fn from_attributes(attributes: &winit::window::WindowAttributes) -> Self {
        let epoch = NEXT_WINDOW
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .unwrap_or(0);
        Self {
            epoch,
            protected: attributes.content_protected,
        }
    }
    pub fn bind<K>(self, window: K) -> Policy<K> {
        Policy {
            window,
            epoch: self.epoch,
            protected: self.protected,
            observed_protected: Cell::new(false),
        }
    }
}

impl<K: PartialEq> Policy<K> {
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn allows(&self, window: K, current_epoch: u64, native_affinity: Option<u32>) -> bool {
        if self.epoch == 0 || current_epoch != self.epoch || self.window != window {
            return false;
        }
        if native_affinity.is_some_and(|value| value != 0) {
            self.observed_protected.set(true);
        }
        !self.protected && !self.observed_protected.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_capture_policy_uses_actual_attributes_and_does_not_turn_native_failure_into_unknown_provenance_permission() {
        let attributes = winit::window::Window::default_attributes();
        let open = Creation::from_attributes(&attributes).bind(17u64);
        assert!(
            open.allows(17, open.epoch(), None),
            "known own unprotected intent survives an unavailable layered-only query"
        );
        assert!(open.allows(17, open.epoch(), Some(0)));
        let protected = Creation::from_attributes(&attributes.with_content_protected(true)).bind(18u64);
        assert!(!protected.allows(18, protected.epoch(), None));
        assert!(
            !protected.allows(18, protected.epoch(), Some(0)),
            "a native query cannot weaken own protected intent"
        );
        let unknown = Creation {
            epoch: 0,
            protected: false,
        }
        .bind(19u64);
        assert!(!unknown.allows(19, 0, None));
        assert!(!unknown.allows(19, 0, Some(0)));
    }

    #[test]
    fn recreation_and_reused_native_ids_cannot_reuse_previous_capture_authority() {
        let attrs = winit::window::Window::default_attributes();
        let old = Creation::from_attributes(&attrs).bind(17u64);
        let new = Creation::from_attributes(&attrs).bind(17u64);
        assert_ne!(old.epoch(), new.epoch());
        assert!(!old.allows(17, new.epoch(), None));
        assert!(!new.allows(18, new.epoch(), None));
        assert!(new.allows(17, new.epoch(), None));
        assert!(!new.allows(17, new.epoch(), Some(0x11)));
        assert!(!new.allows(17, new.epoch(), None));
        assert!(
            !new.allows(17, new.epoch(), Some(0)),
            "observed protection remains closed until a new verified window epoch"
        );
    }

    #[test]
    fn world_capture_intent_cannot_gain_an_untracked_runtime_protection_setter() {
        // There are no runtime protection setters today. Adding one requires
        // a tracked policy transition and recreation review here, rather than
        // silently weakening this creation-provenance guarantee.
        fn visit(path: &std::path::Path) {
            for entry in std::fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(&path);
                    continue;
                }
                if path.extension().is_none_or(|s| s != "rs") || path.file_name().is_some_and(|s| s == "window_capture.rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).unwrap();
                let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
                for setter in [".set_content_protected(", "SetWindowDisplayAffinity("] {
                    assert!(
                        !compact.contains(setter),
                        "{} adds an untracked capture-policy setter",
                        path.display()
                    );
                }
            }
        }
        visit(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
    }
}
