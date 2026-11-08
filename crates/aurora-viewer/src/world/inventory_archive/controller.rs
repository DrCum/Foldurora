//! One actual-source model/IO owner. This stage is registered only for tests;
//! World/App and every ordinary Inventory/chooser consumer must join together.
use super::{
    EditTicket, Error, LoadTicket, Model, SaveIntent, Status, World,
    store::{self, Request, Worker},
    visibility::{CpuTurn, OwnerPolicy, Projection, VisibilityRead},
};
use crate::world::account_metadata_origin::GridDigest;
use std::{path::Path, sync::Arc};
use uuid::Uuid;

#[derive(Clone, PartialEq, Eq)]
enum Pending {
    Load(LoadTicket),
    Save(SaveIntent),
}

#[cfg(test)]
pub(super) mod testing {
    use super::*;
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
    model: Option<Model>,
    projection: Projection,
    worker: Option<Worker>,
    pending: Option<Pending>,
    original: Option<Arc<[u8]>>,
    origin: Option<GridDigest>,
    profile: Option<&'static Path>,
    observed: Option<Stamp>,
}
impl Controller {
    /// No file access or worker startup. Missing actual submitted origin has
    /// no settings/form fallback; offline fixtures remain private-store idle.
    pub(crate) fn bind(world: &World, origin: Option<GridDigest>) -> Self {
        let model = origin.and_then(|grid| Model::new(world, grid).ok());
        let origin = model.as_ref().map(|model| model.source().grid());
        Self {
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
    pub(crate) fn retire(&mut self) {
        if let Some(model) = &mut self.model {
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
        self.model.as_ref().map(Model::status)
    }
    pub(crate) fn poll(&mut self, world: &World) {
        self.observe(world);
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
        if self.pending.is_none() {
            if let Some(ticket) = self.model.as_ref().and_then(Model::load_ticket) {
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
            store::Reply::Saved { intent, result } if self.pending.as_ref() == Some(&Pending::Save(intent.clone())) => {
                if model.complete_save(&intent, result.is_ok()) {
                    if let Ok(original) = result {
                        self.original = Some(original);
                    }
                    self.pending = None;
                }
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
        if let Some(model) = &self.model {
            self.projection.pump(world, model, turn);
        }
    }
    pub(crate) fn read<'a>(&'a self, world: &'a World, policy: &'a mut OwnerPolicy) -> Option<VisibilityRead<'a>> {
        if self.profile != crate::session_host::profile::root() {
            return None;
        }
        self.model.as_ref().map(|model| self.projection.read(world, model, policy))
    }
    pub(crate) fn set_include(&self, world: &World, policy: &mut OwnerPolicy, include: bool) -> bool {
        self.profile == crate::session_host::profile::root()
            && self.model.as_ref().is_some_and(|model| policy.set_include(world, model, include))
    }
    pub(crate) fn prepare_edit(&self, world: &World, folder: Uuid) -> Option<EditTicket> {
        if self.profile != crate::session_host::profile::root() {
            return None;
        }
        self.model.as_ref()?.prepare_edit(world, folder)
    }
    /// Called only by the future final App input dispatcher AFTER ctx.run_ui.
    /// That dispatcher must independently revalidate the original painted
    /// owner/context/control/clip/layer/focus/modal/pass relevance; a pure
    /// ticket is necessary, not sufficient, to authorize a UI-origin write.
    pub(crate) fn assign(&mut self, world: &World, original: EditTicket, desired: bool) -> Result<(), Error> {
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
