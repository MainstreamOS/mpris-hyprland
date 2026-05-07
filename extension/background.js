/*
 * MPRIS for Hyprland — background script.
 *
 * Maintains a single connection to the native messaging host and multiplexes
 * messages from every tab's content script to the host. Forwards commands
 * (Play/Pause/Next/SetPosition/...) coming back from the host to the tab
 * that owns the corresponding MPRIS player.
 */

"use strict";

const HOST_NAME = "io.github.mainstreamos.firefox_mpris_hyprland";

/** @type {browser.runtime.Port | null} */
let hostPort = null;
/** Tabs we've reported as having active media. */
const activeTabs = new Set();
/** Reconnect backoff in ms. */
let reconnectDelayMs = 500;

function log(...args) {
  // Visible in about:debugging → Inspect.
  console.log("[mpris-host]", ...args);
}

function connectHost() {
  try {
    log("connecting to native host:", HOST_NAME);
    hostPort = browser.runtime.connectNative(HOST_NAME);
  } catch (e) {
    log("connectNative threw:", e);
    scheduleReconnect();
    return;
  }

  hostPort.onMessage.addListener(handleHostMessage);
  hostPort.onDisconnect.addListener(() => {
    const err = browser.runtime.lastError || hostPort?.error;
    log("host disconnected:", err);
    hostPort = null;
    activeTabs.clear();
    scheduleReconnect();
  });

  // Reset backoff on successful connect (we treat first message as success
  // too; here we optimistically reset).
  reconnectDelayMs = 500;

  send({ type: "hello", version: "0.1.0" });
}

function scheduleReconnect() {
  const delay = reconnectDelayMs;
  reconnectDelayMs = Math.min(reconnectDelayMs * 2, 30_000);
  setTimeout(connectHost, delay);
}

function send(msg) {
  if (!hostPort) return false;
  try {
    hostPort.postMessage(msg);
    return true;
  } catch (e) {
    log("postMessage failed:", e);
    return false;
  }
}

/**
 * Messages the host sends back: {type:"command", tabId, action, value?}
 */
function handleHostMessage(msg) {
  if (!msg || typeof msg !== "object") return;
  if (msg.type === "command") {
    const { tabId, action, value } = msg;
    if (typeof tabId !== "number") return;
    browser.tabs
      .sendMessage(tabId, { kind: "mpris-command", action, value })
      .catch((e) => log(`tab ${tabId} command failed:`, e?.message || e));
  }
}

/**
 * Messages the content script sends:
 *   {kind:"update", track:{...}}
 *   {kind:"remove"}
 */
browser.runtime.onMessage.addListener((msg, sender) => {
  if (!sender.tab || typeof sender.tab.id !== "number") return;
  const tabId = sender.tab.id;

  if (!msg || typeof msg !== "object") return;

  if (msg.kind === "update") {
    activeTabs.add(tabId);
    send({ type: "update", tabId, ...msg.track });
  } else if (msg.kind === "remove") {
    if (activeTabs.delete(tabId)) {
      send({ type: "remove", tabId });
    }
  }
});

browser.tabs.onRemoved.addListener((tabId) => {
  if (activeTabs.delete(tabId)) {
    send({ type: "remove", tabId });
  }
});

connectHost();
