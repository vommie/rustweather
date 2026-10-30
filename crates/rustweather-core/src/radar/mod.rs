//! The radar: frames from the DWD grid or RainViewer, the motion between them, and the rain picture of a view.

pub mod clouds;
pub mod dwd;
pub mod engine;
pub mod motion;
pub mod outlook;
pub mod palette;
pub mod rainviewer;
pub mod wind;

use chrono::{DateTime, Utc};

/// One picture of the radar timeline as the UI shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameInfo {
    pub time: DateTime<Utc>,
    pub forecast: bool,
    pub state: FrameState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameState {
    Missing,
    Loading,
    Ready,
    Failed,
}
