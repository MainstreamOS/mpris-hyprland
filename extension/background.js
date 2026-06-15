/*
 * MPRIS for Hyprland — background script (MV3 event page).
 *
 * Holds one native-messaging port to the host. Consolidates every media
 * frame's state into ONE MPRIS player per browser WINDOW (keyed by windowId),
 * reflecting that window's active media — the playing frame, else the most
 * recently updated. Forwards commands from the host to whichever (tab, frame)
 * currently represents the window — except Raise, which it handles itself by
 * focusing that tab/window.
 *
 * The open native port keeps this event page resident while a host connection
 * lives (FF104+ resets the idle timer for active ports), so the lifecycle
 * matches the old persistent page while still allowing suspend when fully idle.
 */

"use strict";

const HOST_NAME = "io.github.mainstreamos.firefox_mpris_hyprland";
const VERSION = "0.2.0";

// DEBUG default OFF (production). Toggle from the extension's Inspect console:
//   mprisDebug(true) / mprisDebug(false)   — persisted + broadcast live to all
//   content scripts via storage.local.debug, so one switch flips every layer.
let DEBUG = false;
browser.storage.local.get({ debug: false }).then((r) => { DEBUG = !!r.debug; }).catch(() => {});

globalThis.mprisDebug = (on) => {
  DEBUG = !!on;
  browser.storage.local.set({ debug: DEBUG }).catch(() => {});
  // Push the new flag to every already-loaded content script (no reload).
  browser.tabs.query({}).then((tabs) => {
    for (const t of tabs) {
      if (typeof t.id === "number") {
        browser.tabs.sendMessage(t.id, { kind: "debug-changed", debug: DEBUG }).catch(() => {});
      }
    }
  }).catch(() => {});
  info(`DEBUG=${DEBUG}`);
  return DEBUG;
};

function ts() { return new Date().toISOString().slice(11, 23); }
function info(...a) { console.log(`[mpris-bg ${ts()}]`, ...a); }
function warn(...a) { console.warn(`[mpris-bg ${ts()}]`, ...a); }
function dbg(...a) { if (DEBUG) console.debug(`[mpris-bg ${ts()}]`, ...a); }

/** @type {browser.runtime.Port | null} */
let hostPort = null;
let reconnectDelayMs = 500;
let connectionGen = 0;
let msgsOut = 0, msgsIn = 0;

// Per-WINDOW players. Each browser window gets exactly one MPRIS player,
// reflecting that window's active media: the playing frame, else the most
// recently updated one. We track every media frame so we can pick and switch
// the representative, but only one update per window reaches the host (keyed by
// windowId — the host names the bus ...instance<pid>_t<windowId>). Commands
// route back to whichever (tab, frame) is currently the representative.
//
// In-memory only: if the event page suspends and wakes, `frames` starts empty
// and recovery falls to the resyncAllTabs() backstop (content scripts aren't
// suspended and re-send on __resync). Don't hang suspend-surviving logic on it.
const frames = new Map();        // "tabId:frameId" -> {tabId, frameId, windowId, track, seq}
const windowState = new Map();   // windowId -> {repKey, sig, tabId, frameId}  (last sent)
let seqCounter = 0;
const keyOf = (tabId, frameId) => `${tabId}:${frameId}`;

// The frame that represents a window's player: a playing frame wins; among
// equals, the most recently updated (highest seq).
function pickRep(windowId) {
  let best = null;
  for (const f of frames.values()) {
    if (f.windowId !== windowId) continue;
    if (!best) { best = f; continue; }
    const bp = !!best.track.playing, fp = !!f.track.playing;
    if (fp !== bp) { if (fp) best = f; continue; }
    if (f.seq > best.seq) best = f;
  }
  return best;
}

// Push a window's representative to the host, or remove the window player when
// the window has no media left. Deduped against the last sent state so a
// background frame's update doesn't re-send an unchanged player.
function syncWindow(windowId) {
  const rep = pickRep(windowId);
  const prev = windowState.get(windowId);
  if (!rep) {
    if (prev) {
      windowState.delete(windowId);
      info(`- window ${windowId} (no media; ${windowState.size} window player(s))`);
      send({ type: "remove", tabId: windowId, frameId: 0 });
    }
    return;
  }
  const repKey = keyOf(rep.tabId, rep.frameId);
  const sig = JSON.stringify(rep.track);
  if (prev && prev.repKey === repKey && prev.sig === sig) return; // unchanged
  if (!prev) info(`+ window ${windowId} player (${windowState.size + 1} total)`);
  windowState.set(windowId, { repKey, sig, tabId: rep.tabId, frameId: rep.frameId });
  send({ type: "update", tabId: windowId, frameId: 0, ...rep.track });
}

function syncAllWindows() {
  const ids = new Set([...frames.values()].map(f => f.windowId));
  for (const id of ids) syncWindow(id);
}

function ensureConnected() {
  if (hostPort) return;
  connectHost();
}

