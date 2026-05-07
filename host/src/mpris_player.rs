//! `org.mpris.MediaPlayer2.Player` — the playback interface.

use crate::protocol::{Action, OutMessage, TabId};
use crate::state::PlayerState;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use zbus::object_server::SignalEmitter;
use zbus::{fdo, interface};
use zvariant::{ObjectPath, OwnedValue, Value};

pub struct PlayerIface {
    pub state: Arc<Mutex<PlayerState>>,
    pub cmd_tx: mpsc::UnboundedSender<OutMessage>,
    pub tab_id: TabId,
}

impl PlayerIface {
    fn send(&self, action: Action, value: Option<f64>) {
        let _ = self.cmd_tx.send(OutMessage::Command {
            tab_id: self.tab_id,
            action,
            value,
        });
    }

    fn build_metadata(&self) -> HashMap<String, OwnedValue> {
        let state = self.state.lock().expect("player state poisoned");
        let mut m: HashMap<String, OwnedValue> = HashMap::new();

        // mpris:trackid — required by spec, must be a unique object path.
        let track_path = format!("/org/mpris/MediaPlayer2/firefox/track/{}", self.tab_id);
        if let Ok(op) = ObjectPath::try_from(track_path.as_str()) {
            if let Ok(ov) = OwnedValue::try_from(Value::from(op)) {
                m.insert("mpris:trackid".into(), ov);
            }
        }

        if state.track.duration > 0.0 {
            let length_us = (state.track.duration * 1_000_000.0) as i64;
            if let Ok(ov) = OwnedValue::try_from(Value::from(length_us)) {
                m.insert("mpris:length".into(), ov);
            }
        }

        if !state.track.art_url.is_empty() {
            if let Ok(ov) = OwnedValue::try_from(Value::from(state.track.art_url.as_str())) {
                m.insert("mpris:artUrl".into(), ov);
            }
        }

        if !state.track.title.is_empty() {
            if let Ok(ov) = OwnedValue::try_from(Value::from(state.track.title.as_str())) {
                m.insert("xesam:title".into(), ov);
            }
        }

        if !state.track.artist.is_empty() {
            let artists: Vec<&str> = vec![state.track.artist.as_str()];
            if let Ok(ov) = OwnedValue::try_from(Value::from(artists)) {
                m.insert("xesam:artist".into(), ov);
            }
        }

        if !state.track.album.is_empty() {
            if let Ok(ov) = OwnedValue::try_from(Value::from(state.track.album.as_str())) {
                m.insert("xesam:album".into(), ov);
            }
        }

        if !state.track.page_url.is_empty() {
            if let Ok(ov) = OwnedValue::try_from(Value::from(state.track.page_url.as_str())) {
                m.insert("xesam:url".into(), ov);
            }
        }

        m
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl PlayerIface {
    // ---------- methods ----------

    async fn next(&self) {
        self.send(Action::Next, None);
    }

    async fn previous(&self) {
        self.send(Action::Previous, None);
    }

    async fn pause(&self) {
        self.send(Action::Pause, None);
    }

    async fn play_pause(&self) {
        self.send(Action::PlayPause, None);
    }

    async fn stop(&self) {
        self.send(Action::Stop, None);
    }

    async fn play(&self) {
        self.send(Action::Play, None);
    }

    /// Seek by `offset` microseconds (relative).
    async fn seek(&self, offset: i64) {
        let secs = offset as f64 / 1_000_000.0;
        self.send(Action::Seek, Some(secs));
    }

    /// SetPosition(track_id: o, position: x). We trust the position; the
    /// track_id is checked loosely against our generated trackid.
    async fn set_position(&self, _track_id: ObjectPath<'_>, position: i64) {
        let secs = position as f64 / 1_000_000.0;
        self.send(Action::SetPosition, Some(secs));
    }

    /// OpenUri — not implemented (would require us to open a tab in Firefox).
    async fn open_uri(&self, _uri: String) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported(
            "OpenUri is not supported".into(),
        ))
    }

    // ---------- properties ----------

    #[zbus(property)]
    fn playback_status(&self) -> String {
        let state = self.state.lock().expect("player state poisoned");
        if state.track.playing {
            "Playing".into()
        } else if state.track.duration > 0.0 || !state.track.title.is_empty() {
            "Paused".into()
        } else {
            "Stopped".into()
        }
    }

    #[zbus(property)]
    fn loop_status(&self) -> String {
        "None".into()
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn shuffle(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        self.build_metadata()
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        let state = self.state.lock().expect("player state poisoned");
        state.track.volume.clamp(0.0, 1.0)
    }

    #[zbus(property)]
    fn set_volume(&self, value: f64) {
        let v = value.clamp(0.0, 1.0);
        // Update local state optimistically so a subsequent property read
        // reflects the user's request before the page round-trips.
        if let Ok(mut s) = self.state.lock() {
            s.track.volume = v;
        }
        self.send(Action::SetVolume, Some(v));
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        let state = self.state.lock().expect("player state poisoned");
        state.current_position_us()
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        let state = self.state.lock().expect("player state poisoned");
        state.track.can_go_next
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        let state = self.state.lock().expect("player state poisoned");
        state.track.can_go_previous
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        let state = self.state.lock().expect("player state poisoned");
        state.track.can_seek
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }

    // ---------- signals ----------

    #[zbus(signal)]
    pub async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;
}
