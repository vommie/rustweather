//! The RainViewer frames of the view as whole pictures, kept while the view stays, with the motion between neighbors: playing
//! then only mixes two kept pictures instead of putting each one together from its tiles again.

use std::collections::HashMap;
use std::sync::Arc;

use super::flow::{self, Flow};
use crate::map::View;
use crate::map::tiles::Backdrop;

/// Frames this far from the one on screen are let go.
const KEEP: usize = 2;

#[derive(Default)]
pub struct RvFrames {
    view: Option<View>,
    pictures: HashMap<usize, Arc<Vec<u8>>>,
    flows: HashMap<usize, Arc<Flow>>,
    /// Each frame's last picture, also of an older view: it fills the gaps while the tiles of a new zoom load.
    last: HashMap<usize, Backdrop>,
}

impl RvFrames {
    /// Forgets everything when the view changed, and the frames far from `index`.
    pub fn keep(&mut self, view: &View, index: usize) {
        if self.view.as_ref() != Some(view) {
            let last = std::mem::take(&mut self.last);
            *self = Self {
                view: Some(*view),
                last,
                ..Self::default()
            };
        }
        let near = |i: &usize| i.abs_diff(index) <= KEEP;
        self.pictures.retain(|i, _| near(i));
        self.flows.retain(|i, _| near(i));
    }

    /// Also when the frames themselves changed, after a new list came.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn picture(&self, index: usize) -> Option<Arc<Vec<u8>>> {
        self.pictures.get(&index).cloned()
    }

    pub fn last(&self, index: usize) -> Option<Backdrop> {
        self.last.get(&index).cloned()
    }

    pub fn remember(&mut self, index: usize, picture: Backdrop) {
        self.last.insert(index, picture);
    }

    pub fn put(&mut self, index: usize, rgba: Arc<Vec<u8>>) {
        self.pictures.insert(index, rgba);
    }

    /// The motion from frame `index` to the next one.
    pub fn flow(&mut self, index: usize, a: &[u8], b: &[u8], width: usize, height: usize) -> Arc<Flow> {
        Arc::clone(
            self.flows
                .entry(index)
                .or_insert_with(|| Arc::new(flow::estimate(a, b, width, height))),
        )
    }
}

/// A frame at `opacity`, without motion.
pub fn faded(rgba: &[u8], opacity: f32) -> Vec<u8> {
    let mut out = rgba.to_vec();
    if opacity < 1.0 {
        for pixel in out.as_chunks_mut::<4>().0 {
            pixel[3] = (f32::from(pixel[3]) * opacity) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(lon: f64) -> View {
        View {
            lon,
            lat: 50.0,
            zoom: 6.0,
            width: 100,
            height: 80,
        }
    }

    #[test]
    fn a_new_view_forgets_the_pictures_and_far_frames_go() {
        let mut frames = RvFrames::default();
        frames.keep(&view(10.0), 5);
        frames.put(5, Arc::new(vec![1]));
        frames.put(9, Arc::new(vec![2]));
        frames.keep(&view(10.0), 5);
        assert!(frames.picture(5).is_some() && frames.picture(9).is_none());
        frames.keep(&view(11.0), 5);
        assert!(frames.picture(5).is_none());
    }

    #[test]
    fn fading_scales_only_the_alpha() {
        assert_eq!(faded(&[10, 20, 30, 200], 0.5), [10, 20, 30, 100]);
    }
}
