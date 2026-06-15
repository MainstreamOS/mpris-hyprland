//! mpris-hyprland-host
//!
//! Native messaging host for the mpris-hyprland WebExtension.
//! Reads length-prefixed JSON on stdin (per Mozilla's native messaging
//! protocol), forwards each frame's media state onto the session D-Bus as a
//! distinct MPRIS player, and forwards method calls (Play/Pause/Next/Seek/
//! Raise/...) from D-Bus back to the extension on stdout so the page can act
//! on them.

mod messaging;
mod mpris_player;
mod mpris_root;
mod protocol;
mod state;

use anyhow::{Context, Result};
use protocol::{InMessage, OutMessage, PlayerKey, TrackInfo};
use state::{lock_state, PlayerHandle, PlayerState, PositionDelta, UpdateOutcome};
use std::collections::HashMap;
use std::process;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use tokio::io::BufWriter;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

/// Browser .desktop basename, detected once at startup (see detect_desktop_entry).
static DESKTOP_ENTRY: OnceLock<String> = OnceLock::new();

const PLAYER_OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";

/// A known browser family. `needles` are substrings of the process `comm` that
/// identify it; `entry` is the `.desktop` basename MPRIS clients use for the
/// icon; `segment` is the MPRIS bus-name segment (Firefox-family browsers, incl.
/// Zen/LibreWolf, publish under org.mpris.MediaPlayer2.firefox.instance<pid>_t<window>
/// so status bars that special-case the per-window bridge keep matching;
/// Chromium-family browsers publish under …chromium… so the bridge is distinct
/// from — and dedups against — Chromium's own native single player); `identity`
/// is the human-readable MPRIS Identity base.
struct Browser {
    needles: &'static [&'static str],
    entry: &'static str,
    segment: &'static str,
    identity: &'static str,
}

/// Single source of truth for browser families. Order matters for comm matching:
/// most-specific needles first ("chromium" before "chrome", "google-chrome"
/// before "chrome") so a comm doesn't fall through to the wrong family.
const BROWSERS: &[Browser] = &[
    Browser { needles: &["zen"], entry: "zen", segment: "firefox", identity: "Zen" },
    Browser { needles: &["librewolf"], entry: "librewolf", segment: "firefox", identity: "LibreWolf" },
    Browser { needles: &["floorp"], entry: "floorp", segment: "firefox", identity: "Floorp" },
    Browser { needles: &["waterfox"], entry: "waterfox", segment: "firefox", identity: "Waterfox" },
    Browser { needles: &["mullvad"], entry: "mullvad-browser", segment: "firefox", identity: "Mullvad Browser" },
    Browser { needles: &["chromium"], entry: "chromium", segment: "chromium", identity: "Chromium" },
    Browser { needles: &["brave"], entry: "brave-browser", segment: "chromium", identity: "Brave" },
    Browser { needles: &["vivaldi"], entry: "vivaldi-stable", segment: "chromium", identity: "Vivaldi" },
    Browser { needles: &["microsoft-edge", "msedge"], entry: "microsoft-edge", segment: "chromium", identity: "Microsoft Edge" },
    Browser { needles: &["google-chrome", "chrome"], entry: "google-chrome", segment: "chromium", identity: "Google Chrome" },
    Browser { needles: &["opera"], entry: "opera", segment: "chromium", identity: "Opera" },
    Browser { needles: &["firefox"], entry: "firefox", segment: "firefox", identity: "Firefox" },
];

/// The detected browser's `.desktop` basename, or "firefox" before init / when unknown.
fn desktop_entry() -> &'static str {
    DESKTOP_ENTRY.get().map(String::as_str).unwrap_or("firefox")
}

/// The detected browser's family record, if it's a known one.
fn detected_browser() -> Option<&'static Browser> {
    BROWSERS.iter().find(|b| b.entry == desktop_entry())
}

/// Detect the browser's .desktop basename so MPRIS clients resolve the correct
/// icon. Order: $MPRIS_HYPRLAND_DESKTOP_ENTRY override → parent process comm
/// (the host is a child of the browser) → "firefox". On Zen the parent comm is
/// "zen-bin" and the desktop file is zen.desktop, so plain "firefox" would
/// leave clients with a generic icon.
fn detect_desktop_entry() -> String {
    if let Ok(v) = std::env::var("MPRIS_HYPRLAND_DESKTOP_ENTRY") {
        if !v.is_empty() {
            return v;
        }
    }
    let parent_comm = parent_comm().unwrap_or_default().to_ascii_lowercase();
    BROWSERS
        .iter()
        .find(|b| b.needles.iter().any(|n| parent_comm.contains(n)))
        .map(|b| b.entry)
        .unwrap_or("firefox")
        .to_string()
}

