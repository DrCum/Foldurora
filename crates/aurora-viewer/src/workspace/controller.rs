use super::{
    model::{Catalogue, Layout, Startup},
    store::Lane,
};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Source {
    pub agent: Uuid,
    pub login: Uuid,
    pub grid: String,
    pub hosted: Option<(aurora_session::chat::ChatSource, u64)>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ticket {
    pub instance: Uuid,
    pub binding: u64,
    pub revision: u64,
    pub source: Source,
    pub record: Option<Uuid>,
}
#[derive(Clone, Debug)]
pub(crate) enum Command {
    Save(String),
    Rename(Uuid, String),
    Delete(Uuid),
    Preview(Uuid),
    Switch(Uuid),
    Cancel,
    Previous,
    Retry,
    Favorite(Uuid),
    Startup(Startup),
    ShowFavorites(bool),
}
#[derive(Clone, Debug)]
pub(crate) struct Action {
    pub ticket: Ticket,
    pub command: Command,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Status {
    #[default]
    Unavailable,
    Loading,
    Ready,
    Saving,
    Error,
    Invalid,
    Previewing,
}
pub(crate) struct Preview {
    pub baseline: Layout,
    pub active: Option<Uuid>,
    pub candidate: Uuid,
}
pub(crate) struct Placement {
    pub layout: Layout,
    pub passes: u8,
    pub commit: bool,
    pub active: Option<Uuid>,
    pub room: Option<egui::Rect>,
    pub previous: Option<(Layout, Option<Uuid>)>,
    pub restore_absence: bool,
}
pub(crate) struct Pending {
    pub source: Source,
    pub binding: u64,
    pub operation: u64,
    pub active: Option<Uuid>,
    pub writing: bool,
    pub expected: Option<Catalogue>,
}

pub(crate) struct Controller {
    pub instance: Uuid,
    pub binding: u64,
    pub revision: u64,
    pub source: Option<Source>,
    pub permit: bool,
    pub catalogue: Option<Catalogue>,
    pub persisted: bool,
    pub active: Option<Uuid>,
    pub previous: Option<(Layout, Option<Uuid>)>,
    pub preview: Option<Preview>,
    pub placement: Option<Placement>,
    pub lane: Option<Lane>,
    pub pending: Option<Pending>,
    pub operation: u64,
    pub status: Status,
    pub exhausted: bool,
    pub startup_waiting: bool,
    pub startup_source: Option<Source>,
    pub last_observed: Option<(Layout, std::time::Instant)>,
}
impl Default for Controller {
    fn default() -> Self {
        Self {
            instance: Uuid::new_v4(),
            binding: 1,
            revision: 1,
            source: None,
            permit: false,
            catalogue: None,
            persisted: false,
            active: None,
            previous: None,
            preview: None,
            placement: None,
            lane: None,
            pending: None,
            operation: 0,
            status: Status::Unavailable,
            exhausted: false,
            startup_waiting: false,
            startup_source: None,
            last_observed: None,
        }
    }
}
impl Controller {
    /// A logical login boundary retires even an identical server-provided
    /// identity. The one outstanding IO slot is retained until its own reply.
    pub(crate) fn bind(&mut self, source: Option<Source>) {
        if let Some(next) = self.binding.checked_add(1) {
            self.binding = next;
        } else {
            self.exhausted = true;
        }
        // Activation changes are layout authority boundaries, not new logins.
        // Preserve the normalized startup identity across temporary source gaps.
        self.startup_waiting = false;
        if let Some(mut login) = source.clone() {
            login.hosted = login.hosted.map(|(source, _)| (source, 0));
            if self.startup_source.as_ref() != Some(&login) {
                self.startup_waiting = true;
            }
        }
        self.source = source;
        self.last_observed = None;
        self.catalogue = None;
        self.persisted = false;
        self.active = None;
        self.previous = None;
        self.preview = None;
        self.placement = None;
        self.permit = false;
        self.status = Status::Unavailable;
        self.advance();
    }
    pub(crate) fn begin_preview(&mut self, layout: Layout, id: Uuid, current: Layout, commit: bool) -> Layout {
        let baseline = self.preview.as_ref().map_or(current, |p| p.baseline.clone());
        let active = self.preview.as_ref().map_or(self.active, |p| p.active);
        self.preview = Some(Preview {
            baseline: baseline.clone(),
            active,
            candidate: id,
        });
        self.placement = Some(Placement {
            layout,
            passes: 0,
            commit,
            active: Some(id),
            room: None,
            previous: commit.then(|| (baseline.clone(), active)),
            restore_absence: false,
        });
        self.status = Status::Previewing;
        baseline
    }
    pub(crate) fn begin_previous(&mut self, current: Layout) -> Option<Layout> {
        if self.preview.is_some() {
            return None;
        }
        let (layout, active) = self.previous.clone()?;
        self.preview = Some(Preview {
            baseline: current.clone(),
            active: self.active,
            candidate: active.unwrap_or(Uuid::nil()),
        });
        self.placement = Some(Placement {
            layout,
            passes: 0,
            commit: true,
            active,
            room: None,
            previous: Some((current.clone(), self.active)),
            restore_absence: true,
        });
        self.status = Status::Previewing;
        Some(current)
    }
    pub(crate) fn finish_placement(&mut self, p: Placement) {
        self.active = p.active;
        if let Some(previous) = p.previous {
            self.previous = Some(previous);
        }
        self.preview = None;
        self.status = Status::Ready;
        self.advance();
    }
    /// Full captured identity and exact write result precede any current state
    /// mutation. An old binding can retire only its own pending IO slot.
    pub(crate) fn accept_reply(&mut self, reply: super::store::Reply) {
        let Some(p) = &self.pending else { return };
        if p.binding != reply.binding || p.operation != reply.operation || p.source != reply.source {
            return;
        }
        if let Ok(result) = &reply.result
            && (result
                .as_ref()
                .is_some_and(|c| c.validate().is_err() || c.account != p.source.agent || c.grid != p.source.grid)
                || (p.writing && result.as_ref() != p.expected.as_ref()))
        {
            return;
        }
        let pending = self.pending.take().expect("matched workspace IO");
        if self.exhausted || reply.binding != self.binding || self.source.as_ref() != Some(&reply.source) {
            return;
        }
        match reply.result {
            Ok(c) => {
                self.persisted = c.is_some();
                self.catalogue = c.or_else(|| self.source.as_ref().map(|s| Catalogue::empty(s.agent, s.grid.clone())));
                if pending.writing {
                    self.active = pending.active;
                }
                self.status = Status::Ready;
            }
            Err(()) => {
                self.status = Status::Error;
                self.catalogue = None;
            }
        }
        self.advance();
    }
    pub(crate) fn ticket(&self) -> Option<Ticket> {
        Some(Ticket {
            instance: self.instance,
            binding: self.binding,
            revision: self.revision,
            source: self.source.clone()?,
            record: None,
        })
    }
    pub(crate) fn accepts(&self, t: &Ticket) -> bool {
        self.permit
            && !self.exhausted
            && self.ticket().is_some_and(|mut expected| {
                expected.record = t.record;
                expected == *t
            })
    }
    pub(crate) fn advance(&mut self) -> bool {
        match self.revision.checked_add(1) {
            Some(n) => {
                self.revision = n;
                true
            }
            None => {
                self.exhausted = true;
                false
            }
        }
    }
    pub(crate) fn ready(&self) -> bool {
        self.permit && !self.exhausted && self.pending.is_none() && self.catalogue.is_some() && self.placement.is_none()
    }
    pub(crate) fn record(&self, id: Uuid) -> Option<&super::model::Record> {
        self.catalogue.as_ref()?.records.iter().find(|r| r.id == id)
    }
    pub(crate) fn take_startup(&mut self) -> Option<(Option<Uuid>, Layout)> {
        if !self.startup_waiting || !self.ready() || self.preview.is_some() {
            return None;
        }
        let source = self.source.as_ref()?;
        let catalogue = self.catalogue.as_ref()?;
        if catalogue.account != source.agent || catalogue.grid != source.grid || catalogue.validate().is_err() {
            return None;
        }
        let layout = catalogue.startup_layout();
        self.cancel_startup();
        layout
    }
    pub(crate) fn cancel_startup(&mut self) {
        self.startup_waiting = false;
        if let Some(mut source) = self.source.clone() {
            source.hosted = source.hosted.map(|(source, _)| (source, 0));
            self.startup_source = Some(source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> Source {
        Source {
            agent: Uuid::new_v4(),
            login: Uuid::new_v4(),
            grid: "c".repeat(64),
            hosted: None,
        }
    }
    #[test]
    fn workspace_tickets_are_full_capture_nonreused_and_consumed_once() {
        let mut c = Controller {
            source: Some(source()),
            permit: true,
            ..Default::default()
        };
        let t = c.ticket().unwrap();
        assert!(c.accepts(&t));
        c.advance();
        assert!(!c.accepts(&t));
        let t = c.ticket().unwrap();
        c.permit = false;
        assert!(!c.accepts(&t));
        c.permit = true;
        c.source.as_mut().unwrap().login = Uuid::new_v4();
        assert!(!c.accepts(&t));
        let other = Controller {
            source: c.source.clone(),
            permit: true,
            ..Default::default()
        };
        assert!(!other.accepts(&c.ticket().unwrap()));
        c.revision = u64::MAX;
        assert!(!c.advance());
        assert!(!c.accepts(&c.ticket().unwrap()));
    }
    #[test]
    fn workspace_late_and_mismatched_io_cannot_clear_new_pending_catalogue_or_status() {
        let s = source();
        let mut c = Controller {
            source: Some(s.clone()),
            permit: true,
            ..Default::default()
        };
        c.pending = Some(Pending {
            source: s.clone(),
            binding: 1,
            operation: 7,
            active: None,
            writing: false,
            expected: None,
        });
        let mut wrong = s.clone();
        wrong.login = Uuid::new_v4();
        c.accept_reply(super::super::store::Reply {
            source: wrong.clone(),
            binding: 1,
            operation: 7,
            result: Err(()),
        });
        assert!(c.pending.is_some());
        let current = Catalogue::empty(wrong.agent, wrong.grid.clone());
        c.source = Some(wrong);
        c.binding = 2;
        c.catalogue = Some(current.clone());
        c.status = Status::Saving;
        c.accept_reply(super::super::store::Reply {
            source: s,
            binding: 1,
            operation: 7,
            result: Err(()),
        });
        assert!(c.pending.is_none());
        assert_eq!(c.catalogue, Some(current));
        assert_eq!(c.status, Status::Saving);
    }
    #[test]
    fn workspace_logical_reauthentication_fences_identical_source_tickets_and_old_io_until_finished() {
        let source = source();
        let mut c = Controller::default();
        c.bind(Some(source.clone()));
        c.permit = true;
        let old = c.ticket().unwrap();
        let old_binding = c.binding;
        c.pending = Some(Pending {
            source: source.clone(),
            binding: old_binding,
            operation: 4,
            active: None,
            writing: false,
            expected: None,
        });
        c.bind(None);
        c.bind(Some(source.clone()));
        c.permit = true;
        assert!(!c.accepts(&old));
        assert!(c.pending.is_some(), "unfinished old IO keeps its sole credit");
        c.accept_reply(super::super::store::Reply {
            source: source.clone(),
            binding: old_binding,
            operation: 4,
            result: Ok(None),
        });
        assert!(c.pending.is_none());
        assert!(c.catalogue.is_none());
        assert_eq!(c.status, Status::Unavailable);
        c.binding = u64::MAX;
        c.pending = Some(Pending {
            source: source.clone(),
            binding: u64::MAX,
            operation: 5,
            active: None,
            writing: false,
            expected: None,
        });
        c.bind(Some(source.clone()));
        assert!(c.exhausted);
        c.accept_reply(super::super::store::Reply {
            source,
            binding: u64::MAX,
            operation: 5,
            result: Ok(None),
        });
        assert!(c.pending.is_none());
        assert!(c.catalogue.is_none());
        assert_eq!(c.status, Status::Unavailable);
    }
    #[test]
    fn workspace_write_reply_must_match_entire_original_proposed_catalogue() {
        let s = source();
        let expected = Catalogue::empty(s.agent, s.grid.clone());
        let mut c = Controller {
            source: Some(s.clone()),
            permit: true,
            ..Default::default()
        };
        c.pending = Some(Pending {
            source: s.clone(),
            binding: 1,
            operation: 2,
            active: None,
            writing: true,
            expected: Some(expected.clone()),
        });
        let mut different = expected.clone();
        different.revision = 2;
        c.accept_reply(super::super::store::Reply {
            source: s.clone(),
            binding: 1,
            operation: 2,
            result: Ok(Some(different)),
        });
        assert!(c.pending.is_some());
        assert!(c.catalogue.is_none());
        c.accept_reply(super::super::store::Reply {
            source: s,
            binding: 1,
            operation: 2,
            result: Ok(Some(expected.clone())),
        });
        assert!(c.pending.is_none());
        assert_eq!(c.catalogue, Some(expected));
    }
    fn layout(left: f32) -> Layout {
        let s = crate::settings::Settings::default();
        Layout {
            geometry: [Some(super::super::model::Geometry {
                point: 0,
                offset: [left, 0.0],
                size: [300.0, 200.0],
            }); 4],
            viewport: super::super::model::Viewport {
                left,
                ..(&s.world_viewport).into()
            },
            bars: super::super::model::Bars {
                classic: false,
                stretch: true,
                rights_text: false,
            },
        }
    }
    #[test]
    fn workspace_preview_replacement_cancel_baseline_and_previous_swap_keep_unsaved_arrangements() {
        let mut c = Controller::default();
        let a = layout(1.0);
        let b = layout(2.0);
        let d = layout(3.0);
        let id = Uuid::new_v4();
        assert_eq!(c.begin_preview(b.clone(), id, a.clone(), false), a);
        c.placement = None;
        assert_eq!(c.begin_preview(d.clone(), id, b.clone(), false), a);
        assert_eq!(c.preview.as_ref().unwrap().baseline, a);
        assert!(c.previous.is_none());
        let cancel = c.preview.take().unwrap();
        c.placement = None;
        assert_eq!(cancel.baseline, a);
        c.begin_preview(b.clone(), id, a.clone(), true);
        assert!(c.previous.is_none(), "Previous changes only after successful placement");
        let p = c.placement.take().unwrap();
        c.finish_placement(p);
        assert_eq!(c.previous, Some((a.clone(), None)));
        assert_eq!(c.active, Some(id));
        assert_eq!(c.begin_previous(d.clone()), Some(d.clone()));
        let p = c.placement.take().unwrap();
        assert_eq!(p.layout, a);
        c.finish_placement(p);
        assert_eq!(c.previous, Some((d.clone(), Some(id))));
        c.begin_previous(a);
        assert_eq!(c.placement.as_ref().unwrap().layout, d);
    }
    #[test]
    fn workspace_preview_and_previous_keep_known_geometry_for_unmeasured_closed_roles() {
        let mut c = Controller::default();
        let mut old = layout(0.0);
        old.geometry[3] = None;
        let target = layout(5.0);
        c.begin_preview(target.clone(), Uuid::new_v4(), old.clone(), false);
        assert_eq!(c.placement.as_ref().unwrap().layout.geometry[3], target.geometry[3]);
        assert!(c.preview.as_ref().unwrap().baseline.geometry[3].is_none());
        c.preview = None;
        c.placement = None;
        c.previous = Some((target.clone(), None));
        c.begin_previous(old);
        assert_eq!(c.placement.as_ref().unwrap().layout.geometry[3], target.geometry[3]);
    }

    #[test]
    fn workspace_startup_runs_once_per_login_and_not_after_temporary_source_loss() {
        let source = source();
        let mut c = Controller::default();
        c.bind(Some(source.clone()));
        c.bind(None);
        c.bind(Some(source.clone()));
        assert!(c.startup_waiting, "an unconsumed startup survives a temporary source gap");
        let mut catalogue = Catalogue::empty(source.agent, source.grid.clone());
        catalogue.startup = Startup::LastArrangement;
        catalogue.last_layout = Some(layout(12.0));
        c.catalogue = Some(catalogue.clone());
        assert!(c.take_startup().is_none(), "no effect without native permit");
        c.permit = true;
        assert_eq!(c.take_startup(), Some((None, layout(12.0))));
        assert!(c.take_startup().is_none());
        c.bind(None);
        c.bind(Some(source.clone()));
        c.catalogue = Some(catalogue.clone());
        c.permit = true;
        assert!(!c.startup_waiting);
        assert!(c.take_startup().is_none());
        c.startup_source = None; // App's explicit reauthentication boundary.
        c.bind(Some(source));
        c.catalogue = Some(catalogue);
        c.permit = true;
        assert!(c.take_startup().is_some());
    }

    #[test]
    fn workspace_startup_rejects_another_accounts_catalogue_and_changed_login() {
        let source = source();
        let mut c = Controller::default();
        c.bind(Some(source.clone()));
        c.permit = true;
        let mut catalogue = Catalogue::empty(Uuid::new_v4(), source.grid.clone());
        catalogue.startup = Startup::LastArrangement;
        catalogue.last_layout = Some(layout(2.0));
        c.catalogue = Some(catalogue);
        assert!(c.take_startup().is_none());
        assert!(c.startup_waiting);
        let ticket = c.ticket().unwrap();
        let mut next = source;
        next.login = Uuid::new_v4();
        c.bind(Some(next));
        assert!(c.startup_waiting);
        assert!(!c.accepts(&ticket));
    }
}
