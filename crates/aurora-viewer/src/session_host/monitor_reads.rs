//! Bounded original-expiry view proof cache. Never retains private images,
//! backend command authority or operation receipts and never renews a receipt.
use super::monitor_manager::index;
use super::monitor_model::View;
use super::preview_projection::{PreviewRead, ViewNamespace};

pub(super) struct ViewReads {
    views: [Option<PreviewRead>; 5],
}
impl Default for ViewReads {
    fn default() -> Self {
        Self {
            views: std::array::from_fn(|_| None),
        }
    }
}
impl ViewReads {
    pub fn remember(&mut self, read: &PreviewRead) {
        self.views[index(read.view())] = read.view_only();
    }
    pub fn retain_namespace(&mut self, view: View, namespace: Option<ViewNamespace>) {
        if self.views[index(view)]
            .as_ref()
            .is_some_and(|read| Some(read.namespace()) != namespace || !read.view_current())
        {
            self.views[index(view)] = None;
        }
    }
    pub fn fallback(&mut self, view: View, namespace: ViewNamespace) -> Option<PreviewRead> {
        self.retain_namespace(view, Some(namespace));
        let read = self.views[index(view)].as_ref()?.view_only();
        if read.is_none() {
            self.views[index(view)] = None;
        }
        read
    }
    pub fn clear(&mut self) {
        self.views = std::array::from_fn(|_| None);
    }
}