function connectHost() {
  connectionGen += 1;
  const gen = connectionGen;
  try {
    dbg(`gen=${gen} connecting to ${HOST_NAME}`);
    hostPort = browser.runtime.connectNative(HOST_NAME);
  } catch (e) {
    warn(`gen=${gen} connectNative threw:`, e);
    scheduleReconnect();
    return;
  }

  hostPort.onMessage.addListener(handleHostMessage);
  hostPort.onDisconnect.addListener(() => {
    const err = browser.runtime.lastError || hostPort?.error;
    info(`gen=${gen} host disconnected after ${msgsOut} out / ${msgsIn} in:`, err || "(clean)");
    hostPort = null;
    scheduleReconnect();
  });

  reconnectDelayMs = 500;
  msgsOut = 0; msgsIn = 0;
  send({ type: "hello", version: VERSION });

  // Re-push each window's representative to the fresh host (empty after a
  // respawn), then broadcast a resync as a backstop for anything missed.
  windowState.clear();
  syncAllWindows();
  info(`gen=${gen} connected; ${windowState.size} window player(s) replayed`);
  resyncAllTabs();
}

function resyncAllTabs() {
  browser.tabs.query({}).then((tabs) => {
    for (const t of tabs) {
      if (typeof t.id === "number") {
        browser.tabs.sendMessage(t.id, { kind: "mpris-resync" }).catch(() => {});
      }
    }
  }).catch((e) => dbg("tabs.query failed:", e));
}

function scheduleReconnect() {
  const delay = reconnectDelayMs;
  reconnectDelayMs = Math.min(reconnectDelayMs * 2, 5_000); // host is local — recover fast
  dbg(`reconnect in ${delay}ms (next ${reconnectDelayMs}ms)`);
  setTimeout(ensureConnected, delay);
}

function send(msg) {
  if (!hostPort) { dbg(`send: no port, dropping ${msg.type}`); return false; }
  try {
    hostPort.postMessage(msg);
    msgsOut += 1;
    if (msg.type === "update") {
      dbg(`[ext→host] update window=${msg.tabId} title=${JSON.stringify(msg.title || "")} playing=${msg.playing}`);
    } else {
      dbg(`[ext→host] ${msg.type}`, msg);
    }
    return true;
  } catch (e) { warn("postMessage failed:", e); return false; }
}

// Commands from the host: {type:"command", tabId:<windowId>, action, value?}.
// The id is a windowId; route to that window's current representative frame.
function handleHostMessage(msg) {
  msgsIn += 1;
  if (!msg || typeof msg !== "object" || msg.type !== "command") {
    dbg(`[host→ext] ignoring:`, msg); return;
  }
  const windowId = msg.tabId, action = msg.action, value = msg.value;
  if (typeof windowId !== "number") { warn("command without window id:", msg); return; }
  const ws = windowState.get(windowId);
  if (!ws) { dbg(`[host→ext] command for window ${windowId} with no representative`); return; }
  dbg(`[host→ext] window=${windowId} → tab=${ws.tabId} frame=${ws.frameId} ${action}`);

  // Raise is ours to handle — focus the representative's tab + window.
  if (action === "raise") {
    browser.tabs.update(ws.tabId, { active: true }).then((tab) => {
      if (tab && typeof tab.windowId === "number") {
        browser.windows.update(tab.windowId, { focused: true }).catch(() => {});
      }
    }).catch((e) => dbg(`raise tab ${ws.tabId} failed:`, e && e.message));
    return;
  }

  browser.tabs.sendMessage(ws.tabId, { kind: "mpris-command", action, value }, { frameId: ws.frameId })
    .catch((e) => dbg(`cmd ${action} → tab ${ws.tabId} frame ${ws.frameId} failed:`, e && e.message));
}

// Updates/removes from content scripts. Tracked per frame; consolidated to one
// player per window via syncWindow.
browser.runtime.onMessage.addListener((msg, sender) => {
  if (!sender.tab || typeof sender.tab.id !== "number") return;
  if (!msg || typeof msg !== "object") return;
  const tabId = sender.tab.id;
  const frameId = typeof sender.frameId === "number" ? sender.frameId : 0;
  const windowId = typeof sender.tab.windowId === "number" ? sender.tab.windowId : tabId;
  const k = keyOf(tabId, frameId);

  if (msg.kind === "update") {
    frames.set(k, { tabId, frameId, windowId, track: msg.track, seq: ++seqCounter });
    syncWindow(windowId);
  } else if (msg.kind === "remove") {
    const f = frames.get(k);
    if (f) { frames.delete(k); syncWindow(f.windowId); }
  }
});

// Tab closed → drop its frames, then re-sync any window it affected (which
// removes the window player if that was its last media).
browser.tabs.onRemoved.addListener((tabId) => {
  const affected = new Set();
  for (const [k, f] of frames) {
    if (f.tabId === tabId) { affected.add(f.windowId); frames.delete(k); }
  }
  for (const w of affected) syncWindow(w);
});

// Event-page lifecycle: (re)connect on browser start, install/update, and on
// each top-level load (wake). ensureConnected() is idempotent.
browser.runtime.onStartup.addListener(ensureConnected);
browser.runtime.onInstalled.addListener(ensureConnected);
info("background loaded — connecting to native host");
ensureConnected();
