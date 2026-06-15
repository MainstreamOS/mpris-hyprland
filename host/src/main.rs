//! firefox-mpris-host
//!
//! Native messaging host for the firefox-mpris-hyprland WebExtension.
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
use std::io::Write;
use std::process;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use tokio::io::BufWriter;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

/// Browser .desktop basename, detected once at startup (see detect_desktop_entry).
static DESKTOP_ENTRY: OnceLock<String> = OnceLock::new();

const APP_IDENTITY: &str = "Firefox";
const PLAYER_OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";

/// Detect the browser's .desktop basename so MPRIS clients resolve the correct
/// icon. Order: $FIREFOX_MPRIS_DESKTOP_ENTRY override → parent process name
/// (the host is a child of the browser) → "firefox". On Zen the parent comm is
/// "zen-bin" and the desktop file is zen.desktop, so plain "firefox" would
/// leave clients with a generic icon.
fn detect_desktop_entry() -> String {
    if let Ok(v) = std::env::var("FIREFOX_MPRIS_DESKTOP_ENTRY") {
        if !v.is_empty() {
            return v;
        }
    }
    let parent_comm = parent_comm().unwrap_or_default().to_ascii_lowercase();
    for (needle, entry) in [
        ("zen", "zen"),
        ("librewolf", "librewolf"),
        ("floorp", "floorp"),
        ("waterfox", "waterfox"),
        ("mullvad", "mullvad-browser"),
        ("firefox", "firefox"),
    ] {
        if parent_comm.contains(needle) {
            return entry.to_string();
        }
    }
    "firefox".to_string()
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

/// Tee writer: forwards each log write to BOTH stderr (in case the browser
/// is forwarding it somewhere visible) AND a persistent log file (so we
/// have a reliable diagnostic trail even when the browser drops stderr,
/// which Firefox-family browsers do whenever an already-running instance
/// handles the launch).
struct TeeWriter {
    stderr: std::io::Stderr,
    file: std::fs::File,
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = self.stderr.write_all(buf);
        self.file.write_all(buf)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let _ = self.stderr.flush();
        self.file.flush()
    }
}

/// Resolve the log file path, in order of preference:
///   1. $FIREFOX_MPRIS_HOST_LOG (explicit override)
///   2. $XDG_STATE_HOME/firefox-mpris-host/host.log
///   3. $HOME/.local/state/firefox-mpris-host/host.log
///   4. /tmp/firefox-mpris-host.log
fn resolve_log_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("FIREFOX_MPRIS_HOST_LOG") {
        if !p.is_empty() {
            return p.into();
        }
    }
    let state_home = std::env::var("XDG_STATE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.local/state")));
    match state_home {
        Some(home) => format!("{home}/firefox-mpris-host/host.log").into(),
        None => "/tmp/firefox-mpris-host.log".into(),
    }
}

