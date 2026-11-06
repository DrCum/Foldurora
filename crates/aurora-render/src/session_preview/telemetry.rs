//! Completion metadata for prototype cross-worker preview admission. These
//! timestamp-query results are not native presentation or session lease proof.

use std::time::{Duration, Instant};

const MAX_AGE: Duration = Duration::from_millis(750);

#[derive(Clone, Copy, Debug)]
pub struct CompletedGpuSample {
    pub frame: u64,
    pub surface_epoch: u64,
    /// Recorded before encoding the measured native frame. A delayed queue or
    /// delayed poll cannot make an old measurement authorize new optional work.
    pub submitted_at: Instant,
    pub completed_at: Instant,
    pub gpu_ms: f32,
}

impl CompletedGpuSample {
    /// A challenge must have arrived before this frame was encoded. Callers
    /// additionally bind the result to the current account/device/surface and
    /// reject duplicate frame IDs. No remote clock is an authority.
    pub fn fresh_after(self, challenge: Instant, now: Instant) -> bool {
        self.frame != 0
            && self.surface_epoch != 0
            && self.gpu_ms.is_finite()
            && self.gpu_ms > 0.0
            && self.submitted_at >= challenge
            && self.completed_at >= self.submitted_at
            && now.checked_duration_since(self.completed_at).is_some()
            && now.checked_duration_since(self.submitted_at).is_some_and(|age| age <= MAX_AGE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_preview_telemetry_cannot_refresh_old_queued_or_future_measurements() {
        let now = Instant::now();
        let sample = CompletedGpuSample {
            frame: 9,
            surface_epoch: 2,
            submitted_at: now,
            completed_at: now + Duration::from_millis(30),
            gpu_ms: 25.0,
        };
        assert!(sample.fresh_after(now, now + MAX_AGE));
        assert!(!sample.fresh_after(now, now + MAX_AGE + Duration::from_nanos(1)));
        assert!(!sample.fresh_after(now + Duration::from_nanos(1), now + MAX_AGE));
        assert!(!sample.fresh_after(now, now + Duration::from_millis(29)));
        assert!(
            !CompletedGpuSample {
                completed_at: now - Duration::from_nanos(1),
                ..sample
            }
            .fresh_after(now, now + MAX_AGE)
        );
        assert!(
            !CompletedGpuSample {
                gpu_ms: f32::NAN,
                ..sample
            }
            .fresh_after(now, now + MAX_AGE)
        );
        assert!(
            !CompletedGpuSample {
                gpu_ms: f32::INFINITY,
                ..sample
            }
            .fresh_after(now, now + MAX_AGE)
        );
        assert!(!CompletedGpuSample { frame: 0, ..sample }.fresh_after(now, now + MAX_AGE));
    }
}
