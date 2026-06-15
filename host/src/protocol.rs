//! Wire protocol between the WebExtension and this host.
//!
//! Messages are length-prefixed JSON over stdio (Firefox native messaging).
//! See: https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Native_messaging

use serde::{Deserialize, Serialize};

pub type TabId = i64;
pub type FrameId = i64;

/// Identifies a single media-bearing frame. A tab can host media in several
/// frames at once (e.g. an embedded YouTube/Spotify iframe alongside a
/// top-level player), so players are keyed by (tab, frame), not tab alone.
/// Frame 0 is the top-level document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerKey {
    pub tab_id: TabId,
    pub frame_id: FrameId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    /// http(s) or data: URL of cover art / video thumbnail.
    #[serde(default)]
    pub art_url: String,
    /// Page URL the media is playing on.
    #[serde(default)]
    pub page_url: String,
    /// Duration in seconds (browser native unit). 0 / negative means unknown.
    #[serde(default)]
    pub duration: f64,
    /// Current playback position in seconds.
    #[serde(default)]
    pub position: f64,
    /// `true` if currently playing, `false` if paused.
    #[serde(default)]
    pub playing: bool,
    /// `true` if the extension observed a real seek (currentTime jump) for
    /// this update. Lets the host emit `Seeked` precisely instead of inferring
    /// it from wall-clock drift.
    #[serde(default)]
    pub seeked: bool,
    /// Volume in 0.0..=1.0 (effective: muted reports 0.0).
    #[serde(default = "default_volume")]
    pub volume: f64,
    /// Playback rate (1.0 = normal).
    #[serde(default = "default_rate")]
    pub rate: f64,
    /// Whether the active media element has `loop` set.
    #[serde(default)]
    pub looping: bool,
    /// Whether the source supports seeking.
    #[serde(default = "default_can_seek")]
    pub can_seek: bool,
    /// Whether the page advertised a "next" handler via Media Session.
    #[serde(default)]
    pub can_go_next: bool,
    /// Whether the page advertised a "previous" handler via Media Session.
    #[serde(default)]
    pub can_go_previous: bool,
}

fn default_volume() -> f64 {
    1.0
}
fn default_rate() -> f64 {
    1.0
}
fn default_can_seek() -> bool {
    true
}

impl Default for TrackInfo {
    fn default() -> Self {
        Self {
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            art_url: String::new(),
            page_url: String::new(),
            duration: 0.0,
            position: 0.0,
            playing: false,
            seeked: false,
            volume: 1.0,
            rate: 1.0,
            looping: false,
            can_seek: true,
            can_go_next: false,
            can_go_previous: false,
        }
    }
}

/// Message FROM the extension TO this host.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum InMessage {
    /// Initial handshake from the extension's background script. Optional.
    Hello {
        #[serde(default)]
        version: String,
    },
    /// A frame started or resumed having active media. Creates the MPRIS
    /// player if it doesn't exist and applies the supplied state.
    Update {
        tab_id: TabId,
        #[serde(default)]
        frame_id: FrameId,
        #[serde(flatten)]
        track: TrackInfo,
    },
    /// A frame no longer has active media (element gone, navigated away, or
    /// tab/frame closed). Removes the MPRIS player.
    Remove {
        tab_id: TabId,
        #[serde(default)]
        frame_id: FrameId,
    },
    /// Heartbeat / keep-alive. Ignored.
    Ping,
}

/// Message FROM this host TO the extension. Relays D-Bus method calls
/// (media keys, seek/raise/loop requests from waybar/quickshell) back to the
/// page or — for `Raise` — to the background script.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum OutMessage {
    Command {
        tab_id: TabId,
        #[serde(default)]
        frame_id: FrameId,
        action: Action,
        /// Optional numeric value (seek offset in seconds, set position in
        /// seconds, volume 0..1, rate, loop 0/1).
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<f64>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
    Stop,
    /// Relative seek by `value` seconds (signed).
    Seek,
    /// Absolute seek to `value` seconds.
    SetPosition,
    /// Set volume to `value` (0..1).
    SetVolume,
    /// Set playback rate to `value`.
    SetRate,
    /// Set the active element's `loop` flag (`value` 1.0 = on, 0.0 = off).
    SetLoop,
    /// Focus the owning tab/window (handled by the background script, not the
    /// page). No `value`.
    Raise,
}