/// MPRIS bus-name segment for the detected browser family (see `Browser`).
fn bus_segment() -> &'static str {
    detected_browser().map(|b| b.segment).unwrap_or("firefox")
}

/// Human-readable MPRIS Identity base for the detected browser family.
fn app_identity() -> &'static str {
    detected_browser().map(|b| b.identity).unwrap_or("Firefox")
}

/// Read the parent process's `comm` (its short name) via /proc.
fn parent_comm() -> Option<String> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let ppid = status
        .lines()
        .find_map(|l| l.strip_prefix("PPid:"))
        .map(|s| s.trim())?;
    let comm = std::fs::read_to_string(format!("/proc/{ppid}/comm")).ok()?;
    Some(comm.trim().to_string())
}

type PlayerMap = Arc<AsyncMutex<HashMap<PlayerKey, Arc<PlayerHandle>>>>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    // STDOUT is the native messaging channel and must remain pristine
    // length-prefixed JSON.
    let _ = DESKTOP_ENTRY.set(detect_desktop_entry());

    let players: PlayerMap = Arc::new(AsyncMutex::new(HashMap::new()));

    // Outbound channel: any task pushes OutMessage; one writer task drains
    // them to stdout, framed.
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<OutMessage>();

    let writer_task = tokio::spawn(async move {
        let stdout = tokio::io::stdout();
        let mut out = BufWriter::new(stdout);
        while let Some(msg) = cmd_rx.recv().await {
            let bytes = match serde_json::to_vec(&msg) {
                Ok(b) => b,
                Err(_) => {
                    continue;
                }
            };
            if messaging::write_message(&mut out, &bytes).await.is_err() {
                break;
            }
        }
    });

    let mut stdin = tokio::io::stdin();
    loop {
        let payload = match messaging::read_message(&mut stdin).await {
            Ok(Some(p)) => p,
            Ok(None) => {
                break;
            }
            Err(_) => {
                break;
            }
        };

        let msg: InMessage = match serde_json::from_slice(&payload) {
            Ok(m) => m,
            Err(_) => {
                continue;
            }
        };

        let _ = handle_message(msg, &players, &cmd_tx).await;
    }

    // Clean up: drop all player connections, then close the writer.
    {
        let mut guard = players.lock().await;
        guard.clear();
    }
    drop(cmd_tx);
    let _ = writer_task.await;
    Ok(())
}

async fn handle_message(
    msg: InMessage,
    players: &PlayerMap,
    cmd_tx: &mpsc::UnboundedSender<OutMessage>,
) -> Result<()> {
    match msg {
        InMessage::Hello { version: _ } => {}
        InMessage::Ping => {}
        InMessage::Update {
            tab_id,
            frame_id,
            track,
        } => {
            let key = PlayerKey { tab_id, frame_id };
            let existing = {
                let guard = players.lock().await;
                guard.get(&key).cloned()
            };
            match existing {
                Some(handle) => {
                    update_existing(&handle, track).await?;
                }
                None => {
                    let handle = create_player(key, track, cmd_tx.clone()).await?;
                    let mut guard = players.lock().await;
                    guard.insert(key, handle);
                }
            }
        }
        InMessage::Remove { tab_id, frame_id } => {
            let key = PlayerKey { tab_id, frame_id };
            let mut guard = players.lock().await;
            if let Some(handle) = guard.remove(&key) {
                drop(handle);
            }
        }
    }
    Ok(())
}

