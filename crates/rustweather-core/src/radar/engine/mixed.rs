//! The worker's side of the mosaic (`radar/mosaic.rs`): which sources a view needs, the joint timeline and the mixed picture.

use std::sync::Arc;

use chrono::{DateTime, Utc};

use super::super::FrameState;
use super::super::mosaic::{self, Weights};
use super::{Command, Provider, Worker};
use crate::map::View;
use crate::map::tiles;

impl Worker {
    /// The weights of the view on screen, kept until it changes.
    pub(super) fn weights(&self) -> Arc<Weights> {
        let mut cached = self.mosaic_weights.borrow_mut();
        if let Some(w) = cached.as_ref().filter(|w| *w.view() == self.view) {
            return Arc::clone(w);
        }
        let w = Arc::new(Weights::new(&self.view));
        *cached = Some(Arc::clone(&w));
        w
    }

    pub(super) fn needs_dwd(&self) -> bool {
        self.kind() == Provider::Dwd || (self.kind() == Provider::Mosaic && self.weights().any_dwd)
    }

    pub(super) fn needs_rv(&self) -> bool {
        self.kind() == Provider::RainViewer || (self.kind() == Provider::Mosaic && !self.weights().all_dwd)
    }

    pub(super) fn mosaic_label(&self) -> String {
        match (self.needs_dwd(), self.needs_rv()) {
            (true, true) => "DWD + RainViewer",
            (true, false) => "DWD",
            _ => "RainViewer",
        }
        .to_owned()
    }

    /// A RainViewer tile is fetched only where the DWD does not count.
    pub(super) fn rv_tile_wanted(&self, z: i32, x: i64, y: i64) -> bool {
        self.kind() != Provider::Mosaic || mosaic::tile_needed(z, x, y)
    }

    fn rv_times(&self) -> Vec<DateTime<Utc>> {
        self.rv.iter().map(|f| f.time).collect()
    }

    pub(super) fn mosaic_times(&self) -> Vec<DateTime<Utc>> {
        let dwd: Vec<_> = self.grid.iter().map(|f| f.time).collect();
        mosaic::times(&dwd, &self.rv_times())
    }

    /// Whether each frame of the joint timeline can be drawn completely.
    pub(super) fn mosaic_states(&self, times: &[DateTime<Utc>]) -> Vec<FrameState> {
        let rv_times = self.rv_times();
        let (dwd_missing, rv_needed) = (self.needs_dwd() && self.grid.is_empty(), self.needs_rv());
        let ready = |i: usize| self.rv.get(i).is_some_and(|f| self.rv_state(f) == FrameState::Ready);
        times
            .iter()
            .map(|t| {
                let rv_ok = !rv_needed
                    || mosaic::rv_position(&rv_times, *t).is_none_or(|r| {
                        let max = rv_times.len().saturating_sub(1);
                        let index = (r.max(0.0) as usize).min(max);
                        ready(index) && (r <= index as f32 || index >= max || ready(index + 1))
                    });
                if dwd_missing || !rv_ok {
                    FrameState::Missing
                } else {
                    FrameState::Ready
                }
            })
            .collect()
    }

    /// Asks for what the view needs and is not there yet, for instance after panning from Spain into Germany.
    pub(super) fn fetch_missing(&mut self) {
        if self.kind() != Provider::Mosaic || !self.active || self.away {
            return;
        }
        if self.needs_dwd() && self.grid.is_empty() {
            self.spawn_dwd();
        }
        if self.needs_rv() && self.rv.is_empty() {
            self.spawn_rv();
        }
    }

    pub(super) fn spawn_dwd(&mut self) {
        if std::mem::replace(&mut self.dwd_busy, true) {
            return;
        }
        let (http, tx, cache) = (self.http.clone(), self.tx.clone(), self.grid_cache.clone());
        self.runtime.spawn(async move {
            let _ = tx.send(Command::DwdLoaded(super::dwd::fetch(&http, &cache).await));
        });
    }

    pub(super) fn spawn_rv(&mut self) {
        if std::mem::replace(&mut self.rv_busy, true) {
            return;
        }
        let (http, tx) = (self.http.clone(), self.tx.clone());
        self.runtime.spawn(async move {
            let _ = tx.send(Command::RvLoaded(super::rainviewer::frames(&http).await));
        });
    }

    /// Both sources for the moment on the timeline, mixed by the reach of the DWD's radars.
    pub(super) fn rain_mosaic(&mut self) -> Vec<u8> {
        let view: View = self.view;
        let weights = self.weights();
        let times = self.mosaic_times();
        let position = if self.position.is_nan() { 0.0 } else { self.position };
        let Some(time) = mosaic::time_at(&times, position) else {
            return vec![0; (view.width * view.height * 4) as usize];
        };
        let dwd = (weights.any_dwd && !self.grid.is_empty()).then(|| self.rain_dwd_at(position));
        let rv = if weights.all_dwd {
            None
        } else {
            mosaic::rv_position(&self.rv_times(), time).map(|r| self.rain_rv_at(r))
        };
        mosaic::blend(dwd.as_deref(), rv.as_deref(), &weights, self.seam)
    }

    /// Tiles of a view that the store does not have, without those the mosaic does not need.
    pub(super) fn rv_missing(&self, view: &View, layer: &tiles::Layer, z: i32) -> Vec<(i64, i64)> {
        tiles::visible(view, layer, z)
            .into_iter()
            .map(|(x, y)| (x.rem_euclid(1 << z), y))
            .filter(|(x, y)| self.rv_tile_wanted(z, *x, *y) && self.store.cached(layer, z, *x, *y).is_none())
            .collect()
    }
}
