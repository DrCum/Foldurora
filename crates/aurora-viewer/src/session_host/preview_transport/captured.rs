//! Own the renderer's already-credited allocation; never allocate/copy RGBA.
use super::Error;
use aurora_render::viewport_capture::{CapturePublicationGuard, CaptureToken, CapturedViewport};
use aurora_session::preview_image::PreviewStamp;

enum Publication {
    Renderer(CapturePublicationGuard),
    Native(Box<super::super::preview_io::CapturePermitGuard>),
    // Codec/resource fixtures only. This is not a native/render permit and is
    // inaccessible outside this private test boundary.
    #[cfg(test)]
    Fixture {
        token: CaptureToken,
        current: std::sync::Arc<std::sync::atomic::AtomicBool>,
    },
}
pub(super) struct CapturedBody {
    frame: CapturedViewport,
    publication: Publication,
}
impl CapturedBody {
    pub fn new(stamp: PreviewStamp, frame: CapturedViewport, guard: CapturePublicationGuard) -> Result<Self, Error> {
        if guard.token() != frame.request().token() || !guard.current() {
            return Err(Error::Expired);
        }
        Self::checked(stamp, frame, Publication::Renderer(guard))
    }
    fn checked(stamp: PreviewStamp, frame: CapturedViewport, publication: Publication) -> Result<Self, Error> {
        let r = frame.request();
        let b = r.boundary();
        let view = r.viewport();
        let g = stamp.geometry();
        if stamp.frame() != frame.frame()
            || stamp.surface_epoch() != b.surface_epoch()
            || g.source_size() != [b.size().0, b.size().1]
            || g.rect() != [view.x, view.y, view.width, view.height]
            || g.size() != [r.layout().size().0, r.layout().size().1]
            || g.payload_bytes() != frame.pixels().bytes().len()
        {
            return Err(Error::Binding);
        }
        Ok(Self { frame, publication })
    }
    pub(super) fn guarded(
        stamp: PreviewStamp,
        frame: CapturedViewport,
        guard: Box<super::super::preview_io::CapturePermitGuard>,
    ) -> Result<Self, Error> {
        if !guard.accepts(stamp, &frame, std::time::Instant::now()) {
            return Err(Error::Expired);
        }
        Self::checked(stamp, frame, Publication::Native(guard))
    }
    pub fn check(&self) -> Result<(), Error> {
        if let Publication::Native(guard) = &self.publication {
            return guard.transport_check();
        }
        if self.current() { Ok(()) } else { Err(Error::Expired) }
    }
    pub fn current(&self) -> bool {
        match &self.publication {
            Publication::Native(guard) => guard.current(std::time::Instant::now()),
            Publication::Renderer(guard) => guard.current() && guard.token() == self.frame.request().token(),
            #[cfg(test)]
            Publication::Fixture { token, current } => {
                *token == self.frame.request().token() && current.load(std::sync::atomic::Ordering::Acquire)
            }
        }
    }
    pub fn bytes(&self) -> &[u8] {
        self.frame.pixels().bytes()
    }
    #[cfg(test)]
    pub(super) fn fixture(
        stamp: PreviewStamp,
        frame: CapturedViewport,
        current: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Self, Error> {
        let token = frame.request().token();
        Self::checked(stamp, frame, Publication::Fixture { token, current })
    }
}
