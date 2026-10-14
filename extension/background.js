/*
 * MPRIS for Hyprland — background script (MV3 event page on Firefox, service
 * worker on Chromium — same code via the browser/chrome shim below).
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

// Cross-browser namespace: Firefox exposes `browser` (event page), Chromium
// `chrome` (service worker). The promise-based runtime / tabs / windows /
// storage calls used below work on both under MV3; connectNative keeps the
// service worker alive while the port is open (and the reconnect + resync
// backstop covers the harsher Chromium SW idle teardown).
const browser = globalThis.browser ?? chrome;

const HOST_NAME = "io.github.mainstreamos.firefox_mpris_hyprland";
const VERSION = browser.runtime.getManifest().version;

/** @type {browser.runtime.Port | null} */
let hostPort = null;
let reconnectDelayMs = 500;
let reconnectTimer = null;

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
const detachedTabs = new Set();
const cleanString = (value, limit = 4096) => typeof value === "string" ? value.slice(0, limit) : "";
const boundedNumber = (value, fallback, min, max) => typeof value === "number" && Number.isFinite(value) ? Math.max(min, Math.min(max, value)) : fallback;

// Validate at the native-host boundary even if a page bypasses the content
// script's artwork guard. The complete normalized payload stays below 1 MiB.
function normalizeTrack(t) {
  if (!t || typeof t !== "object") return null;
  let artUrl = cleanString(t.artUrl, 256 * 1024 + 1);
  if (!/^(https?:|data:image\/)/i.test(artUrl) || artUrl.length > (/^data:/i.test(artUrl) ? 256 * 1024 : 8192)) artUrl = "";
  return {
    title: cleanString(t.title), artist: cleanString(t.artist), album: cleanString(t.album),
    pageUrl: cleanString(t.pageUrl, 8192), sourceId: cleanString(t.sourceId, 8192), artUrl,
    duration: boundedNumber(t.duration, 0, 0, 1e10), position: boundedNumber(t.position, 0, 0, 1e10),
    volume: boundedNumber(t.volume, 1, 0, 1), rate: boundedNumber(t.rate, 1, 0.25, 4),
    playing: t.playing === true, seeked: t.seeked === true, looping: t.looping === true,
    canSetVolume: t.canSetVolume === true, canSetRate: t.canSetRate === true, canSetLoop: t.canSetLoop === true,
    canSeek: t.canSeek === true, canGoNext: t.canGoNext === true, canGoPrevious: t.canGoPrevious === true,
  };
}
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
      send({ type: "remove", tabId: windowId, frameId: 0 });
    }
    return;
  }
  const repKey = keyOf(rep.tabId, rep.frameId);
  const sig = JSON.stringify(rep.track);
  if (prev && prev.repKey === repKey && prev.sig === sig) return; // unchanged
  windowState.set(windowId, { repKey, sig, tabId: rep.tabId, frameId: rep.frameId });
  send({ type: "update", tabId: windowId, frameId: 0, ...rep.track, sourceId: `${repKey}:${rep.track.sourceId}` });
  // Seeked is an event, not persistent state to replay from an older frame.
  rep.track.seeked = false;
  windowState.get(windowId).sig = JSON.stringify(rep.track);
}

function syncAllWindows() {
  const ids = new Set([...frames.values()].map(f => f.windowId).filter(id => id >= 0));
  for (const id of ids) syncWindow(id);
}

function ensureConnected() {
  if (hostPort) return;
  connectHost();
}

function connectHost() {
  try {
    hostPort = browser.runtime.connectNative(HOST_NAME);
  } catch {
    scheduleReconnect();
    return;
  }

  const port = hostPort, connectedAt = Date.now();
  port.onMessage.addListener(handleHostMessage);
  port.onDisconnect.addListener(() => {
    // Reading lastError also prevents Chromium's unchecked-lastError warning.
    void browser.runtime.lastError;
    if (hostPort !== port) return;
    hostPort = null;
    if (Date.now() - connectedAt >= 10_000) reconnectDelayMs = 500;
    scheduleReconnect();
  });
  send({ type: "hello", version: VERSION });

  // Re-push each window's representative to the fresh host (empty after a
  // respawn), then broadcast a resync as a backstop for anything missed.
  windowState.clear();
  syncAllWindows();
  resyncAllTabs();
}

function resyncAllTabs() {
  browser.tabs.query({}).then((tabs) => {
    for (const t of tabs) {
      if (typeof t.id === "number") {
        browser.tabs.sendMessage(t.id, { kind: "mpris-resync" }).catch(() => {});
      }
    }
  }).catch(() => {});
}

