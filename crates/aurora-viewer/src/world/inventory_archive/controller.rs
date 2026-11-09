//! One actual-source model/IO owner shared by World/App and all ordinary
//! Inventory/chooser consumers. Offline scopes have no persistence authority.
use super::{
    EditTicket, Error, LoadTicket, Model, SaveIntent, Status, World, offline,
    store::{self, Request, Worker},
    visibility::{CpuTurn, OwnerPolicy, Projection, VisibilityRead},
};
use crate::world::account_metadata_origin::GridDigest;
use std::{path::Path, sync::Arc};
use uuid::Uuid;
#[cfg(test)]
std::thread_local! { static ASSIGNMENTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[derive(Clone, PartialEq, Eq)]
enum Pending {
    Load(LoadTicket),
    Save(SaveIntent),
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    pub(crate) fn reset_assignments() {
        ASSIGNMENTS.with(|count| count.set(0));
    }
    pub(crate) fn assignments() -> usize {
        ASSIGNMENTS.with(std::cell::Cell::get)
    }
    pub(in crate::world::inventory_archive) fn projection(controller: &mut Controller, projection: Projection) {
        controller.projection = projection;
        controller.observed = None;
    }
    pub(in crate::world::inventory_archive) fn initialized(controller: &Controller) -> usize {
        super::super::visibility::testing::initialized(&controller.projection)
    }
    pub(in crate::world::inventory_archive) fn previous_profile(controller: &mut Controller) {
        // Perturb only the original captured selection, never actual global
        // selection or NativeDirectory paths. Fresh APIs must reject it.
        controller.profile = Some(Path::new("/synthetic-archive-previous-profile"));
    }
    pub(in crate::world::inventory_archive) fn resources(controller: &Controller) -> (bool, bool, Option<usize>) {
        (
            controller.worker.is_some(),
            controller.pending.is_some(),
            super::super::visibility::testing::payload(&controller.projection),
        )
    }
    pub(in crate::world::inventory_archive) fn reply(controller: &mut Controller, reply: store::Reply) {
        controller.accept(reply);
    }
    /// Complete the actual native Model's exact load ticket using codec data.
    /// No Worker exists, so even a positive UI proposal cannot publish IO.
    pub(crate) fn native_loaded(world: &World, flags: &[Uuid]) -> Controller {
        let grid = GridDigest::from_uri("https://archive-ui-fixture.example.invalid/login").unwrap();
        let mut controller = Controller::bind(world, Some(grid));
        let model = controller.model.as_ref().expect("genuine native metadata fixture source");
        let ticket = model.load_ticket().unwrap();
        let source = model.source();
        let document = super::super::Document {
            account: source.account(),
            grid: grid.bytes(),
            publication: 1,
            folders: Arc::new(flags.iter().copied().collect()),
        };
        let bytes = document.encode().unwrap();
        let document = super::super::Document::decode(&bytes, source).unwrap();
        controller.pending = Some(Pending::Load(ticket));
        controller.accept(store::Reply::Loaded {
            ticket,
            result: Ok(store::Loaded {
                document: Some(document),
                original: Some(Arc::from(bytes)),
            }),
        });
        controller.projection = super::super::visibility::testing::projection(32, 16, 16, super::super::MAX_DEPTH, false);
        controller
    }
    /// Test-only exact data receipt through the real pending/accept path.
    /// It is not native IO evidence and cannot be used by MemoryOnly.
    pub(crate) fn publish_fixture(controller: &mut Controller, world: &World, folder: Uuid, desired: bool) {
        let model = controller.model.as_mut().expect("persistent metadata fixture");
        let ticket = model.prepare_edit(world, folder).unwrap();
        let intent = model.begin_save(world, ticket, desired).unwrap();
        let bytes = intent.next().encode().unwrap();
        controller.pending = Some(Pending::Save(intent.clone()));
        controller.accept(store::Reply::Saved {
            intent,
            result: Ok(Arc::from(bytes)),
        });
        controller.observe(world);
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    instance: u64,
    publication: u64,
    known: bool,
    generation: u64,
    library: Uuid,
    privacy: u64,
    privacy_enabled: bool,
}
pub(crate) struct Controller {
    mode: Mode,
    model: Option<Model>,
    projection: Projection,
    worker: Option<Worker>,
    pending: Option<Pending>,
    original: Option<Arc<[u8]>>,
    origin: Option<GridDigest>,
    profile: Option<&'static Path>,
    observed: Option<Stamp>,
}
enum Mode {
    Persistent,
    MemoryOnly(offline::Scope),
}
impl Controller {
    /// No file access or worker startup. Missing actual submitted origin has
    /// no settings/form fallback; offline fixtures remain private-store idle.
    pub(crate) fn bind(world: &World, origin: Option<GridDigest>) -> Self {
        let model = world
            .inventory_archive_instance
            .and_then(|_| origin.and_then(|grid| Model::new(world, grid).ok()));
        let origin = model.as_ref().map(|model| model.source().grid());
        Self {
            mode: Mode::Persistent,
            model,
            projection: Projection::default(),
            worker: None,
            pending: None,
            original: None,
            origin,
            profile: crate::session_host::profile::root(),
            observed: None,
        }
    }
    fn memory(scope: offline::Scope) -> Self {
        Self {
            mode: Mode::MemoryOnly(scope),
            model: None,
            projection: Projection::default(),
            worker: None,
            pending: None,
            original: None,
            origin: None,
            profile: crate::session_host::profile::root(),
            observed: None,
        }
    }
    pub(super) fn demo(world: &World, witness: crate::app::inventory_archive::DemoWitness) -> Option<Self> {
        offline::Scope::demo(world, witness).map(Self::memory)
    }
    #[cfg(test)]
    pub(super) fn offline_fixture(world: &World) -> Option<Self> {
        offline::Scope::fixture(world).map(Self::memory)
    }
    pub(crate) fn memory_only(&self) -> bool {
        matches!(self.mode, Mode::MemoryOnly(_))
    }
    pub(crate) fn retire(&mut self) {
        if let Mode::MemoryOnly(scope) = &mut self.mode {
            scope.retire();
        }
        if let Some(model) = &mut self.model
            && model.status() != Status::Retired
        {
            model.retire();
        }
        self.worker = None;
        self.pending = None;
        self.original = None;
        self.origin = None;
        self.observed = None;
        self.projection.invalidate();
    }
    /// Source mismatch is terminal, never an implicit rebind. The actual World
    /// replacement adapter must also retire BEFORE any Inventory assignment,
    /// including equal-source/equal-generation physical replacement.
    pub(crate) fn observe(&mut self, world: &World) {
        if let Mode::MemoryOnly(scope) = &self.mode {
            if !scope.matches(world) {
                self.retire();
            }
            return;
        }
        if self.profile_changed() || self.model.as_ref().is_some_and(|model| !model.matches(world)) {
            self.retire();
            return;
        }
        let stamp = self.model.as_ref().map(|model| Stamp {
            instance: model.instance,
            publication: model.published.publication(),
            known: matches!(
                model.status(),
                Status::Ready | Status::Saving | Status::SaveFailed | Status::ReadOnly
            ),
            generation: world.inventory.generation,
            library: world.inventory.lib_root,
            privacy: world.rlva.revision,
            privacy_enabled: world.rlva.enabled(),
        });
        if self.observed != stamp {
            self.observed = stamp;
            self.projection.invalidate();
        }
    }
    pub(super) fn profile_changed(&self) -> bool {
        self.profile != crate::session_host::profile::root()
    }
    pub(super) fn original_for_profile_rebind(&self, world: &World) -> Option<GridDigest> {
        self.model.as_ref().filter(|model| model.matches(world)).and(self.origin)
    }
    pub(crate) fn status(&self) -> Option<Status> {
        if self.memory_only() {
            return Some(Status::ReadOnly);
        }
        self.model.as_ref().map(Model::status)
    }
    pub(crate) fn poll(&mut self, world: &World) {
        self.observe(world);
        if self.memory_only() {
            return;
        }
        if self.worker.is_none() && self.model.as_ref().is_some_and(|model| model.status() == Status::Loading) {
            let source = self.model.as_ref().expect("loading model").source();
            match Worker::start(source) {
                Ok(Some(worker)) => self.worker = Some(worker),
                Ok(None) => return, // TWO physically occupied workers: retry next actual turn.
                Err(_) => {
                    let model = self.model.as_mut().expect("loading model");
                    model.complete_load(model.load_ticket().expect("loading ticket"), Err(Error::Unavailable));
                    return;
                }
            }
        }
        if self.pending.is_none()
            && let Some(ticket) = self.model.as_ref().and_then(Model::load_ticket)
        {
            self.pending = Some(Pending::Load(ticket));
            if self
                .worker
                .as_ref()
                .is_none_or(|worker| worker.submit(Request::Load(ticket)).is_err())
            {
                self.transport_failed();
                return;
            }
        }
        let Some(worker) = &self.worker else { return };
        match worker.poll() {
            Ok(Some(reply)) => self.accept(reply),
            Ok(None) => {}
            Err(_) => self.transport_failed(),
        }
        self.observe(world);
    }
    fn accept(&mut self, reply: store::Reply) {
        let Some(model) = &mut self.model else { return };
        match reply {
            store::Reply::Loaded { ticket, result } if self.pending == Some(Pending::Load(ticket)) => {
                let (document, original) = match result {
                    Ok(loaded) => (Ok(loaded.document), loaded.original),
                    Err(error) => (Err(error), None),
                };
                if model.complete_load(ticket, document) {
                    self.original = original;
                    self.pending = None;
                }
            }
            store::Reply::Saved { intent, result }
                if self.pending.as_ref() == Some(&Pending::Save(intent.clone())) && model.complete_save(&intent, result.is_ok()) =>
            {
                if let Ok(original) = result {
                    self.original = Some(original);
                }
                self.pending = None;
            }
            _ => {} // Old source/instance/intent cannot clear current pending state.
        }
    }
    fn transport_failed(&mut self) {
        self.worker = None;
        if let (Some(model), Some(pending)) = (&mut self.model, self.pending.take()) {
            match pending {
                Pending::Load(ticket) => {
                    model.complete_load(ticket, Err(Error::Unavailable));
                }
                Pending::Save(intent) => {
                    model.complete_save(&intent, false);
                }
            }
        }
    }
    pub(crate) fn pump(&mut self, world: &World, turn: &mut CpuTurn) {
        self.observe(world);
        if self.memory_only() {
            return;
        }
        if let Some(model) = &self.model {
            self.projection.pump(world, model, turn);
        }
    }
    /// Original native submitted-login namespace, independent of IO completion.
    /// This is correlation only; it grants neither visibility nor store access.
    pub(crate) fn native_grid(&self, world: &World) -> Option<GridDigest> {
        if self.memory_only() || self.profile_changed() {
            return None;
        }
        self.model
            .as_ref()
            .filter(|model| model.matches(world))
            .map(|model| model.source().grid())
    }
    /// Native publication identity only, for retiring reviewed workspace HUD
    /// work. Does not grant visibility or mutation; callers still borrow their
    /// own ordinary read for every displayed/actionable item. Inventory changes
    /// need not invalidate consent to the unchanged publication.
    pub(crate) fn metadata_revision(&self, world: &World) -> Option<super::Revision> {
        if self.memory_only() || self.profile != crate::session_host::profile::root() {
            return None;
        }
        Some(self.model.as_ref()?.read(world)?.revision())
    }
    pub(crate) fn read<'a>(&'a self, world: &'a World, policy: &'a mut OwnerPolicy) -> Option<VisibilityRead<'a>> {
        if let Mode::MemoryOnly(scope) = &self.mode {
            return Some(self.projection.offline_read(world, scope, policy));
        }
        if self.profile != crate::session_host::profile::root() || self.model.is_none() {
            policy.unavailable();
            return None;
        }
        self.model.as_ref().map(|model| self.projection.read(world, model, policy))
    }
    pub(crate) fn set_include(&self, world: &World, policy: &mut OwnerPolicy, include: bool) -> bool {
        if self.memory_only() {
            return false;
        }
        self.profile == crate::session_host::profile::root()
            && self.model.as_ref().is_some_and(|model| policy.set_include(world, model, include))
    }
    #[cfg(test)]
    pub(crate) fn prepare_edit(&self, world: &World, folder: Uuid) -> Option<EditTicket> {
        if self.memory_only() {
            return None;
        }
        if self.profile != crate::session_host::profile::root() {
            return None;
        }
        self.model.as_ref()?.prepare_edit(world, folder)
    }
    /// Called only by the final App input dispatcher AFTER ctx.run_ui.
    /// That dispatcher must independently revalidate the original painted
    /// owner/context/control/clip/layer/focus/modal/pass relevance; a pure
    /// ticket is necessary, not sufficient, to authorize a UI-origin write.
    pub(crate) fn assign(&mut self, world: &World, original: EditTicket, desired: bool) -> Result<(), Error> {
        #[cfg(test)]
        ASSIGNMENTS.with(|count| count.set(count.get().saturating_add(1)));
        if self.memory_only() {
            return Err(Error::Unavailable);
        }
        self.observe(world);
        let worker = self.worker.as_ref().ok_or(Error::Unavailable)?;
        let model = self.model.as_mut().ok_or(Error::Unavailable)?;
        let intent = model.begin_save(world, original, desired)?;
        self.pending = Some(Pending::Save(intent.clone()));
        if worker
            .submit(Request::Save {
                intent,
                original: self.original.clone(),
            })
            .is_err()
        {
            self.transport_failed();
            return Err(Error::Unavailable);
        }
        Ok(())
    }
}
