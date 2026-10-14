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
    fn track_path(&self, counter: u64) -> String {
        format!(
            "/io/github/mainstreamos/mpris_hyprland/w{}/f{}/n{counter}",
            self.tab_id.unsigned_abs(),
            self.frame_id.unsigned_abs()
        )
    }

    fn send(&self, action: Action, value: Option<f64>) {
        let _ = self.cmd_tx.send(OutMessage::Command {
            tab_id: self.tab_id,
            frame_id: self.frame_id,
            action,
            value,
        });
    }

    /// True when there's something playable loaded — drives Can* and the
    /// Paused-vs-Stopped distinction.
    fn has_content(state: &PlayerState) -> bool {
        state.track.has_content()
    }

    fn build_metadata(&self) -> HashMap<String, OwnedValue> {
        let state = lock_state(&self.state);
        let mut m: HashMap<String, OwnedValue> = HashMap::new();

        // mpris:trackid — required, unique object path. Includes the per-track
        // counter so it changes on a genuine track change and clients reset
        // Position. tab/frame use unsigned magnitude to stay path-valid.
        let track_path = self.track_path(state.track_counter);
        if let Ok(op) = ObjectPath::try_from(track_path.as_str()) {
            if let Ok(ov) = OwnedValue::try_from(Value::from(op)) {
                m.insert("mpris:trackid".into(), ov);
            }
        }

        let mut put = |k: &str, v: Value| {
            if let Ok(ov) = OwnedValue::try_from(v) {
                m.insert(k.into(), ov);
            }
        };

        if state.track.duration > 0.0 {
            let length_us = (state.track.duration * 1_000_000.0) as i64;
            put("mpris:length", Value::from(length_us));
        }

        if !state.track.art_url.is_empty() {
            put("mpris:artUrl", Value::from(state.track.art_url.as_str()));
        }

        if !state.track.title.is_empty() {
            put("xesam:title", Value::from(state.track.title.as_str()));
        }

        if !state.track.artist.is_empty() {
            let artists: Vec<&str> = vec![state.track.artist.as_str()];
            put("xesam:artist", Value::from(artists));
        }

        if !state.track.album.is_empty() {
            put("xesam:album", Value::from(state.track.album.as_str()));
        }

        if !state.track.page_url.is_empty() {
            put("xesam:url", Value::from(state.track.page_url.as_str()));
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
        if !lock_state(&self.state).track.can_seek {
            return;
        }
        let state = lock_state(&self.state);
        let target = state.current_position_us().saturating_add(offset);
        if state.track.duration > 0.0 && target > (state.track.duration * 1_000_000.0) as i64 {
            self.send(Action::Next, None);
        } else {
            self.send(Action::Seek, Some(offset as f64 / 1_000_000.0));
        }
    }

    /// Ignore stale scrubs and invalid positions instead of applying them to
    /// a new track that happened to start before the command arrived.
    async fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
        let state = lock_state(&self.state);
        if !state.track.can_seek
            || position < 0
            || track_id.as_str() != self.track_path(state.track_counter)
            || (state.track.duration > 0.0
                && position > (state.track.duration * 1_000_000.0) as i64)
        {
            return;
        }
        self.send(Action::SetPosition, Some(position as f64 / 1_000_000.0));
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
        {
            let mut state = lock_state(&self.state);
            state.last_position_us = state.current_position_us();
            state.last_position_at = std::time::Instant::now();
            state.track.rate = v;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::TrackInfo;
    fn player() -> (PlayerIface, mpsc::UnboundedReceiver<OutMessage>) {
        let (cmd_tx, rx) = mpsc::unbounded_channel();
        (
            PlayerIface {
                state: Arc::new(Mutex::new(PlayerState::new(TrackInfo {
                    title: "Track".into(),
                    duration: 100.0,
                    position: 10.0,
                    ..TrackInfo::default()
                }))),
                cmd_tx,
                tab_id: 1,
                frame_id: 0,
            },
            rx,
        )
    }
    #[tokio::test]
    async fn stale_and_invalid_scrubs_are_ignored() {
        let (player, mut rx) = player();
        let current = player.track_path(0);
        player
            .set_position(ObjectPath::try_from("/stale").unwrap(), 20_000_000)
            .await;
        player
            .set_position(ObjectPath::try_from(current.as_str()).unwrap(), -1)
            .await;
        player
            .set_position(ObjectPath::try_from(current.as_str()).unwrap(), 101_000_000)
            .await;
        assert!(rx.try_recv().is_err());
        player
            .set_position(ObjectPath::try_from(current.as_str()).unwrap(), 20_000_000)
            .await;
        assert!(matches!(
            rx.try_recv().unwrap(),
            OutMessage::Command {
                action: Action::SetPosition,
                value: Some(20.0),
                ..
            }
        ));
        lock_state(&player.state).track_counter += 1;
        player
            .set_position(ObjectPath::try_from(current.as_str()).unwrap(), 30_000_000)
            .await;
        assert!(rx.try_recv().is_err());
    }
    #[tokio::test]
    async fn nonseekable_tracks_ignore_both_methods() {
        let (player, mut rx) = player();
        lock_state(&player.state).track.can_seek = false;
        player.seek(1_000_000).await;
        let current = player.track_path(0);
        player
            .set_position(ObjectPath::try_from(current.as_str()).unwrap(), 20_000_000)
            .await;
        assert!(rx.try_recv().is_err());
    }
    #[tokio::test]
    async fn seeking_past_end_advances_track() {
        let (player, mut rx) = player();
        player.seek(100_000_000).await;
        assert!(matches!(
            rx.try_recv().unwrap(),
            OutMessage::Command {
                action: Action::Next,
                ..
            }
        ));
    }
    #[test]
    fn changing_rate_preserves_current_position() {
        let (player, _) = player();
        {
            let mut state = lock_state(&player.state);
            state.track.playing = true;
            state.last_position_at -= std::time::Duration::from_secs(30);
        }
        player.set_rate(2.0);
        let pos = lock_state(&player.state).current_position_us();
        assert!((40_000_000..41_000_000).contains(&pos));
    }
}
