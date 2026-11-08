//! Worker-only native paths, stopped locks, strict projection and byte CAS.
use super::*;
use crate::session_host::profile::settings_file::{SlotLocks, SourceSettingsFile, TargetSettingsFile, lock_slots};
use crate::settings::profile_seed::ProfileSeed;
use sha2::{Digest, Sha256};

pub(super) struct Prepared {
    source: SourceSettingsFile,
    target: TargetSettingsFile,
    original_source: [u8; 32],
    original_target: Option<Vec<u8>>,
    output: Vec<u8>,
    pub(super) summary: Vec<Summary>,
    locks: SlotLocks,
}
fn check(inspect: &Inspect, deadline: Option<Instant>) -> Result<(), Outcome> {
    if inspect.cancelled.load(Ordering::Acquire) {
        return Err(Outcome::CancelledBeforeCommit);
    }
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(Outcome::Expired);
    }
    super::super::slots::validate_controller(&inspect.capture.installation.to_string(), inspect.capture.controller)
        .map_err(|_| Outcome::Refused(Failure::Stopped))?;
    // The stamp validation itself may block on filesystem IO. Recheck the
    // memory-only latch/deadline after it, immediately before returning.
    if inspect.cancelled.load(Ordering::Acquire) {
        return Err(Outcome::CancelledBeforeCommit);
    }
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(Outcome::Expired);
    }
    Ok(())
}
fn io_error(_: std::io::Error) -> Outcome {
    Outcome::Refused(Failure::Io)
}
impl Prepared {
    pub(super) fn inspect(inspect: &Inspect) -> Result<Self, Outcome> {
        check(inspect, None)?;
        let capture = inspect.capture;
        let installation = capture.installation.to_string();
        let locks = lock_slots(&installation, capture.selection.target, capture.selection.source.slot()).map_err(io_error)?;
        check(inspect, None)?;
        let source = match capture.selection.source {
            Source::MainSaved => SourceSettingsFile::main_saved(),
            Source::Slot(slot) => SourceSettingsFile::stopped_slot(&installation, slot),
        }
        .map_err(io_error)?;
        let target = TargetSettingsFile::slot(&installation, capture.selection.target).map_err(io_error)?;
        let raw_source = source.read().map_err(io_error)?.ok_or(Outcome::Refused(Failure::Io))?;
        let original_source = Sha256::digest(&raw_source).into();
        let seed =
            ProfileSeed::read(&raw_source, capture.selection.families).map_err(|error| Outcome::Refused(Failure::Projection(error)))?;
        // The seed contains selected native types only; discard every raw
        // source value before reading/patching/serializing the destination.
        drop(raw_source);
        check(inspect, None)?;
        let original_target = target.read().map_err(io_error)?;
        let (output, summary) = seed
            .patch(original_target.as_deref())
            .map_err(|error| Outcome::Refused(Failure::Projection(error)))?;
        drop(seed);
        check(inspect, None)?;
        Ok(Self {
            source,
            target,
            original_source,
            original_target,
            output,
            summary,
            locks,
        })
    }

    pub(super) fn apply(self, inspect: &Inspect, deadline: Instant) -> Outcome {
        if let Err(outcome) = check(inspect, Some(deadline)) {
            return outcome;
        }
        let mut refusal = None;
        let result = self.target.publish(&self.output, || {
            let final_checks = (|| {
                check(inspect, Some(deadline))?;
                // Each fresh raw read is dropped before the next. Peak bounded
                // raw/string payload <=16MiB (+read sentinel bytes), selected
                // projection <=256KiB; parse/allocator metadata is separate.
                let source = self.source.read().map_err(io_error)?.ok_or(Outcome::Changed)?;
                let digest: [u8; 32] = Sha256::digest(&source).into();
                drop(source);
                if digest != self.original_source {
                    return Err(Outcome::Changed);
                }
                if self.target.read().map_err(io_error)?.as_deref() != self.original_target.as_deref() {
                    return Err(Outcome::Changed);
                }
                // Last current-controller/cancel/deadline validation, after
                // data+temp checks, with no intentional delay before rename.
                self.locks.check().map_err(io_error)?;
                check(inspect, Some(deadline))
            })();
            if let Err(outcome) = final_checks {
                refusal = Some(outcome);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "profile copy final check refused",
                ));
            }
            Ok(())
        });
        match result {
            // A cancellation after the final check may race a real rename.
            // Its exact original capture must still report Applied.
            Ok(()) => Outcome::Applied,
            Err(_) => refusal.unwrap_or(Outcome::Refused(Failure::Io)),
        }
    }
}
