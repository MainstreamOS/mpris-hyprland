//! `org.mpris.MediaPlayer2.Player` — the playback interface.

use crate::protocol::{Action, FrameId, OutMessage, TabId};
use crate::state::{lock_state, PlayerState};
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
    pub frame_id: FrameId,
}

impl PlayerIface {
    fn send(&self, action: Action, value: Option<f64>) {
        log::info!(
            "[dbus→ext] tab={} frame={} action={action:?} value={value:?}",
            self.tab_id,
            self.frame_id
        );
        if let Err(e) = self.cmd_tx.send(OutMessage::Command {
            tab_id: self.tab_id,
            frame_id: self.frame_id,
            action,
            value,
        }) {
            log::warn!(
                "[dbus→ext] tab={} frame={} action={action:?} channel send failed: {e}",
                self.tab_id,
                self.frame_id
            );
        }
    }

    /// True when there's something playable loaded — drives Can* and the
    /// Paused-vs-Stopped distinction.
    fn has_content(state: &PlayerState) -> bool {
        !state.track.title.is_empty() || state.track.duration > 0.0
    }

    fn build_metadata(&self) -> HashMap<String, OwnedValue> {
        let state = lock_state(&self.state);
        let mut m: HashMap<String, OwnedValue> = HashMap::new();

        // mpris:trackid — required, unique object path. Includes the per-track
        // counter so it changes on a genuine track change and clients reset
        // Position. tab/frame use unsigned magnitude to stay path-valid.
        let track_path = format!(
            "/org/mpris/MediaPlayer2/firefox/t{}/f{}/{}",
            self.tab_id.unsigned_abs(),
            self.frame_id.unsigned_abs(),
            state.track_counter
        );
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

    /// Stop: mark the player Stopped locally (so PlaybackStatus flips to
    /// Stopped immediately, not Paused, and Position reads 0) and tell the
    /// page to halt + rewind.
    async fn stop(&self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        lock_state(&self.state).mark_stopped();
        self.send(Action::Stop, None);
        // Surface the status change now; the page's follow-up update would
        // otherwise report Paused (metadata still present). CanPlay/CanPause
        // derive from has_content, which Stop doesn't change, so they're not
        // re-emitted here.
        self.playback_status_changed(&emitter).await.ok();
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
    /// track_id is accepted loosely.
    async fn set_position(&self, _track_id: ObjectPath<'_>, position: i64) {
        let secs = position as f64 / 1_000_000.0;
        self.send(Action::SetPosition, Some(secs));
    }

    /// OpenUri — not implemented (would require opening a tab in Firefox).
    async fn open_uri(&self, _uri: String) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported("OpenUri is not supported".into()))
    }

    // ---------- properties ----------

    #[zbus(property)]
    fn playback_status(&self) -> String {
        let state = lock_state(&self.state);
        if state.track.playing {
            "Playing".into()
        } else if state.stopped || !Self::has_content(&state) {
            "Stopped".into()
        } else {
            "Paused".into()
        }
    }

    #[zbus(property)]
    fn loop_status(&self) -> String {
        let state = lock_state(&self.state);
        if state.track.looping {
            "Track".into()
        } else {
            "None".into()
        }
    }

    /// LoopStatus write: map None→off, Track/Playlist→on (a browser media
    /// element only has a single-element `loop`, so both "on" values collapse
    /// to element.loop = true).
    #[zbus(property)]
    fn set_loop_status(&self, value: String) {
        let on = value != "None";
        lock_state(&self.state).track.looping = on;
        self.send(Action::SetLoop, Some(if on { 1.0 } else { 0.0 }));
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        let state = lock_state(&self.state);
        state.track.rate.clamp(0.25, 4.0)
    }

    #[zbus(property)]
    fn set_rate(&self, value: f64) {
        let v = value.clamp(0.25, 4.0);
        lock_state(&self.state).track.rate = v;
        self.send(Action::SetRate, Some(v));
    }

    #[zbus(property)]
    fn shuffle(&self) -> bool {
        false
    }

    /// Shuffle write: accepted as a no-op. CanControl is true, so refusing the
    /// set would surface a D-Bus error in clients; there's no per-tab shuffle
    /// concept to honor, so we swallow it.
    #[zbus(property)]
    fn set_shuffle(&self, _value: bool) {}

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        self.build_metadata()
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        let state = lock_state(&self.state);
        state.track.volume.clamp(0.0, 1.0)
    }

    #[zbus(property)]
    fn set_volume(&self, value: f64) {
        let v = value.clamp(0.0, 1.0);
        // Optimistic local update so a subsequent read reflects the request
        // before the page round-trips.
        lock_state(&self.state).track.volume = v;
        self.send(Action::SetVolume, Some(v));
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        let state = lock_state(&self.state);
        state.current_position_us()
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        0.25
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        4.0
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        let state = lock_state(&self.state);
        state.track.can_go_next
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        let state = lock_state(&self.state);
        state.track.can_go_previous
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        let state = lock_state(&self.state);
        Self::has_content(&state)
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        let state = lock_state(&self.state);
        Self::has_content(&state)
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        let state = lock_state(&self.state);
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
