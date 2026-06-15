/*
 * MPRIS for Hyprland — MAIN-world helper (Chromium only).
 *
 * Chromium content scripts run in an isolated world and cannot see the page's
 * navigator.mediaSession, MediaSession.prototype, or the action handlers the
 * page registers — there is no Xray (wrappedJSObject / exportFunction) like on
 * Firefox. This script is declared with "world":"MAIN" so it runs IN the page
 * realm, where it can read and patch Media Session directly, then relays the
 * data to the isolated content script (content.js) over window.postMessage.
 *
 * It is NOT loaded on Firefox — the Firefox manifest only injects content.js,
 * which reaches the page realm through Xray instead. The <video>/<audio>
 * element observation (and per-tab volume) all live in the isolated content.js
 * on both browsers; only Media Session metadata/handlers need this MAIN bridge.
 *
 * Messages OUT (to the isolated world): {source:"mpris-main", type:"state",
 *   meta, handlers, positionState}. Messages IN (from the isolated world):
 *   {source:"mpris-iso", type:"invoke"|"invoke-seek"|"request-state", ...}.
 */

"use strict";

(() => {
  if (window.__mprisMainHooked) return;
  window.__mprisMainHooked = true;

  const ms = navigator.mediaSession;
  if (!ms) return; // page realm has no Media Session API

  // Action handlers the page registered, as page-realm callable references.
  const handlers = Object.create(null);
  // Last setPositionState the page reported (element-less players).
  let positionState = null;

  const ACTIONS = ["play", "pause", "playpause", "stop", "nexttrack",
    "previoustrack", "seekto", "seekbackward", "seekforward"];

  function snapshotMeta() {
    try {
      const md = ms.metadata;
      if (!md) return null;
      const artwork = [];
      try {
        const aw = md.artwork;
        if (aw && aw.length) {
          for (let i = 0; i < aw.length; i++) {
            artwork.push({ src: String(aw[i].src || ""), sizes: String(aw[i].sizes || "") });
          }
        }
      } catch (_) {}
      return {
        title: String(md.title || ""),
        artist: String(md.artist || ""),
        album: String(md.album || ""),
        artwork,
        playbackState: String(ms.playbackState || "none"),
      };
    } catch (_) { return null; }
  }

  function postState() {
    const present = Object.create(null);
    for (const a of ACTIONS) present[a] = !!handlers[a];
    try {
      window.postMessage({
        source: "mpris-main",
        type: "state",
        meta: snapshotMeta(),
        handlers: present,
        positionState,
      }, "*");
    } catch (_) {}
  }

  let postTimer = null;
  function scheduleState() {
    if (postTimer) return;
    postTimer = setTimeout(() => { postTimer = null; postState(); }, 50);
  }

  // Patch setActionHandler — record which actions the page registers (drives
  // CanGoNext/CanGoPrevious and lets the isolated world invoke Next/Previous),
  // then still call through so the page keeps working.
  try {
    const orig = ms.setActionHandler.bind(ms);
    ms.setActionHandler = function (action, handler) {
      try {
        if (handler) handlers[action] = handler;
        else delete handlers[action];
      } catch (_) {}
      scheduleState();
      return orig(action, handler);
    };
  } catch (_) {}

  // metadata setter → notify on assignment (catches paused-tab track changes).
  try {
    const proto = window.MediaSession && window.MediaSession.prototype;
    const d = proto && Object.getOwnPropertyDescriptor(proto, "metadata");
    if (d && d.get && d.set) {
      Object.defineProperty(proto, "metadata", {
        configurable: true,
        get: d.get,
        set: function (v) { d.set.call(this, v); scheduleState(); },
      });
    }
  } catch (_) {}

  // playbackState setter → notify.
  try {
    const proto = window.MediaSession && window.MediaSession.prototype;
    const d = proto && Object.getOwnPropertyDescriptor(proto, "playbackState");
    if (d && d.get && d.set) {
      Object.defineProperty(proto, "playbackState", {
        configurable: true,
        get: d.get,
        set: function (v) { d.set.call(this, v); scheduleState(); },
      });
    }
  } catch (_) {}

  // setPositionState → capture authoritative duration/position/rate.
  try {
    const orig = ms.setPositionState && ms.setPositionState.bind(ms);
    if (orig) {
      ms.setPositionState = function (st) {
        try {
          positionState = st ? {
            duration: Number(st.duration) || 0,
            position: Number(st.position) || 0,
            rate: Number(st.playbackRate) || 1,
          } : null;
        } catch (_) {}
        scheduleState();
        return orig(st);
      };
    }
  } catch (_) {}

  // Commands from the isolated world: invoke a Media Session action handler in
  // the page realm (no cloneInto needed — we ARE the page realm here).
  window.addEventListener("message", (ev) => {
    if (ev.source !== window) return;
    const d = ev.data;
    if (!d || d.source !== "mpris-iso") return;
    if (d.type === "invoke") {
      const h = handlers[d.name];
      if (h) { try { h(); } catch (_) {} }
    } else if (d.type === "invoke-seek") {
      const h = handlers[d.name];
      if (h) { try { h(d.detail || {}); } catch (_) {} }
    } else if (d.type === "request-state") {
      postState();
    }
  });

  // Initial push (the isolated side also requests once, covering injection-order
  // races between the MAIN and isolated content scripts).
  scheduleState();
})();
