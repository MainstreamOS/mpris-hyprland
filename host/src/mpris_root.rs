//! `org.mpris.MediaPlayer2` — the root MPRIS interface.

use zbus::interface;

pub struct RootIface {
    pub identity: String,
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl RootIface {
    // ---- methods ----

    /// MPRIS spec: bring the player UI to the front. We can't reasonably
    /// raise a specific tab from here without more wiring, so this is a no-op.
    async fn raise(&self) {}

    /// MPRIS spec: quit the player. We don't quit Firefox.
    async fn quit(&self) {}

    // ---- properties ----

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
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
        "firefox"
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
