//! Per-frame player state shared between the message dispatcher and the
//! MPRIS interface objects.

use crate::protocol::TrackInfo;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// How far the reported position may drift from the wall-clock prediction
/// before we treat it as a seek and emit `Seeked` (microseconds). Lowered
/// from 2s so small arrow-key seeks are reported; the extension also sends an
/// explicit `seeked` flag, which takes precedence over this heuristic.
const SEEK_DRIFT_THRESHOLD_US: i64 = 1_000_000;

/// Which MPRIS properties changed on an update. `update_existing` emits a
/// `PropertiesChanged` only for the ones set here, so steady-state playback
/// (nothing changing but the interpolated Position, which clients poll)
/// produces zero D-Bus signal traffic.
#[derive(Debug, Clone, Copy, Default)]
pub struct Changed {
    pub metadata: bool,
    pub playback_status: bool,
    pub volume: bool,
    pub rate: bool,
    pub loop_status: bool,
    pub can_seek: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
    /// CanPlay/CanPause derive from playback status; emitted together.
    pub can_play_pause: bool,
}

impl Changed {
    pub fn any(&self) -> bool {
        self.metadata
            || self.playback_status
            || self.volume
            || self.rate
            || self.loop_status
            || self.can_seek
            || self.can_go_next
            || self.can_go_previous
            || self.can_play_pause
    }
}

/// Outcome of applying an update: which properties changed, and whether the
/// position jumped (→ emit `Seeked`).
#[derive(Debug)]
pub struct UpdateOutcome {
    pub changed: Changed,
    pub position: PositionDelta,
}

/// State of a single frame's player.
#[derive(Debug)]
pub struct PlayerState {
    pub track: TrackInfo,
    /// Last position reported by the extension, in microseconds.
    pub last_position_us: i64,
    /// Wall-clock time when `last_position_us` was set.
    pub last_position_at: Instant,
    /// Bumped whenever the logical track changes (title/artist/album/url/
    /// duration differ). Feeds `mpris:trackid` so clients reset Position on a
    /// genuine track change (YouTube autoplay, Spotify-web queue) — a stable
    /// per-tab trackid never triggered that reset.
    pub track_counter: u64,
    /// Set when the client called `Stop`; cleared the next time playback
    /// resumes. Lets `PlaybackStatus` report `Stopped` (not `Paused`) even
    /// though the page keeps reporting metadata.
    pub stopped: bool,
}

impl PlayerState {
    pub fn new(track: TrackInfo) -> Self {
        let last_position_us = (track.position * 1_000_000.0) as i64;
        Self {
            track,
            last_position_us,
            last_position_at: Instant::now(),
            track_counter: 0,
            stopped: false,
        }
    }

    /// Apply an update from the extension. Returns which properties changed
    /// and whether a `Seeked` signal is warranted.
    pub fn apply_update(&mut self, new_track: TrackInfo) -> UpdateOutcome {
        let now = Instant::now();
        let new_pos_us = (new_track.position * 1_000_000.0) as i64;

        // Position: trust an explicit seek flag; otherwise compare against the
        // wall-clock prediction and treat a large gap as a seek. The
        // prediction must scale by playback rate to match current_position_us,
        // or fast/slow playback drifts past the threshold and fires spurious
        // Seeked signals.
        let predicted_us = if self.track.playing {
            let elapsed_us = now.duration_since(self.last_position_at).as_micros() as i64;
            let scaled = (elapsed_us as f64 * self.track.rate.max(0.0)) as i64;
            self.last_position_us + scaled
        } else {
            self.last_position_us
        };
        let drift_us = (new_pos_us - predicted_us).abs();
        let position = if new_track.seeked || drift_us > SEEK_DRIFT_THRESHOLD_US {
            PositionDelta::Seeked(new_pos_us.max(0))
        } else {
            PositionDelta::Continuous
        };

        // A logical track change bumps the trackid counter.
        let logical_track_changed = self.track.title != new_track.title
            || self.track.artist != new_track.artist
            || self.track.album != new_track.album
            || self.track.page_url != new_track.page_url
            || (self.track.duration - new_track.duration).abs() > 0.5;
        if logical_track_changed {
            self.track_counter = self.track_counter.wrapping_add(1);
        }

        // Resuming playback clears a prior Stop.
        if new_track.playing {
            self.stopped = false;
        }

        // CanPlay/CanPause derive from "is there content" (title or duration),
        // not from the playing flag — emit them only when that crosses.
        let old_has_content = !self.track.title.is_empty() || self.track.duration > 0.0;
        let new_has_content = !new_track.title.is_empty() || new_track.duration > 0.0;

        // Diff every field that maps to a PropertiesChanged member.
        let changed = Changed {
            metadata: logical_track_changed || self.track.art_url != new_track.art_url,
            playback_status: self.track.playing != new_track.playing,
            volume: (self.track.volume - new_track.volume).abs() > 0.001,
            rate: (self.track.rate - new_track.rate).abs() > 0.001,
            loop_status: self.track.looping != new_track.looping,
            can_seek: self.track.can_seek != new_track.can_seek,
            can_go_next: self.track.can_go_next != new_track.can_go_next,
            can_go_previous: self.track.can_go_previous != new_track.can_go_previous,
            can_play_pause: old_has_content != new_has_content,
        };

        self.track = new_track;
        self.last_position_us = new_pos_us;
        self.last_position_at = now;
        UpdateOutcome { changed, position }
    }

    /// Mark the player Stopped. Per the MPRIS spec a Stopped player reports
    /// Position 0, so zero the position anchor too — keeps the synchronous
    /// Stopped state `stop()` surfaces internally consistent until the page's
    /// follow-up update arrives.
    pub fn mark_stopped(&mut self) {
        self.stopped = true;
        self.track.playing = false;
        self.track.position = 0.0;
        self.last_position_us = 0;
        self.last_position_at = Instant::now();
    }

    /// Compute the current playback position in microseconds, interpolating
    /// against wall clock (scaled by playback rate) if the player is playing.
    /// Never negative.
    pub fn current_position_us(&self) -> i64 {
        let pos = if self.track.playing {
            let elapsed_us = Instant::now()
                .duration_since(self.last_position_at)
                .as_micros() as i64;
            let scaled = (elapsed_us as f64 * self.track.rate.max(0.0)) as i64;
            self.last_position_us + scaled
        } else {
            self.last_position_us
        };
        let pos = pos.max(0);
        if self.track.duration > 0.0 {
            let max = (self.track.duration * 1_000_000.0) as i64;
            pos.min(max)
        } else {
            pos
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PositionDelta {
    Continuous,
    /// Position jumped — emit MPRIS `Seeked` with this value (microseconds).
    Seeked(i64),
}

/// Bundles the mutable state of a frame with the D-Bus connection that
/// publishes it as an MPRIS player.
#[derive(Debug)]
pub struct PlayerHandle {
    /// Shared with the `PlayerIface` registered on the connection's object
    /// server, so the dispatcher can mutate state and the iface can read it.
    pub state: Arc<Mutex<PlayerState>>,
    /// D-Bus connection holding our well-known name. Dropping this releases
    /// the bus name, which is the cleanest way to "remove" the player —
    /// MPRIS clients see NameOwnerChanged and drop their entry.
    pub _connection: zbus::Connection,
}

/// Lock a `PlayerState`, recovering rather than panicking if a previous
/// holder panicked. With `panic = "abort"` a single poisoned lock would
/// otherwise take down the host and every tab's MPRIS player at once.
pub fn lock_state(state: &Mutex<PlayerState>) -> std::sync::MutexGuard<'_, PlayerState> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}