function scheduleReconnect() {
  if (reconnectTimer !== null) return;
  const delay = reconnectDelayMs;
  reconnectDelayMs = Math.min(reconnectDelayMs * 2, 5_000); // host is local — recover fast
  reconnectTimer = setTimeout(() => { reconnectTimer = null; ensureConnected(); }, delay);
}

function send(msg) {
  if (!hostPort) { return false; }
  try {
    hostPort.postMessage(msg);
    return true;
  } catch { return false; }
}

// Commands from the host: {type:"command", tabId:<windowId>, action, value?}.
// The id is a windowId; route to that window's current representative frame.
function handleHostMessage(msg) {
  if (!msg || typeof msg !== "object" || msg.type !== "command") {
    return;
  }
  const windowId = msg.tabId, action = msg.action, value = msg.value;
  if (typeof windowId !== "number") { return; }
  const ws = windowState.get(windowId);
  if (!ws) { return; }

  // Raise is ours to handle — focus the representative's tab + window.
  if (action === "raise") {
    browser.tabs.update(ws.tabId, { active: true }).then((tab) => {
      if (tab && typeof tab.windowId === "number") {
        browser.windows.update(tab.windowId, { focused: true }).catch(() => {});
      }
    }).catch(() => {});
    return;
  }

  browser.tabs.sendMessage(ws.tabId, { kind: "mpris-command", action, value }, { frameId: ws.frameId })
    .catch(() => {});
}

// Updates/removes from content scripts. Tracked per frame; consolidated to one
// player per window via syncWindow.
browser.runtime.onMessage.addListener((msg, sender) => {
  if (!sender.tab || !Number.isInteger(sender.tab.id) || sender.tab.id < 0) return;
  if (!msg || typeof msg !== "object") return;
  const tabId = sender.tab.id;
  const frameId = typeof sender.frameId === "number" ? sender.frameId : 0;
  const windowId = typeof sender.tab.windowId === "number" ? sender.tab.windowId : tabId;
  const k = keyOf(tabId, frameId);

  const documentId = sender.documentId || msg.documentId || "";
  if (msg.kind === "update") {
    if (detachedTabs.has(tabId)) return;
    const track = normalizeTrack(msg.track);
    if (!track) return;
    const old = frames.get(k);
    frames.set(k, { tabId, frameId, windowId, documentId, track, seq: ++seqCounter });
    if (old && old.windowId !== windowId) syncWindow(old.windowId);
    syncWindow(windowId);
  } else if (msg.kind === "remove") {
    const f = frames.get(k);
    if (f && (!f.documentId || !documentId || f.documentId === documentId)) {
      frames.delete(k); syncWindow(f.windowId);
    }
  }
});

function dropTab(tabId) {
  const affected = new Set();
  for (const [k, f] of frames) {
    if (f.tabId === tabId) { affected.add(f.windowId); frames.delete(k); }
  }
  detachedTabs.delete(tabId);
  for (const w of affected) syncWindow(w);
}

browser.tabs.onRemoved.addListener(dropTab);
browser.tabs.onDetached.addListener((tabId, info) => {
  detachedTabs.add(tabId);
  const affected = new Set([info.oldWindowId]);
  for (const f of frames.values()) if (f.tabId === tabId) {
    if (f.windowId >= 0) affected.add(f.windowId);
    f.windowId = -1;
  }
  for (const id of affected) syncWindow(id);
});
browser.tabs.onAttached.addListener((tabId, info) => {
  detachedTabs.delete(tabId);
  const affected = new Set([info.newWindowId]);
  for (const f of frames.values()) if (f.tabId === tabId) {
    if (f.windowId >= 0) affected.add(f.windowId);
    f.windowId = info.newWindowId;
  }
  for (const id of affected) syncWindow(id);
  browser.tabs.sendMessage(tabId, { kind: "mpris-resync" }).catch(() => {});
});
browser.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (changeInfo.discarded === true || changeInfo.status === "loading") dropTab(tabId);
  if (changeInfo.status === "complete") browser.tabs.sendMessage(tabId, { kind: "mpris-resync" }).catch(() => {});
});
browser.tabs.onReplaced.addListener((addedTabId, removedTabId) => {
  dropTab(removedTabId);
  browser.tabs.sendMessage(addedTabId, { kind: "mpris-resync" }).catch(() => {});
});

browser.windows.onRemoved.addListener(windowId => {
  for (const [key, frame] of frames) if (frame.windowId === windowId) frames.delete(key);
  syncWindow(windowId);
});

// Event-page lifecycle: (re)connect on browser start, install/update, and on
// each top-level load (wake). ensureConnected() is idempotent.
browser.runtime.onStartup.addListener(ensureConnected);
browser.runtime.onInstalled.addListener(ensureConnected);
ensureConnected();
