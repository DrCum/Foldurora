//! Small inactive-world entry on the existing Renderer. The native App joins
//! scheduling only after exact source/privacy/lease and aggregate admission.
use super::*;
use crate::monitor::{Config, Frame, Request};

impl Renderer {
    /// Configure only after aggregate admission and current inactive-source,
    /// visibility and privacy checks. None suspends a hidden/minimized/closed
    /// view. The App retains this reservation until actual backend retirement.
    pub fn configure_monitor_prototype(&mut self, config: Option<Config>) {
        self.monitor.configure(config.filter(|_| {
            self.threaded()
                && !self.monitor_release_pending
                && !self.failed
                && !self.lost.get()
                && !self.vr_engaged()
                && !self.vr.wanted()
                && !self.session_fences.pending()
        }));
    }

    /// Revoke before promotion starts. Already submitted work stays reserved
    /// until its callback (or device loss); use the existing ordered session
    /// fence for switching. A Monitor frame never satisfies that proof.
    pub fn stop_monitor_prototype(&mut self) {
        if self.monitor.retire() {
            self.motion_cut = true;
        }
    }

    /// Drain and destroy the small backend before acknowledging a demand Stop.
    /// Revocation alone never frees the host's aggregate resource reservation.
    /// A failed/lost owner cannot prove release: it stays charged until exit.
    pub fn poll_monitor_retirement(&mut self) -> bool {
        self.stop_monitor_prototype();
        self.poll_monitor_prototype();
        if self.failed || self.lost.get() {
            return false;
        }
        if self.monitor.pending() || self.monitor_release_pending {
            return false;
        }
        if !self.monitor_resources {
            return crate::monitor::resources_idle();
        }
        let owner = match &self.host {
            Host::Thread { owner, .. } => owner.borrowed(),
            _ => return false,
        };
        match owner.try_submit(Job::ReleaseMonitor) {
            Ok(Some(submitted)) => {
                if let Some(done) = submitted.previous {
                    self.apply_done(done);
                }
                self.monitor_release_pending = true;
            }
            Ok(None) => {}
            Err(_) => self.fail(),
        }
        false
    }

    /// Completion maintenance runs independently of preview FPS, with no wait
    /// and no new scene work. Returns only the latest completed GPU image.
    pub fn poll_monitor_prototype(&mut self) -> Option<Frame> {
        if self.lost.get() || self.failed {
            self.stop_monitor_prototype();
            return None;
        }
        let owner = match &self.host {
            Host::Thread { owner, .. } => owner.borrowed(),
            _ => return None,
        };
        match owner.try_take_ready() {
            Ok(Some(done)) => self.apply_done(done),
            Ok(None) => {}
            Err(_) => {
                self.fail();
                return None;
            }
        }
        self.monitor.poll();
        if self.monitor.pending() {
            match owner.try_submit(Job::PollMonitor) {
                Ok(Some(submitted)) => {
                    if let Some(done) = submitted.previous {
                        self.apply_done(done);
                    }
                }
                Ok(None) => {}
                Err(_) => self.fail(),
            }
        }
        self.monitor.take_latest().filter(|frame| frame.texture().is_some())
    }

    /// Prepare scene projection/LOD/culling at the supplied SMALL dimensions.
    /// The closure runs only when due and the render worker is already free.
    /// It must use the existing scene and renderer stores, and must not resize
    /// the window, draw UI, send a frame or change Monitor configuration.
    /// Networking/chat ticking belongs outside this closure and its cadence.
    pub fn try_render_monitor_prototype(
        &mut self,
        now: Instant,
        prepare: impl FnOnce(&mut Renderer, (u32, u32)) -> (FrameParams, DrawLists),
    ) -> bool {
        if self.monitor_release_pending
            || self.failed
            || self.lost.get()
            || self.vr_engaged()
            || self.vr.wanted()
            || self.session_fences.pending()
            || self.snapshot_request.is_some()
            || self.capture_request
            || self.capture_scene
        {
            self.stop_monitor_prototype();
            return false;
        }
        self.monitor.poll();
        // Never cold-allocate under unknown/insufficient driver headroom.
        // This is in addition to host aggregate credit, not a total VRAM claim.
        if !self.monitor_resources
            && self
                .viewport_capture_pending
                .as_ref()
                .is_some_and(|frame| frame.request.kind() == crate::viewport_capture::CaptureKind::InactiveWorld)
            && self
                .video_memory
                .read(&self.device)
                .is_none_or(|video| video.budget.saturating_sub(video.usage) < 256 * 1024 * 1024)
        {
            return false;
        }
        if self.monitor.due(now).is_none() {
            return false;
        }
        let owner = match &self.host {
            Host::Thread { owner, .. } => owner.borrowed(),
            _ => return false,
        };
        match owner.try_submit_with(|| {
            let (config, epoch, completion) = self.monitor.begin(now).expect("reserved due Monitor");
            self.monitor_resources = true;
            let viewport_capture = self
                .viewport_capture_pending
                .take_if(|frame| frame.request.kind() == crate::viewport_capture::CaptureKind::InactiveWorld);
            let dimensions = viewport_capture
                .as_ref()
                .map_or(config.dimensions(), |frame| frame.request.layout().size());
            if viewport_capture.is_some() {
                self.viewport_capture_retirement.dirty();
            }
            let (params, lists) = prepare(self, dimensions);
            self.records.flush(&self.device, &mut self.writes);
            self.cull.objects.flush(&self.device, &mut self.writes);
            self.cull.boxes.flush(&self.device, &mut self.writes);
            self.textures.maintain(&self.device, &mut self.writes, &mut self.uploads, false);
            let mut spent = self.spare.pop().unwrap_or_default();
            std::mem::swap(&mut self.writes, &mut spent.writes);
            spent.lists.copy_from(&lists);
            let packet = FramePacket {
                output: None,
                index: self.frames,
                writes: spent.writes,
                uploads: self.uploads.take_frame(),
                scene: self.scene_gpu(),
                params,
                lists: spent.lists,
                ui: None,
                surface: SurfaceState {
                    epoch: self.surface_epoch,
                    width: dimensions.0,
                    height: dimensions.1,
                    vsync: false,
                },
                settings: self.settings,
                hover: None,
                picks: Vec::new(),
                drawn: Vec::new(),
                capture: None,
                viewport_capture,
                histogram: None,
                motion_cut: true,
                input_at: None,
            };
            self.frames += 1;
            Job::Monitor(
                Box::new(packet),
                Request {
                    dimensions,
                    epoch,
                    completion,
                },
            )
        }) {
            Ok(Some(submitted)) => {
                if let Some(done) = submitted.previous {
                    self.apply_done(done);
                }
                true
            }
            Ok(None) => false,
            Err(_) => {
                self.fail();
                false
            }
        }
    }
}
