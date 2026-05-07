//! firefox-mpris-host
//!
//! Native messaging host for the firefox-mpris-hyprland WebExtension.
//! Reads length-prefixed JSON on stdin (per Mozilla's native messaging
//! protocol), forwards each tab's media state onto the session D-Bus as a
//! distinct MPRIS player, and forwards method calls (Play/Pause/Next/Seek/...)
//! from D-Bus back to the extension on stdout so the page can act on them.

mod messaging;
mod mpris_player;
mod mpris_root;
mod protocol;
mod state;

use anyhow::{Context, Result};
use protocol::{InMessage, OutMessage, TabId, TrackInfo};
use state::{PlayerHandle, PlayerState, PositionDelta};
use std::collections::HashMap;
use std::process;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::io::BufWriter;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

const APP_IDENTITY: &str = "Firefox";
const PLAYER_OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";

type PlayerMap = Arc<AsyncMutex<HashMap<TabId, Arc<PlayerHandle>>>>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    // Log to STDERR. STDOUT is the native messaging channel and must remain
    // pristine length-prefixed JSON.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("firefox_mpris_host=info"),
    )
    .target(env_logger::Target::Stderr)
    .init();

    log::info!("firefox-mpris-host starting (pid {})", process::id());

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
                Err(e) => {
                    log::warn!("serialize outbound: {e}");
                    continue;
                }
            };
            if let Err(e) = messaging::write_message(&mut out, &bytes).await {
                log::error!("write stdout: {e}");
                break;
            }
        }
        log::debug!("writer task exiting");
    });

    let mut stdin = tokio::io::stdin();
    loop {
        let payload = match messaging::read_message(&mut stdin).await {
            Ok(Some(p)) => p,
            Ok(None) => {
                log::info!("stdin EOF — Firefox has gone away, shutting down");
                break;
            }
            Err(e) => {
                log::error!("read stdin: {e}");
                break;
            }
        };

        let msg: InMessage = match serde_json::from_slice(&payload) {
            Ok(m) => m,
            Err(e) => {
                log::warn!(
                    "invalid JSON ({} bytes): {e} — payload: {}",
                    payload.len(),
                    String::from_utf8_lossy(&payload)
                );
                continue;
            }
        };

        if let Err(e) = handle_message(msg, &players, &cmd_tx).await {
            log::error!("handle_message: {e:#}");
        }
    }

    // Clean up: drop all player connections, then close the writer.
    {
        let mut guard = players.lock().await;
        guard.clear();
    }
    drop(cmd_tx);
    let _ = writer_task.await;
    log::info!("firefox-mpris-host exited cleanly");
    Ok(())
}

async fn handle_message(
    msg: InMessage,
    players: &PlayerMap,
    cmd_tx: &mpsc::UnboundedSender<OutMessage>,
) -> Result<()> {
    match msg {
        InMessage::Hello { version } => {
            log::info!("extension connected, version={version:?}");
        }
        InMessage::Ping => {}
        InMessage::Update { tab_id, track } => {
            let existing = {
                let guard = players.lock().await;
                guard.get(&tab_id).cloned()
            };
            match existing {
                Some(handle) => update_existing(&handle, track).await?,
                None => {
                    let handle = create_player(tab_id, track, cmd_tx.clone()).await?;
                    let mut guard = players.lock().await;
                    guard.insert(tab_id, handle);
                }
            }
        }
        InMessage::Remove { tab_id } => {
            let mut guard = players.lock().await;
            if let Some(handle) = guard.remove(&tab_id) {
                log::info!("removing player tab={}", tab_id);
                drop(handle);
            }
        }
    }
    Ok(())
}

async fn create_player(
    tab_id: TabId,
    track: TrackInfo,
    cmd_tx: mpsc::UnboundedSender<OutMessage>,
) -> Result<Arc<PlayerHandle>> {
    let bus_name = format!(
        "org.mpris.MediaPlayer2.firefox.instance{}_t{}",
        process::id(),
        // Use unsigned magnitude so negative tab ids (shouldn't happen, but
        // belt-and-braces) don't produce hyphens that would break the bus
        // name well-known-name validation.
        tab_id.unsigned_abs()
    );

    let state = Arc::new(StdMutex::new(PlayerState::new(track)));

    let identity = make_identity(&state.lock().expect("state poisoned").track);

    let player_iface = mpris_player::PlayerIface {
        state: state.clone(),
        cmd_tx,
        tab_id,
    };
    let root_iface = mpris_root::RootIface { identity };

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

    log::info!("created player tab={} bus={}", tab_id, bus_name);

    Ok(Arc::new(PlayerHandle {
        state,
        tab_id,
        _connection: conn,
    }))
}

async fn update_existing(handle: &Arc<PlayerHandle>, track: TrackInfo) -> Result<()> {
    let delta = {
        let mut state = handle.state.lock().expect("state poisoned");
        state.apply_update(track)
    };

    let conn = &handle._connection;
    let path: zbus::zvariant::ObjectPath<'static> =
        zbus::zvariant::ObjectPath::try_from(PLAYER_OBJECT_PATH)
            .context("path")?
            .into();

    let iface_ref = conn
        .object_server()
        .interface::<_, mpris_player::PlayerIface>(path)
        .await
        .context("get player iface")?;

    {
        let iface = iface_ref.get().await;
        let emitter = iface_ref.signal_emitter();
        // Emit changes for the properties that mutate per update. Position
        // is intentionally omitted (per MPRIS spec — clients poll).
        iface.metadata_changed(emitter).await.ok();
        iface.playback_status_changed(emitter).await.ok();
        iface.can_seek_changed(emitter).await.ok();
        iface.can_go_next_changed(emitter).await.ok();
        iface.can_go_previous_changed(emitter).await.ok();
        iface.volume_changed(emitter).await.ok();
    }

    if let PositionDelta::Seeked(pos_us) = delta {
        let emitter = iface_ref.signal_emitter().to_owned();
        mpris_player::PlayerIface::seeked(&emitter, pos_us)
            .await
            .ok();
    }

    Ok(())
}

fn make_identity(track: &TrackInfo) -> String {
    // Try to pick a hostname from the page URL so users can tell Spotify-web
    // from YouTube etc. Falls back to "Firefox".
    if let Some(host) = url_host(&track.page_url) {
        format!("{APP_IDENTITY} ({host})")
    } else {
        APP_IDENTITY.to_string()
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
