use super::{
    model::{Arrangement, Background, Catalogue, HudRef, ImageMode, InventoryWindow, Layout, Picture, Startup},
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
    Update(Uuid),
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
    RememberFolders(bool),
    CaptureGraphics(bool),
    CaptureCamera(bool),
    CaptureBackground(bool),
    BackgroundCommon(Option<[u8; 3]>),
    BackgroundDisplay(Arrangement, usize, Option<[u8; 3]>),
    BackgroundImage(Option<(Arrangement, usize)>),
    BackgroundLocal(Option<(Arrangement, usize)>),
    BackgroundPicture(Option<(Arrangement, usize)>, Picture),
    BackgroundMode(Option<(Arrangement, usize)>, ImageMode),
    BackgroundBrightness(Option<(Arrangement, usize)>, u8),
    CaptureHuds(bool),
    CaptureAdditionalInventory(bool),
    SelectHud(HudRef, bool),
    ClearHuds,
    ReviewHuds(Uuid),
    DecideHuds(Uuid, bool),
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
    FolderUnavailable,
    ExpandedUnavailable,
    InventoryWindowUnavailable,
    CameraUnavailable,
    HudUnavailable,
    PictureUnavailable,
    LocalImageUnavailable,
}
pub(crate) struct Preview {
    pub baseline: Layout,
    pub active: Option<Uuid>,
    pub candidate: Uuid,
    background: Option<Background>,
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
pub(crate) struct InventoryPending {
    pub owner: Uuid,
    pub source: Source,
    pub binding: u64,
    pub rules: (u64, bool),
    pub metadata: crate::world::inventory_archive::Revision,
    pub wanted: InventoryWindow,
    pub passes: u8,
    pub fresh: bool,
    pub deadline: std::time::Instant,
}
impl InventoryPending {
    pub(crate) fn same_owner(&self, current: Option<Uuid>) -> bool {
        current == Some(self.owner)
    }
    pub(crate) fn current(
        &self,
        source: Option<&Source>,
        binding: u64,
        rules: (u64, bool),
        metadata: Option<crate::world::inventory_archive::Revision>,
        now: std::time::Instant,
    ) -> bool {
        source == Some(&self.source)
            && binding == self.binding
            && rules == self.rules
            && metadata == Some(self.metadata)
            && now < self.deadline
            && self.passes < 3
    }
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
    pub last_sample: Option<std::time::Instant>,
    pub inventory_pending: Option<InventoryPending>,
    pub modified_sample: Option<(u64, std::time::Instant, bool)>,
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
            last_sample: None,
            inventory_pending: None,
            modified_sample: None,
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
        self.last_sample = None;
        self.inventory_pending = None;
        self.modified_sample = None;
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
    /// Sample expensive optional metadata at most four times per second; the
    /// explicit logout capture remains immediate and freshly qualified.
    pub(crate) fn sample_last(&mut self, now: std::time::Instant, force: bool) -> bool {
        if !force
            && self
                .last_sample
                .is_some_and(|at| now.saturating_duration_since(at) < std::time::Duration::from_millis(250))
        {
            return false;
        }
        self.last_sample = Some(now);
        true
    }
    /// Persistent workspace presentation needs the private catalogue after a
    /// source gap too, even when startup was already consumed for this login.
    pub(crate) fn needs_catalogue(&self) -> bool {
        self.permit
            && !self.exhausted
            && self.source.is_some()
            && self.pending.is_none()
            && self.catalogue.is_none()
            && self.status != Status::Error
    }
    /// Current account-only choice, a valid pending write, or a reversible
    /// staged target. No source/foreign/stale pending data can color this view.
    pub(crate) fn background(&self) -> Background {
        let Some(source) = &self.source else {
            return Background::Black {};
        };
        let Some(catalogue) = self
            .catalogue
            .as_ref()
            .filter(|c| c.account == source.agent && c.grid == source.grid)
        else {
            return Background::Black {};
        };
        let committed = self
            .pending
            .as_ref()
            .filter(|p| p.binding == self.binding && &p.source == source && p.writing)
            .and_then(|p| p.expected.as_ref())
            .filter(|c| c.account == source.agent && c.grid == source.grid)
            .map_or_else(|| catalogue.background.clone(), |c| c.background.clone());
        if let Some(preview) = &self.preview {
            return preview.background.clone().unwrap_or(committed);
        }
        committed
    }
    pub(crate) fn begin_preview(&mut self, layout: Layout, id: Uuid, current: Layout, commit: bool) -> Layout {
        let mut baseline = self
            .preview
            .as_ref()
            .map_or_else(|| current.baseline_for(&layout), |p| p.baseline.clone());
        if baseline.inventory_folder.is_none() && layout.inventory_folder.is_some() {
            baseline.inventory_folder = current.inventory_folder;
        }
        if layout.additional_inventory.is_some() {
            if let Some(old) = &mut baseline.additional_inventory {
                if let Some(current) = current.additional_inventory {
                    if old.geometry.is_none() {
                        old.geometry = current.geometry;
                    }
                    if old.visible && !old.minimized {
                        if old.folder.is_none() {
                            old.folder = current.folder;
                        }
                        if old.expanded.is_none() {
                            old.expanded = current.expanded;
                        }
                    }
                }
            } else {
                baseline.additional_inventory = current.additional_inventory;
            }
        }
        if baseline.inventory_expanded.is_none() && layout.inventory_expanded.is_some() {
            baseline.inventory_expanded = current.inventory_expanded;
        }
        if baseline.huds.is_none() && layout.huds.is_some() {
            baseline.huds = current.huds;
        }
        if baseline.background.is_none() && layout.background.is_some() {
            baseline.background = current.background.clone();
        }
        if baseline.camera.is_none() && layout.camera.is_some() {
            baseline.camera = current.camera;
        }
        if baseline.graphics.is_none() && layout.graphics.is_some() {
            baseline.graphics = current.graphics;
        }
        let previous = commit.then(|| {
            (
                baseline.baseline_for(&layout),
                self.preview.as_ref().map_or(self.active, |p| p.active),
            )
        });
        let active = self.preview.as_ref().map_or(self.active, |p| p.active);
        self.preview = Some(Preview {
            baseline: baseline.clone(),
            active,
            candidate: id,
            background: layout.background.clone(),
        });
        self.placement = Some(Placement {
            layout,
            passes: 0,
            commit,
            active: Some(id),
            room: None,
            previous,
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
        let current = current.baseline_for(&layout);
        self.preview = Some(Preview {
            baseline: current.clone(),
            active: self.active,
            candidate: active.unwrap_or(Uuid::nil()),
            background: layout.background.clone(),
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
            inventory_folder: None,
            inventory_expanded: None,
            additional_inventory: None,
            graphics: None,
            camera: None,
            background: None,
            huds: None,
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
    #[test]
    fn workspace_previous_only_restores_folder_when_the_switch_included_it() {
        let mut c = Controller::default();
        let mut live = layout(0.0);
        live.inventory_folder = Some(Uuid::new_v4());
        let target = layout(12.0);
        c.begin_preview(target.clone(), Uuid::new_v4(), live.clone(), true);
        let placement = c.placement.take().unwrap();
        c.finish_placement(placement);
        assert!(c.previous.as_ref().unwrap().0.inventory_folder.is_none());
        let mut target = target;
        target.inventory_folder = Some(Uuid::new_v4());
        c.begin_preview(target, Uuid::new_v4(), live.clone(), true);
        let placement = c.placement.take().unwrap();
        c.finish_placement(placement);
        assert_eq!(c.previous.as_ref().unwrap().0.inventory_folder, live.inventory_folder);
    }
    #[test]
    fn workspace_graphics_preview_union_keeps_rollback_but_previous_uses_final_switch_scope() {
        let mut c = Controller::default();
        let mut live = layout(0.0);
        live.graphics = Some(super::super::model::Graphics::capture(&crate::settings::Settings::default()));
        let mut target = layout(10.0);
        target.graphics = live.graphics.clone();
        let baseline = c.begin_preview(target, Uuid::new_v4(), live.clone(), false);
        assert_eq!(baseline.graphics, live.graphics);
        c.begin_preview(layout(20.0), Uuid::new_v4(), live.clone(), true);
        assert_eq!(c.preview.as_ref().unwrap().baseline.graphics, live.graphics);
        let p = c.placement.take().unwrap();
        c.finish_placement(p);
        assert!(c.previous.as_ref().unwrap().0.graphics.is_none());
    }
    #[test]
    fn workspace_camera_preview_union_rolls_back_and_previous_tracks_only_final_target() {
        let mut c = Controller::default();
        let mut live = layout(0.0);
        live.camera = Some(super::super::model::CameraContext::capture(
            &crate::camera::CameraSettings::default(),
            0.4,
        ));
        let mut target = layout(10.0);
        target.camera = live.camera.clone();
        c.begin_preview(target, Uuid::new_v4(), live.clone(), false);
        c.begin_preview(layout(20.0), Uuid::new_v4(), live.clone(), true);
        assert_eq!(c.preview.as_ref().unwrap().baseline.camera, live.camera);
        let placement = c.placement.take().unwrap();
        c.finish_placement(placement);
        assert!(c.previous.as_ref().unwrap().0.camera.is_none());
    }
    #[test]
    fn workspace_background_preview_keeps_committed_choice_and_source_loss_hides_it() {
        let source = source();
        let mut catalogue = Catalogue::empty(source.agent, source.grid.clone());
        let committed = Background::Solid { rgb: [10, 20, 30] };
        let candidate = Background::Solid { rgb: [40, 50, 60] };
        catalogue.background = committed.clone();
        let mut c = Controller {
            source: Some(source),
            catalogue: Some(catalogue),
            ..Default::default()
        };
        let mut live = layout(0.0);
        live.background = Some(committed.clone());
        let mut target = layout(10.0);
        target.background = Some(candidate.clone());
        c.begin_preview(target, Uuid::new_v4(), live, false);
        assert_eq!(c.background(), candidate);
        assert_eq!(c.catalogue.as_ref().unwrap().background, committed);
        c.preview = None;
        c.placement = None;
        assert_eq!(c.background(), committed);
        c.bind(None);
        assert_eq!(c.background(), Background::Black {});
    }
    #[test]
    fn workspace_background_pending_choice_requires_exact_binding_and_account() {
        let source = source();
        let catalogue = Catalogue::empty(source.agent, source.grid.clone());
        let mut next = catalogue.clone();
        next.background = Background::Solid { rgb: [10, 20, 30] };
        let mut c = Controller {
            source: Some(source.clone()),
            catalogue: Some(catalogue),
            ..Default::default()
        };
        c.pending = Some(Pending {
            source,
            binding: c.binding,
            operation: 1,
            active: None,
            writing: true,
            expected: Some(next),
        });
        assert_eq!(c.background(), Background::Solid { rgb: [10, 20, 30] });
        c.pending.as_mut().unwrap().binding += 1;
        assert_eq!(c.background(), Background::Black {});
        c.pending.as_mut().unwrap().binding = c.binding;
        c.pending.as_mut().unwrap().expected.as_mut().unwrap().account = Uuid::new_v4();
        assert_eq!(c.background(), Background::Black {});
    }
    #[test]
    fn workspace_source_gap_reloads_presentation_without_replaying_consumed_startup() {
        let source = source();
        let mut c = Controller::default();
        c.bind(Some(source.clone()));
        c.cancel_startup();
        c.bind(None);
        assert!(!c.needs_catalogue());
        c.bind(Some(source.clone()));
        c.permit = true;
        assert!(!c.startup_waiting);
        assert!(c.needs_catalogue());
        c.status = Status::Error;
        assert!(!c.needs_catalogue(), "no repeated failed IO each frame");
        c.status = Status::Ready;
        c.catalogue = Some(Catalogue::empty(source.agent, source.grid));
        assert!(!c.needs_catalogue());
    }
    #[test]
    fn workspace_background_previous_keeps_scoped_target_instead_of_old_record_lookup() {
        let source = source();
        let mut catalogue = Catalogue::empty(source.agent, source.grid.clone());
        let old = Uuid::new_v4();
        catalogue.background = Background::Solid { rgb: [10, 20, 30] };
        let mut record_layout = layout(0.0);
        record_layout.background = Some(Background::Solid { rgb: [40, 50, 60] });
        catalogue.records.push(super::super::model::Record {
            id: old,
            name: "Old".into(),
            layout: record_layout,
        });
        let expected = catalogue.background.clone();
        let mut c = Controller {
            source: Some(source),
            catalogue: Some(catalogue),
            previous: Some((layout(0.0), Some(old))),
            ..Default::default()
        };
        c.begin_previous(layout(10.0));
        assert_eq!(
            c.background(),
            expected,
            "geometry-only Previous must not borrow that record's old context"
        );
    }
    #[test]
    fn workspace_display_background_preview_and_previous_keep_full_original_rules_and_source_fence() {
        let s = source();
        let topology = Arrangement::from_physical(&[[0, 0, 100, 100], [100, 0, 200, 100]]).unwrap();
        let original = Background::Solid { rgb: [1, 2, 3] }
            .with_display(&topology, 1, Some([4, 5, 6]))
            .unwrap();
        let candidate = original.with_display(&topology, 0, Some([7, 8, 9])).unwrap();
        let mut catalogue = Catalogue::empty(s.agent, s.grid.clone());
        catalogue.background = original.clone();
        let mut c = Controller {
            source: Some(s),
            catalogue: Some(catalogue),
            ..Default::default()
        };
        let mut live = layout(0.0);
        live.background = Some(original.clone());
        let mut target = layout(5.0);
        target.background = Some(candidate.clone());
        let baseline = c.begin_preview(target, Uuid::new_v4(), live.clone(), true);
        assert_eq!(baseline.background, Some(original.clone()));
        assert_eq!(c.background(), candidate);
        assert_eq!(c.catalogue.as_ref().unwrap().background, original);
        let placement = c.placement.take().unwrap();
        c.finish_placement(placement);
        assert_eq!(c.previous.as_ref().unwrap().0.background, Some(original.clone()));
        c.begin_previous(live);
        assert_eq!(c.background(), original);
        c.bind(None);
        assert_eq!(c.background(), Background::Black {});
    }
    #[test]
    fn workspace_hud_preview_baseline_and_previous_preserve_only_the_scoped_additive_set() {
        use super::super::model::{HudRef, HudSet};
        let mut c = Controller::default();
        let mut current = layout(1.0);
        let original = HudSet::try_from(vec![HudRef {
            item: Uuid::new_v4(),
            point: 31,
        }])
        .unwrap();
        current.huds = Some(original.clone());
        let target = layout(2.0);
        let id = Uuid::new_v4();
        c.begin_preview(target, id, current.clone(), false);
        assert!(c.preview.as_ref().unwrap().baseline.huds.is_none());
        let mut target = layout(3.0);
        target.huds = Some(HudSet::default());
        c.begin_preview(target, id, current, true);
        assert_eq!(c.preview.as_ref().unwrap().baseline.huds, Some(original.clone()));
        let placement = c.placement.take().unwrap();
        c.finish_placement(placement);
        assert_eq!(c.previous.as_ref().unwrap().0.huds, Some(original.clone()));
        c.begin_previous(layout(4.0));
        assert_eq!(c.placement.as_ref().unwrap().layout.huds, Some(original));
    }
    #[test]
    fn workspace_last_capture_sampling_is_bounded_forceable_and_source_reset() {
        use std::time::{Duration, Instant};
        let mut c = Controller::default();
        let now = Instant::now();
        assert!(c.sample_last(now, false));
        assert!(!c.sample_last(now + Duration::from_millis(249), false));
        assert!(c.sample_last(now + Duration::from_millis(250), false));
        assert!(c.sample_last(now + Duration::from_millis(251), true));
        c.modified_sample = Some((c.revision, now, true));
        c.bind(None);
        assert!(c.last_sample.is_none());
        assert!(c.modified_sample.is_none());
        assert!(c.sample_last(now + Duration::from_millis(252), false));
    }
    #[test]
    fn workspace_additional_preview_scope_never_invents_hidden_folder_navigation() {
        let mut c = Controller::default();
        let mut original = layout(1.0);
        original.additional_inventory = Some(InventoryWindow {
            visible: false,
            minimized: false,
            geometry: None,
            folder: None,
            expanded: None,
        });
        let mut target = layout(2.0);
        target.additional_inventory = Some(InventoryWindow {
            visible: true,
            minimized: false,
            geometry: None,
            folder: None,
            expanded: None,
        });
        let id = Uuid::new_v4();
        c.begin_preview(target.clone(), id, original, false);
        let mut current = target.clone();
        current.additional_inventory.as_mut().unwrap().folder = Some(Uuid::new_v4());
        target.additional_inventory = current.additional_inventory.clone();
        let baseline = c.begin_preview(target, id, current, true);
        assert!(baseline.validate().is_ok());
        let window = baseline.additional_inventory.unwrap();
        assert!(!window.visible && window.folder.is_none());
        let placement = c.placement.take().unwrap();
        c.finish_placement(placement);
        assert!(c.previous.as_ref().unwrap().0.validate().is_ok());
    }
    #[test]
    fn workspace_additional_pending_requires_exact_native_scope_and_never_extends_its_deadline() {
        use std::time::{Duration, Instant};
        let world = crate::world::inventory_archive::tests::native_fixture_world();
        let archive = crate::world::inventory_archive::controller::testing::native_loaded(&world, &[]);
        let metadata = archive.metadata_revision(&world).unwrap();
        let source = Source {
            agent: world.agent_id,
            login: world.login.as_ref().unwrap().session_id,
            grid: archive.native_grid(&world).unwrap().namespace(),
            hosted: None,
        };
        let now = Instant::now();
        let mut pending = InventoryPending {
            owner: Uuid::new_v4(),
            source: source.clone(),
            binding: 1,
            rules: (5, true),
            metadata,
            wanted: InventoryWindow {
                visible: true,
                minimized: false,
                geometry: None,
                folder: None,
                expanded: None,
            },
            passes: 0,
            fresh: true,
            deadline: now + Duration::from_secs(2),
        };
        assert!(pending.same_owner(Some(pending.owner)));
        assert!(!pending.same_owner(None));
        assert!(
            !pending.same_owner(Some(Uuid::new_v4())),
            "a replacement full window cannot consume an old restore"
        );
        assert!(pending.current(Some(&source), 1, (5, true), Some(metadata), now));
        assert!(!pending.current(None, 1, (5, true), Some(metadata), now));
        let mut other = source.clone();
        other.login = Uuid::new_v4();
        assert!(!pending.current(Some(&other), 1, (5, true), Some(metadata), now));
        assert!(!pending.current(Some(&source), 2, (5, true), Some(metadata), now));
        assert!(!pending.current(Some(&source), 1, (6, true), Some(metadata), now));
        assert!(!pending.current(Some(&source), 1, (5, false), Some(metadata), now));
        assert!(!pending.current(Some(&source), 1, (5, true), None, now));
        assert!(!pending.current(Some(&source), 1, (5, true), Some(metadata), now + Duration::from_secs(2)));
        pending.passes = 3;
        assert!(!pending.current(Some(&source), 1, (5, true), Some(metadata), now));
    }
}
