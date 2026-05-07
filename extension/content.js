/*
 * Content script. Bridges between the page-context script (inject.js)
 * and the background script.
 *
 * - Injects inject.js into the page realm at document_start.
 * - Forwards window.postMessage("mpris-fx", ...) updates upward.
 * - Receives runtime messages with MPRIS commands and forwards them
 *   downward via window.postMessage("mpris-fx-cmd", ...).
 */

"use strict";

const TAG_OUT = "mpris-fx";
const TAG_IN = "mpris-fx-cmd";

// Inject the page-context script. We add a <script src=...> rather than
// inlining the source so that strict CSP pages (e.g. github.com) still
// load it via the moz-extension:// URL.
try {
  const url = browser.runtime.getURL("inject.js");
  const s = document.createElement("script");
  s.src = url;
  s.async = false;
  // documentElement may exist before head/body at document_start.
  (document.head || document.documentElement).appendChild(s);
  s.onload = () => s.remove();
} catch (e) {
  console.warn("[mpris-fx content] inject failed:", e);
}

// Page → background.
window.addEventListener("message", (e) => {
  if (e.source !== window) return;
  const d = e.data;
  if (!d || d.tag !== TAG_OUT) return;
  if (d.kind === "update") {
    browser.runtime.sendMessage({ kind: "update", track: d.track }).catch(() => {});
  } else if (d.kind === "remove") {
    browser.runtime.sendMessage({ kind: "remove" }).catch(() => {});
  }
});

// Background → page.
browser.runtime.onMessage.addListener((msg) => {
  if (!msg || msg.kind !== "mpris-command") return;
  window.postMessage(
    { tag: TAG_IN, action: msg.action, value: msg.value },
    "*"
  );
});

// Tell the host to forget us when the page unloads (covers reloads /
// navigations away). Tab close is also handled by the background script's
// browser.tabs.onRemoved listener.
window.addEventListener("pagehide", () => {
  browser.runtime.sendMessage({ kind: "remove" }).catch(() => {});
});
