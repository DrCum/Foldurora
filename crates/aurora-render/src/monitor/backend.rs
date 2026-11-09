//! Runs only on the existing queue owner. Shares the device and scene handles;
//! the small view owns its own depth, culling, exposure and temporal resources.
use super::*;
use crate::monitor::{Frame, Request, Targets};

pub(super) struct View {
    backend: Backend,
    targets: Targets,
    #[cfg(test)]
    cold_resources: crate::monitor::ColdResources,
    // All backend/target handles must drop before credit becomes available.
    resources: crate::monitor::ResourceLease,
}

impl Backend {
    // Even a cancelled packet must replay its journal and streamed copies:
    // the scene mirrors have already marked those ranges sent. No image is drawn.
    fn retire_monitor_packet(&mut self, mut packet: FramePacket, request: Request) -> Spent {
        if let Some(frame) = packet.viewport_capture.take()
            && let Some(capture) = &self.viewport_capture
        {
            capture.shared.fail(&frame, crate::viewport_capture::CaptureError::Cancelled);
        }
        if !self.lost.get() {
            packet.writes.replay(&mut TrackedSink {
                queue: &self.queue,
                lost: &self.lost,
                frame: packet.index,
            });
            let commands = packet.uploads.take_commands();
            crate::device_lost::submit_tracked(&self.lost, &self.queue, packet.index, commands);
            packet.uploads.after_submit();
            let completion = request.completion;
            self.queue.on_submitted_work_done(move || completion.finish(None));
        } else {
            request.completion.finish(None);
        }
        Spent {
            writes: packet.writes,
            lists: packet.lists,
        }
    }

    pub(crate) fn render_monitor(&mut self, mut packet: FramePacket, request: Request) -> Spent {
        self.lost.poll(&self.device);
        let size = request.dimensions;
        if !request.epoch.current()
            || self.lost.get()
            || self.monitor_seed.is_none()
            || packet.params.photo_context != crate::PhotoContext::None
        {
            return self.retire_monitor_packet(packet, request);
        }
        // Do not accumulate old target generations when a consumer holds a lease.
        if self
            .monitor_view
            .as_ref()
            .is_some_and(|v| v.targets.size() != size && !v.targets.idle())
        {
            return self.retire_monitor_packet(packet, request);
        }
        if self.monitor_view.is_none() {
            let Some(resources) = crate::monitor::acquire_resources() else {
                return self.retire_monitor_packet(packet, request);
            };
            let mut init = (**self.monitor_seed.as_ref().unwrap()).clone();
            init.external = true;
            init.width = size.0;
            init.height = size.1;
            init.world_viewport = None;
            init.output_format = Some(self.config.format);
            init.viewport_capture = None;
            init.scene = packet.scene.clone();
            // No XR/session proof/pacing state is shared with this offscreen view.
            init.vr = Arc::new(crate::vr_control::Control::new(false));
            init.gpu_frames = Arc::new(crate::pacing::GpuFrames::new(false));
            #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
            {
                init.xr = None;
            }
            match Backend::new_monitor(init) {
                Ok(backend) => {
                    #[cfg(test)]
                    let cold_resources = crate::monitor::ColdResources {
                        samples: backend.msaa_samples,
                        probe_slots: backend.probes.slots,
                        shadow_size: backend.shadow_size,
                        shadow_count: backend.shadow_count,
                    };
                    self.monitor_view = Some(Box::new(View {
                        resources,
                        backend,
                        #[cfg(test)]
                        cold_resources,
                        targets: Targets::new(&self.device, size, self.config.format),
                    }))
                }
                Err(error) => {
                    log::warn!("Monitor prototype backend unavailable: {error}");
                    let spent = self.retire_monitor_packet(packet, request);
                    // Constructor writes and any replay are now submitted.
                    // Even a failed cold allocation keeps its lease until this
                    // actual queue callback; never renew it on a new device.
                    self.queue.on_submitted_work_done(move || drop(resources));
                    return spent;
                }
            }
        }
        if !request.epoch.current() {
            return self.retire_monitor_packet(packet, request);
        }
        let view = self.monitor_view.as_mut().unwrap();
        if view.targets.size() != size {
            view.targets = Targets::new(&self.device, size, self.config.format);
        }
        let Some(texture) = view.targets.acquire() else {
            return self.retire_monitor_packet(packet, request);
        };
        packet.output = Some((*texture).clone());
        packet.surface.width = size.0;
        packet.surface.height = size.1;
        packet.params.world_viewport = None;
        // No photos, interaction readbacks, HUD/UI or temporal samples from the
        // presentation view. RLVa pixel effects on FrameParams remain intact.
        packet.ui = None;
        packet.capture = None;
        let viewport_capture = packet.viewport_capture.take();
        packet.hover = None;
        packet.picks.clear();
        packet.drawn.clear();
        packet.histogram = None;
        packet.input_at = None;
        packet.motion_cut = true;
        packet.lists.strip_viewer_overlays();
        packet.params.probes = Default::default();
        packet.settings = crate::monitor::reduced_settings(packet.settings);
        let index = packet.index;
        let result = view.backend.render_view(packet);
        debug_assert!(!result.stats.presented);
        if let Some(frame) = viewport_capture
            && let Some(capture) = &mut self.viewport_capture
        {
            if result.rendered && request.epoch.current() && !self.lost.get() {
                let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Monitor bounded export"),
                });
                capture.encode_inactive(&self.device, &self.queue, &mut encoder, &texture, frame, index);
                crate::device_lost::submit_tracked(&self.lost, &self.queue, index, [encoder.finish()]);
                capture.after_submit(&self.queue);
            } else {
                capture.shared.fail(&frame, crate::viewport_capture::CaptureError::Cancelled);
            }
        }
        let rendered = result.rendered;
        let frame = Frame {
            _resources: Some(view.resources.clone()),
            texture,
            epoch: request.epoch.clone(),
            lost: self.lost.flag(),
            index,
            stats: result.stats,
            #[cfg(test)]
            cold_resources: Some(view.cold_resources),
        };
        // GPU completion, not render-thread return, publishes the one result.
        self.queue.on_submitted_work_done(move || {
            request.completion.finish((rendered && frame.texture().is_some()).then_some(frame));
        });
        self.lost.poll(&self.device);
        result.spent
    }

    /// Called on the queue owner only after the frontend has observed the
    /// actual completion callback and dropped the retired Lane result. A
    /// remaining texture borrower keeps the reservation, never licenses reuse.
    pub(crate) fn release_monitor(&mut self) -> bool {
        self.lost.poll(&self.device);
        if self.monitor_view.as_ref().is_some_and(|view| !view.targets.idle()) {
            return false;
        }
        self.monitor_view = None;
        true
    }

    pub(crate) fn poll_monitor(&mut self) {
        self.lost.poll(&self.device);
    }
}
