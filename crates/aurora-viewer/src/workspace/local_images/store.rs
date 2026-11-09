//! One bounded native IO lane. Normalize and publish are separate operations:
//! selecting a file cannot write a profile before App rechecks the original
//! source/intent after the native dialog has returned.
pub(crate) use super::codec::{Decoded, Failure};
use super::codec::{decode, normalize};
use super::model::{Image, Library};
use crate::{
    session_host::profile::{NativeDirectory, NativeStoreCancellation, NativeStoreKind},
    workspace::controller::Source,
};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use uuid::Uuid;

pub(crate) enum Operation {
    Choose(String),
    Load(Vec<Uuid>),
    Publish(Image),
}
pub(crate) struct Request {
    pub source: Source,
    pub binding: u64,
    pub operation: u64,
    pub directory: NativeDirectory,
    pub cancellation: NativeStoreCancellation,
    pub action: Operation,
}
pub(crate) enum Outcome {
    Prepared(Option<Image>),
    Loaded(Vec<Decoded>),
    Published(Image),
}
pub(crate) struct Reply {
    pub source: Source,
    pub binding: u64,
    pub operation: u64,
    pub finished: std::time::Instant,
    pub result: Result<Outcome, Failure>,
}
pub(crate) struct Lane {
    send: SyncSender<Request>,
    receive: Receiver<Reply>,
    busy: std::cell::Cell<bool>,
}
impl Lane {
    pub(crate) fn new() -> std::io::Result<Self> {
        let (send, requests) = sync_channel::<Request>(1);
        let (results, receive) = sync_channel(1);
        std::thread::Builder::new().name("aurora-workspace-images".into()).spawn(move || {
            while let Ok(r) = requests.recv() {
                let result = perform(&r);
                if results
                    .send(Reply {
                        source: r.source,
                        binding: r.binding,
                        operation: r.operation,
                        finished: std::time::Instant::now(),
                        result,
                    })
                    .is_err()
                {
                    break;
                }
            }
        })?;
        Ok(Self {
            send,
            receive,
            busy: std::cell::Cell::new(false),
        })
    }
    pub(crate) fn submit(&self, request: Request) -> bool {
        if self.busy.replace(true) {
            return false;
        }
        if self.send.try_send(request).is_ok() {
            true
        } else {
            self.busy.set(false);
            false
        }
    }
    pub(crate) fn poll(&self) -> Result<Option<Reply>, ()> {
        match self.receive.try_recv() {
            Ok(r) => {
                self.busy.set(false);
                Ok(Some(r))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
            Err(_) => {
                self.busy.set(false);
                Err(())
            }
        }
    }
}
fn cancelled(r: &Request) -> Result<(), Failure> {
    if r.cancellation.is_cancelled() {
        Err(Failure::Cancelled)
    } else {
        Ok(())
    }
}

fn read(r: &Request) -> Result<Library, Failure> {
    let bytes = match r.directory.read() {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(Failure::Unavailable),
    };
    let library = bytes.as_deref().map_or_else(
        || Ok(Library::empty(r.source.agent, r.source.grid.clone())),
        |b| Library::decode(b, r.source.agent, &r.source.grid).map_err(|_| Failure::Invalid),
    )?;
    for image in &library.images {
        cancelled(r)?;
        decode(image)?;
    }
    Ok(library)
}
fn perform(r: &Request) -> Result<Outcome, Failure> {
    if r.binding == 0
        || r.operation == 0
        || r.source.login.is_nil()
        || !r
            .directory
            .matches(NativeStoreKind::WorkspaceImages, r.source.agent, &r.source.grid)
    {
        return Err(Failure::Invalid);
    }
    cancelled(r)?;
    match &r.action {
        Operation::Choose(title) => {
            let path = rfd::FileDialog::new()
                .set_title(title)
                .add_filter("PNG / JPEG / BMP / TGA", &["png", "jpg", "jpeg", "bmp", "tga"])
                .pick_file();
            cancelled(r)?;
            path.map_or(Ok(Outcome::Prepared(None)), |p| normalize(&p).map(|i| Outcome::Prepared(Some(i))))
        }
        Operation::Load(ids) => {
            if ids.len() > 18 || ids.iter().any(Uuid::is_nil) {
                return Err(Failure::Invalid);
            }
            // Passive restore never creates a directory, file or lock.
            let library = read(r)?;
            let mut decoded = Vec::new();
            for image in &library.images {
                if ids.contains(&image.id) {
                    cancelled(r)?;
                    decoded.push(decode(image)?);
                }
            }
            Ok(Outcome::Loaded(decoded))
        }
        Operation::Publish(image) => {
            decode(image)?;
            cancelled(r)?;
            let _lock = r.directory.lock().map_err(|_| Failure::Unavailable)?;
            let original = r.directory.read().map_err(|_| Failure::Unavailable)?;
            let mut library = original.as_deref().map_or_else(
                || Ok(Library::empty(r.source.agent, r.source.grid.clone())),
                |b| Library::decode(b, r.source.agent, &r.source.grid).map_err(|_| Failure::Invalid),
            )?;
            for current in &library.images {
                cancelled(r)?;
                decode(current)?;
            }
            library.insert(image.clone()).map_err(|_| Failure::Full)?;
            let bytes = library.encode().map_err(|_| Failure::Full)?;
            cancelled(r)?;
            r.directory
                .publish_cancellable(&bytes, original.as_deref(), &r.cancellation)
                .map_err(|_| Failure::Unavailable)?;
            // A successful rename receipt stays success if cancellation races
            // inside publication. App may retire the old intent, never replay it.
            Ok(Outcome::Published(image.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image() -> Image {
        let pixels = image::RgbaImage::from_pixel(8, 4, image::Rgba([50, 100, 200, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        Image {
            id: Uuid::new_v4(),
            original: [8, 4],
            png: super::super::model::Png::new(bytes.into_inner()).unwrap(),
        }
    }
    fn request(action: Operation) -> Request {
        let source = Source {
            agent: Uuid::new_v4(),
            login: Uuid::new_v4(),
            grid: "a".repeat(64),
            hosted: None,
        };
        Request {
            directory: NativeDirectory::select(NativeStoreKind::WorkspaceImages, source.agent, &source.grid).unwrap(),
            source,
            binding: 1,
            operation: 1,
            cancellation: NativeStoreCancellation::new(),
            action,
        }
    }
    #[test]
    fn native_workspace_images_missing_load_and_cancelled_publication_never_create_a_store() {
        let mut r = request(Operation::Load(vec![Uuid::new_v4()]));
        assert!(matches!(r.directory.read(),Err(e) if e.kind()==std::io::ErrorKind::NotFound));
        assert!(matches!(perform(&r),Ok(Outcome::Loaded(v)) if v.is_empty()));
        assert!(matches!(r.directory.read(),Err(e) if e.kind()==std::io::ErrorKind::NotFound));
        r.action = Operation::Publish(image());
        r.cancellation.cancel();
        assert!(matches!(perform(&r), Err(Failure::Cancelled)));
        assert!(matches!(r.directory.read(),Err(e) if e.kind()==std::io::ErrorKind::NotFound));
    }
    #[test]
    fn native_workspace_images_publish_load_exact_pixels_and_refuse_identity_or_immutable_id_replacement() {
        let image = image();
        let mut r = request(Operation::Publish(image.clone()));
        assert!(matches!(perform(&r),Ok(Outcome::Published(ref i)) if i==&image));
        let original = r.directory.read().unwrap().unwrap();
        assert!(matches!(perform(&r), Err(Failure::Full)));
        assert_eq!(r.directory.read().unwrap().unwrap(), original);
        r.action = Operation::Load(vec![image.id]);
        let loaded = match perform(&r).unwrap() {
            Outcome::Loaded(v) => v,
            _ => panic!("actual managed load"),
        };
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, image.id);
        assert_eq!(loaded[0].size, [8, 4]);
        assert!(loaded[0].rgba.as_chunks::<4>().0.iter().all(|p| *p == [50, 100, 200, 255]));
        r.source.agent = Uuid::new_v4();
        assert!(matches!(perform(&r), Err(Failure::Invalid)));
        assert_eq!(r.directory.read().unwrap().unwrap(), original);
    }
    #[test]
    fn native_workspace_images_corrupt_library_is_not_repaired_or_overwritten_by_an_import() {
        let r = request(Operation::Publish(image()));
        {
            let _lock = r.directory.lock().unwrap();
            r.directory.publish(b"{unrecognized library", None).unwrap();
        }
        assert!(matches!(perform(&r), Err(Failure::Invalid)));
        assert_eq!(r.directory.read().unwrap().unwrap(), b"{unrecognized library");
    }
}
