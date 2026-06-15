//! `org.mpris.MediaPlayer2` — the root MPRIS interface.

use crate::protocol::{Action, FrameId, OutMessage, TabId};
use tokio::sync::mpsc;
use zbus::interface;

pub struct RootIface {
    pub identity: String,
    /// Basename of the browser's .desktop file, so MPRIS clients resolve the
    /// right app icon (e.g. "zen" on Zen, not "firefox").
    pub desktop_entry: String,
    pub cmd_tx: mpsc::UnboundedSender<OutMessage>,
    pub tab_id: TabId,
    pub frame_id: FrameId,
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl RootIface {
    // ---- methods ----

    /// Bring the player to the front. The player *is* a browser tab, so we
    /// relay this to the background script, which focuses the owning tab and
    /// window (browser.tabs.update + windows.update).
    async fn raise(&self) {
        let _ = self.cmd_tx.send(OutMessage::Command {
            tab_id: self.tab_id,
            frame_id: self.frame_id,
            action: Action::Raise,
            value: None,
        });
    }

    /// Quit the player. A media bar must not be able to quit the browser, so
    /// this stays a no-op (CanQuit is false).
    async fn quit(&self) {}

    // ---- properties ----

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> &str {
        &self.identity
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        &self.desktop_entry
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec!["http".into(), "https".into(), "file".into()]
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        vec![
            "audio/mpeg".into(),
            "audio/ogg".into(),
            "audio/webm".into(),
            "video/mp4".into(),
            "video/webm".into(),
        ]
    }
}
