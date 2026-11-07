//! Native Board prerequisites, not yet a UI/store consumer. Port contract:
//! FSGestureBoardController::{resolve,commit,play,stopBoard,poll}
//! (fsgestureboardcontroller.cpp, abbe3cb0ca66ee2484f277219f6a27706fab7fc6).
//! Saved data contains inventory references only; native permissions and the
//! actual captured session are rechecked by the later App adapter before play.
pub(crate) mod model;
pub(crate) mod playback;
