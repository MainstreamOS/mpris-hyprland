//! Wire protocol between the WebExtension and this host.
//!
//! Messages are length-prefixed JSON over stdio (Firefox native messaging).
//! See: https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Native_messaging

use serde::{Deserialize, Serialize};

pub type TabId = i64;

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
    /// Volume in 0.0..=1.0.
    #[serde(default = "default_volume")]
    pub volume: f64,
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
            volume: 1.0,
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
    /// A tab started or resumed having active media. Creates the MPRIS player
    /// if it doesn't exist and applies the supplied state.
    Update {
        tab_id: TabId,
        #[serde(flatten)]
        track: TrackInfo,
    },
    /// A tab no longer has active media (paused with no metadata, navigated away,
    /// or closed). Removes the MPRIS player.
    Remove { tab_id: TabId },
    /// Heartbeat / keep-alive. Ignored.
    Ping,
}

/// Message FROM this host TO the extension. Used to relay D-Bus method calls
/// (e.g. media keys, seek requests from waybar/quickshell) back to the page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum OutMessage {
    Command {
        tab_id: TabId,
        action: Action,
        /// Optional numeric value (seek offset in seconds, set position in seconds, volume 0..1).
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<f64>,
    },
    /// Host is shutting down. Extension can ignore.
    Bye,
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
}