async fn create_player(
    key: PlayerKey,
    track: TrackInfo,
    cmd_tx: mpsc::UnboundedSender<OutMessage>,
) -> Result<Arc<PlayerHandle>> {
    // Top-level frames keep the clean ...instance<pid>_t<tab> name; subframes
    // append _f<frame> so an embedded player gets its own bus name instead of
    // clobbering the top document's. unsigned_abs keeps the name valid for the
    // (shouldn't-happen) negative id case.
    let mut bus_name = format!(
        "org.mpris.MediaPlayer2.{}.instance{}_t{}",
        bus_segment(),
        process::id(),
        key.tab_id.unsigned_abs()
    );
    if key.frame_id != 0 {
        bus_name.push_str(&format!("_f{}", key.frame_id.unsigned_abs()));
    }

    let state = Arc::new(StdMutex::new(PlayerState::new(track)));

    let identity = make_identity(&lock_state(&state).track);

    let player_iface = mpris_player::PlayerIface {
        state: state.clone(),
        cmd_tx: cmd_tx.clone(),
        tab_id: key.tab_id,
        frame_id: key.frame_id,
    };
    let root_iface = mpris_root::RootIface {
        identity,
        desktop_entry: desktop_entry().to_string(),
        cmd_tx,
        tab_id: key.tab_id,
        frame_id: key.frame_id,
    };

    // One zbus connection per player. With zbus's `tokio` feature this does
    // NOT spawn an OS thread per connection (the internal executor is a no-op;
    // tasks run on the shared tokio runtime), so the per-player model is cheap.
    // Its payoff is teardown: dropping the connection releases the bus name and
    // clients see NameOwnerChanged — don't "optimize" this into one shared
    // connection serving many object paths, which makes clean removal hard.
    let conn = zbus::connection::Builder::session()
        .context("session bus")?
        .name(bus_name.as_str())
        .context("request name")?
        .serve_at(PLAYER_OBJECT_PATH, root_iface)
        .context("serve root iface")?
        .serve_at(PLAYER_OBJECT_PATH, player_iface)
        .context("serve player iface")?
        .build()
        .await
        .context("build connection")?;

    Ok(Arc::new(PlayerHandle {
        state,
        _connection: conn,
    }))
}

async fn update_existing(handle: &Arc<PlayerHandle>, track: TrackInfo) -> Result<()> {
    let UpdateOutcome { changed, position } = {
        let mut state = lock_state(&handle.state);
        state.apply_update(track)
    };

    // Nothing to announce — steady-state playback. Position is read on demand
    // by clients, never pushed, so this is the common no-traffic path.
    if !changed.any() && matches!(position, PositionDelta::Continuous) {
        return Ok(());
    }

    let conn = &handle._connection;
    let path: zbus::zvariant::ObjectPath<'static> =
        zbus::zvariant::ObjectPath::try_from(PLAYER_OBJECT_PATH).context("path")?;

    let iface_ref = conn
        .object_server()
        .interface::<_, mpris_player::PlayerIface>(path)
        .await
        .context("get player iface")?;

    {
        let iface = iface_ref.get().await;
        let emitter = iface_ref.signal_emitter();
        // Emit ONLY the properties that actually changed. Steady playback
        // emits nothing here.
        if changed.metadata {
            iface.metadata_changed(emitter).await.ok();
        }
        if changed.playback_status {
            iface.playback_status_changed(emitter).await.ok();
        }
        if changed.volume {
            iface.volume_changed(emitter).await.ok();
        }
        if changed.rate {
            iface.rate_changed(emitter).await.ok();
        }
        if changed.loop_status {
            iface.loop_status_changed(emitter).await.ok();
        }
        if changed.can_seek {
            iface.can_seek_changed(emitter).await.ok();
        }
        if changed.can_go_next {
            iface.can_go_next_changed(emitter).await.ok();
        }
        if changed.can_go_previous {
            iface.can_go_previous_changed(emitter).await.ok();
        }
        if changed.can_play_pause {
            iface.can_play_changed(emitter).await.ok();
            iface.can_pause_changed(emitter).await.ok();
        }
    }

    if let PositionDelta::Seeked(pos_us) = position {
        let emitter = iface_ref.signal_emitter().to_owned();
        mpris_player::PlayerIface::seeked(&emitter, pos_us).await.ok();
    }

    Ok(())
}

fn make_identity(track: &TrackInfo) -> String {
    // Try to pick a hostname from the page URL so users can tell Spotify-web
    // from YouTube etc. Falls back to the detected browser's display name.
    let app = app_identity();
    if let Some(host) = url_host(&track.page_url) {
        format!("{app} ({host})")
    } else {
        app.to_string()
    }
}

fn url_host(url: &str) -> Option<String> {
    // Lightweight extractor — avoids pulling in a URL crate just for this.
    let after_scheme = url.split_once("://")?.1;
    let host = after_scheme.split(['/', '?', '#']).next()?;
    let host = host.split_once('@').map(|(_, h)| h).unwrap_or(host);
    let host = host.split_once(':').map(|(h, _)| h).unwrap_or(host);
    let host = host.strip_prefix("www.").unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}
