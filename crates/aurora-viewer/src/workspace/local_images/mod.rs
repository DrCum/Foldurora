//! Managed local workspace images. Storage uses the existing private native
//! directory/lock/publisher; no reference can contain an external file path.
pub(crate) mod codec;
pub(crate) mod intent;
pub(crate) mod model;
pub(crate) mod store;
