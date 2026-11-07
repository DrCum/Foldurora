//! Native Board data, exact playback ownership and private metadata store. Port contract:
//! FSGestureBoardController::{resolve,commit,play,stopBoard,poll}
//! (fsgestureboardcontroller.cpp, abbe3cb0ca66ee2484f277219f6a27706fab7fc6).
//! Saved data contains inventory references only; native permissions and the
//! actual captured session are rechecked by the App adapter before play.
pub(crate) mod model;
pub(crate) mod playback;
pub(crate) mod store;
