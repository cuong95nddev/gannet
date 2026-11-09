//! Gannet's core: grabbing the screen, the markup drawn on a capture, the
//! exported image, where captures are kept and the user's settings. No UI
//! here, so all of it is testable without a window.

pub mod capture;
pub mod edits;
pub mod folders;
pub mod model;
pub mod raster;
pub mod settings;
pub mod stitch;
pub mod store;
pub mod trash;
pub mod vault;
