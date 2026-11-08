//! Actual App event/frame integration. Every ordinary Inventory and Folder
//! Jump consumer uses the same borrowed visibility gate.
use super::App;
use crate::ui::inventory::archive::{Proposal, State};
use crate::world::{World, account_metadata_origin::GridDigest, inventory_archive::offline::Identity};

#[derive(Default)]
pub(super) struct Pending {
    proposal: Option<Proposal>,
    conflict: bool,
    native: crate::ui::inventory::archive::NativeInput,
}
impl Pending {
    pub(super) fn native_event(&mut self, event: &winit::event::WindowEvent) -> bool {
        self.native.event(event)
    }
    pub(super) fn input(&self) -> crate::ui::inventory::archive::InputRead {
        self.native.read()
    }
    pub(super) fn finish_input(&mut self) {
        self.native.finish();
    }
    pub(super) fn begin(&mut self) {
        self.proposal = None;
        self.conflict = false;
    }
    pub(super) fn record(&mut self, proposal: Proposal) {
        match &self.proposal {
            None => self.proposal = Some(proposal),
            Some(original) if original.same(&proposal) => {}
            Some(_) => self.conflict = true,
        }
    }
    fn take(&mut self) -> Option<Proposal> {
        let proposal = self.proposal.take();
        if self.conflict { None } else { proposal }
    }
}

/// Sealed actual App-demo initialization witness. Nil demo login facts are
/// retained and never interpreted as native Source or worker permission.
pub(crate) struct DemoWitness {
    identity: Identity,
}
impl DemoWitness {
    fn for_app(app: &App) -> Option<Self> {
        let world = &app.world;
        let login = world.login.as_ref()?;
        (app.demo
            && world.agent_id == uuid::Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0001)
            && login.agent_id == world.agent_id
            && login.session_id.is_nil()
            && login.inventory_root.is_nil()
            && world.inventory.owner == world.agent_id
            && !world.inventory.root.is_nil()
            && world.inventory.root != world.inventory.lib_root
            && world
                .inventory
                .folders
                .get(&world.inventory.root)
                .is_some_and(|folder| folder.info.id == world.inventory.root && !folder.library && folder.info.type_default != 14))
        .then(|| Identity::observe(world).map(|identity| Self { identity }))
        .flatten()
    }
    pub(crate) fn matches(&self, world: &World) -> bool {
        self.identity.matches(world)
    }
}

impl App {
    /// Only called after the ENTIRE ctx.run_ui, not from a paint/action arm.
    pub(super) fn finish_inventory_archive(&mut self, ctx: &egui::Context) {
        let Some(proposal) = self.inventory_archive_ui.take() else { return };
        self.observe_inventory_archive();
        if !matches!(self.screen, super::Screen::World)
            || self.login_form.busy
            || !self.host_input_allowed()
            || !self.focused
            || self.ctrl
            || self.alt
            || self.shift
            || self.command
            || self.alt_graph
            || !proposal.input_current(self.inventory_archive_ui.input())
            || self.exit_prompt.open
            || self.options_ui.capture.is_some()
            || self.options_ui.joystick_capturing()
            || self.web_has_focus()
            || self.media.focus.is_some()
            || !self
                .inventory_ui
                .archive_proposal_current(ctx, self.panels.inventory, &self.settings.inventory, &proposal)
        {
            return;
        }
        let Some(state) = self.inventory_ui.archive_owner_mut(proposal.owner) else {
            return;
        };
        let instance = state.instance();
        let read = state.read(&self.world);
        let gate = State::ordinary(instance, read.as_ref(), self.world.inventory_archive_instance);
        let include = proposal.include_desired(read.as_ref(), gate);
        let current = proposal.current_gate(gate);
        if let Some(desired) = include {
            state.assign_include(&self.world, desired);
            self.egui_ctx.request_repaint();
            return;
        }
        if current && proposal.dispatch(&mut self.world).is_err() {
            self.inventory_ui.archive_failure(aurora_i18n::tr!("inventory-archive-save-failed"));
        }
    }
    pub(super) fn bind_inventory_archive_login(&mut self, origin: Option<GridDigest>) {
        if self.demo {
            if let Some(witness) = DemoWitness::for_app(self) {
                self.world.bind_demo_inventory_archive(witness);
            } else {
                self.world.retire_inventory_archive();
            }
        } else {
            self.world.bind_inventory_archive(origin);
        }
    }
    pub(super) fn observe_inventory_archive(&mut self) {
        self.world.observe_inventory_archive();
        if !matches!(self.screen, super::Screen::World) || self.login_form.busy || !self.host_input_allowed() || !self.focused {
            self.inventory_ui.retire_archive_input(&self.egui_ctx);
            self.inventory_archive_ui.begin();
        }
    }
    /// The only App admission site is immediately before the whole UI run,
    /// outside all retryable paint closures. World reset cannot replace this
    /// App-owned budget. No observer/native/view helper advances it.
    pub(super) fn pump_inventory_archive(&mut self) {
        self.world.poll_inventory_archive();
        if self.inventory_archive_turn.advance() {
            self.world.pump_inventory_archive(&mut self.inventory_archive_turn);
        }
    }
}