/// Set up env_logger writing to stderr-tee-file. Returns the resolved log
/// path so the banner can mention it.
fn setup_logging() -> Result<std::path::PathBuf> {
    let log_path = resolve_log_path();

    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Cap log size to prevent unbounded growth (10 MiB). On overflow we
    // rename the existing file to host.log.1 so the most recent run is
    // still recoverable, then truncate.
    if let Ok(meta) = std::fs::metadata(&log_path) {
        if meta.len() > 10 * 1024 * 1024 {
            let mut backup = log_path.clone();
            backup.set_extension("log.1");
            let _ = std::fs::rename(&log_path, &backup);
        }
    }

    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening log file {}", log_path.display()))?;

    let tee = TeeWriter {
        stderr: std::io::stderr(),
        file: log_file,
    };

    // Default to info: routine per-message frames are at debug/trace, so info
    // is a quiet-but-useful baseline (lifecycle, player create/remove, D-Bus
    // method calls). Crank with RUST_LOG=firefox_mpris_host=trace.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("firefox_mpris_host=info,warn"),
    )
    .format_timestamp_millis()
    .target(env_logger::Target::Pipe(Box::new(tee)))
    .init();

    Ok(log_path)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    // STDOUT is the native messaging channel and must remain pristine
    // length-prefixed JSON. We log via env_logger to a Tee that writes to
    // both stderr (in case the browser forwards it) and a persistent file at
    // ~/.local/state/firefox-mpris-host/host.log (always works).
    //
    // Override the log filter via RUST_LOG, e.g.:
    //   RUST_LOG=firefox_mpris_host=trace zen-browser
    //   RUST_LOG=trace zen-browser   (everything, including zbus)
    // Override the file path via FIREFOX_MPRIS_HOST_LOG.
    let log_path = setup_logging()?;
    let _ = DESKTOP_ENTRY.set(detect_desktop_entry());

    log::info!("================================================================");
    log::info!(
        "firefox-mpris-host v{} starting (pid {})",
        env!("CARGO_PKG_VERSION"),
        process::id()
    );
    log::info!("log file: {}", log_path.display());
    log::info!(
        "desktop entry: {} (override with FIREFOX_MPRIS_DESKTOP_ENTRY)",
        DESKTOP_ENTRY.get().map(String::as_str).unwrap_or("firefox")
    );
    log::info!(
        "RUST_LOG: {}",
        std::env::var("RUST_LOG").unwrap_or_else(|_| "(default = firefox_mpris_host=info,warn)".into())
    );
    log::info!("================================================================");

    let players: PlayerMap = Arc::new(AsyncMutex::new(HashMap::new()));

    // Outbound channel: any task pushes OutMessage; one writer task drains
    // them to stdout, framed.
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<OutMessage>();

    let writer_task = tokio::spawn(async move {
        let stdout = tokio::io::stdout();
        let mut out = BufWriter::new(stdout);
        let mut written: u64 = 0;
        while let Some(msg) = cmd_rx.recv().await {
            log::debug!("[host→ext] {msg:?}");
            let bytes = match serde_json::to_vec(&msg) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("[host→ext] serialize failed: {e}");
                    continue;
                }
            };
            log::trace!("[host→ext] {} bytes", bytes.len());
            if let Err(e) = messaging::write_message(&mut out, &bytes).await {
                log::error!("[host→ext] write stdout failed after {written} message(s): {e}");
                break;
            }
            written += 1;
        }
        log::info!("writer task exiting after {written} message(s)");
    });

    let mut stdin = tokio::io::stdin();
    let mut received: u64 = 0;
    loop {
        let payload = match messaging::read_message(&mut stdin).await {
            Ok(Some(p)) => p,
            Ok(None) => {
                log::info!(
                    "stdin EOF — browser has gone away, shutting down (received {received} message(s))"
                );
                break;
            }
            Err(e) => {
                log::error!("read stdin: {e}");
                break;
            }
        };
        received += 1;

        log::trace!("[ext→host] frame {received} ({} bytes)", payload.len());

        let msg: InMessage = match serde_json::from_slice(&payload) {
            Ok(m) => m,
            Err(e) => {
                log::warn!(
                    "[ext→host] invalid JSON ({} bytes): {e} — payload: {}",
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
            log::info!("[hello] extension connected, version={version:?}");
            let n = players.lock().await.len();
            log::info!("[hello] currently tracking {n} player(s)");
        }
        InMessage::Ping => {
            log::trace!("[ping]");
        }
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
            log::debug!(
                "[update] tab={tab_id} frame={frame_id} title={:?} artist={:?} dur={:.1}s pos={:.1}s playing={} rate={:.2} loop={} canSeek={} canNext={} canPrev={} art={}",
                track.title,
                track.artist,
                track.duration,
                track.position,
                track.playing,
                track.rate,
                track.looping,
                track.can_seek,
                track.can_go_next,
                track.can_go_previous,
                if track.art_url.is_empty() { "(none)" } else { "(present)" }
            );
            match existing {
                Some(handle) => {
                    update_existing(&handle, track).await?;
                }
                None => {
                    log::info!("[update] tab={tab_id} frame={frame_id} → no existing player, creating");
                    let handle = create_player(key, track, cmd_tx.clone()).await?;
                    let mut guard = players.lock().await;
                    guard.insert(key, handle);
                    log::debug!("[update] → player count now {}", guard.len());
                }
            }
        }
        InMessage::Remove { tab_id, frame_id } => {
            let key = PlayerKey { tab_id, frame_id };
            let mut guard = players.lock().await;
            if let Some(handle) = guard.remove(&key) {
                log::info!(
                    "[remove] tab={tab_id} frame={frame_id} (player count now {})",
                    guard.len()
                );
                drop(handle);
            } else {
                log::debug!("[remove] tab={tab_id} frame={frame_id} but no player tracked — ignoring");
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
        "org.mpris.MediaPlayer2.firefox.instance{}_t{}",
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
        desktop_entry: DESKTOP_ENTRY
            .get()
            .cloned()
            .unwrap_or_else(|| "firefox".to_string()),
        cmd_tx,
        tab_id: key.tab_id,
        frame_id: key.frame_id,
    };

    log::trace!("[create_player] requesting name {bus_name}");
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

    log::info!(
        "[create_player] tab={} frame={} bus={}",
        key.tab_id,
        key.frame_id,
        bus_name
    );

    Ok(Arc::new(PlayerHandle {
        state,
        tab_id: key.tab_id,
        frame_id: key.frame_id,
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
        log::trace!("[update_existing] tab={} frame={} → no signals", handle.tab_id, handle.frame_id);
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
        log::debug!(
            "[update_existing] tab={} frame={} → Seeked({:.2}s)",
            handle.tab_id,
            handle.frame_id,
            pos_us as f64 / 1_000_000.0
        );
        let emitter = iface_ref.signal_emitter().to_owned();
        mpris_player::PlayerIface::seeked(&emitter, pos_us).await.ok();
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
