//! Per-tab player state shared between the message dispatcher and the
//! MPRIS interface objects.

use crate::protocol::TrackInfo;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// State of a single tab's player.
#[derive(Debug)]
pub struct PlayerState {
    pub track: TrackInfo,
    /// Last position reported by the extension, in microseconds.
    pub last_position_us: i64,
    /// Wall-clock time when `last_position_us` was set.
    pub last_position_at: Instant,
}

impl PlayerState {
    pub fn new(track: TrackInfo) -> Self {
        let last_position_us = (track.position * 1_000_000.0) as i64;
        Self {
            track,
            last_position_us,
            last_position_at: Instant::now(),
        }
    }

    /// Apply an update from the extension. Returns whether the position
    /// changed enough to warrant a `Seeked` signal (jump > 2s).
    pub fn apply_update(&mut self, new_track: TrackInfo) -> PositionDelta {
        let now = Instant::now();
        let new_pos_us = (new_track.position * 1_000_000.0) as i64;

        // Predict what the position would be if we just kept playing without
        // any seek. If the reported position differs significantly from that
        // prediction, treat it as a seek.
        let predicted_us = if self.track.playing {
            let elapsed_us =
                now.duration_since(self.last_position_at).as_micros() as i64;
            self.last_position_us + elapsed_us
        } else {
            self.last_position_us
        };
        let drift_us = (new_pos_us - predicted_us).abs();
        let delta = if drift_us > 2_000_000 {
            PositionDelta::Seeked(new_pos_us)
        } else {
            PositionDelta::Continuous
        };

        self.track = new_track;
        self.last_position_us = new_pos_us;
        self.last_position_at = now;
        delta
    }

    /// Compute the current playback position in microseconds, interpolating
    /// against wall clock if the player is playing.
    pub fn current_position_us(&self) -> i64 {
        if self.track.playing {
            let elapsed_us =
                Instant::now().duration_since(self.last_position_at).as_micros() as i64;
            let pos = self.last_position_us + elapsed_us;
            if self.track.duration > 0.0 {
                let max = (self.track.duration * 1_000_000.0) as i64;
                pos.min(max)
            } else {
                pos
            }
        } else {
            self.last_position_us
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PositionDelta {
    Continuous,
    /// Position jumped — emit MPRIS `Seeked` with this value (microseconds).
    Seeked(i64),
}

/// Bundles the mutable state of a tab with the D-Bus connection that
/// publishes it as an MPRIS player.
#[derive(Debug)]
pub struct PlayerHandle {
    /// Shared with the `PlayerIface` registered on the connection's object
    /// server, so the dispatcher can mutate state and the iface can read it.
    pub state: Arc<Mutex<PlayerState>>,
    pub tab_id: i64,
    /// D-Bus connection holding our well-known name. Dropping this releases
    /// the bus name, which is the cleanest way to "remove" the player —
    /// MPRIS clients see NameOwnerChanged and drop their entry.
    pub _connection: zbus::Connection,
}
