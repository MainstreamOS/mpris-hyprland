/*
 * MPRIS for Hyprland — content script (isolated world).
 *
 * Runs at document_start in the isolated content-script world, in every frame.
 * Reads the page's Media Session and <video>/<audio> state and reports it to
 * the background script, which bridges it to the native host → D-Bus MPRIS.
 *
 * No <script> is injected into the page (the old inject.js approach broke on
 * strict-CSP sites like github.com / x.com that block the extension origin in
 * script-src). Reaching the page realm is browser-specific:
 *   - Firefox: Xray — window.wrappedJSObject for navigator.mediaSession, and
 *     exportFunction to wrap setActionHandler / the metadata setter so the page
 *     can still call them while we observe.
 *   - Chromium: a MAIN-world content script (content-main.js) does the same
 *     patching in the page realm and relays the data here over postMessage,
 *     since Chromium isolated worlds have no Xray.
 * <video>/<audio> elements and their live state (including per-tab volume) are
 * read directly from the content-script DOM on both — no page realm needed.
 *
 * Lightness: there is NO recurring metadata poll. A page with no media installs
 * zero timers (just a cheap MutationObserver). The single ~1s position ticker
 * runs ONLY while a media element is actively playing, and is cleared the
 * moment it pauses/ends. That ticker also re-reads metadata, so track changes
 * during playback are caught without a dedicated poll.
 */

"use strict";

(() => {
  if (window.__mprisHooked) return;
  window.__mprisHooked = true;

  // Cross-browser namespace: Firefox exposes `browser`, Chromium `chrome`.
  const browser = globalThis.browser ?? chrome;
  // Firefox reaches the page realm via Xray (wrappedJSObject / exportFunction).
  // Chromium has no Xray; the MAIN-world helper (content-main.js) reads the
  // page's Media Session and relays it here over postMessage. Everything else
  // (media-element observation, position ticker, per-tab volume, commands)
  // runs in this isolated world identically on both browsers.
  const XRAY = typeof exportFunction === "function";

  // ---- page-realm access (Firefox Xray) -----------------------------------
  function pageMediaSession() {
    if (!XRAY) return null; // Chromium: data arrives via the postMessage bridge
    try { return window.wrappedJSObject.navigator.mediaSession; } catch (_) { return null; }
  }

  /** Action handlers the page registered (drives CanGoNext / CanGoPrevious and
   *  lets us invoke Next/Previous). Stored as page-callable references. */
  const handlers = Object.create(null);

  /** Last navigator.mediaSession.setPositionState({...}) the page reported.
   *  Used for sites that drive playback without a DOM media element. */
  let positionState = null; // { duration, position, rate }

  // Chromium bridge mode: latest Media Session snapshot from content-main.js
  // (the MAIN-world helper). Null until the first state message arrives.
  let bridgedMeta = null;   // { title, artist, album, artwork, playbackState }

  // Patch setActionHandler, the metadata setter, and setPositionState through
  // the Xray. Each is best-effort and independent — a failure on a hardened
  // page just means we fall back to media-element observation. None of these
  // install a timer; they are passive interceptors. Firefox only — on Chromium
  // content-main.js does the equivalent patching in the page realm.
  if (XRAY) (function patchMediaSession() {
    const ms = pageMediaSession();
    if (!ms) { return; }
    const win = window.wrappedJSObject;

    try {
      const orig = ms.setActionHandler.bind(ms);
      ms.setActionHandler = exportFunction(function (action, handler) {
        try {
          if (handler) handlers[action] = handler;
          else delete handlers[action];
        } catch (_) {}
        scheduleNotify();
        return orig(action, handler);
      }, win);
    } catch (_) {}

    // metadata setter → notify on assignment (catches paused-tab track changes
    // that no media event would surface).
    try {
      const proto = win.MediaSession && win.MediaSession.prototype;
      const d = proto && Object.getOwnPropertyDescriptor(proto, "metadata");
      if (d && d.get && d.set) {
        Object.defineProperty(proto, "metadata", {
          configurable: true,
          get: d.get,
          set: exportFunction(function (v) { d.set.call(this, v); scheduleNotify(); }, win),
        });
      }
    } catch (_) {}

    // playbackState setter → notify.
    try {
      const proto = win.MediaSession && win.MediaSession.prototype;
      const d = proto && Object.getOwnPropertyDescriptor(proto, "playbackState");
      if (d && d.get && d.set) {
        Object.defineProperty(proto, "playbackState", {
          configurable: true,
          get: d.get,
          set: exportFunction(function (v) { d.set.call(this, v); scheduleNotify(); }, win),
        });
      }
    } catch (_) {}

    // setPositionState → capture authoritative duration/position/rate.
    try {
      const orig = ms.setPositionState && ms.setPositionState.bind(ms);
      if (orig) {
        ms.setPositionState = exportFunction(function (st) {
          try {
            if (st) {
              positionState = {
                duration: Number(st.duration) || 0,
                position: Number(st.position) || 0,
                rate: Number(st.playbackRate) || 1,
              };
            } else {
              positionState = null;
            }
          } catch (_) {}
          scheduleNotify();
          return orig(st);
        }, win);
      }
    } catch (_) {}
  })();

  // Chromium bridge: receive Media Session state from the MAIN-world helper and
  // mirror it into the same vars the Xray path fills (bridgedMeta / handlers /
  // positionState), then re-report. handlers hold `true` flags here (presence
  // only — the actual page-realm callables live in content-main.js).
  if (!XRAY) {
    window.addEventListener("message", (ev) => {
      if (ev.source !== window) return;
      const d = ev.data;
      if (!d || d.source !== "mpris-main" || d.type !== "state") return;
      bridgedMeta = d.meta || null;
      positionState = d.positionState || null;
      for (const k of Object.keys(handlers)) delete handlers[k];
      if (d.handlers) for (const k in d.handlers) if (d.handlers[k]) handlers[k] = true;
      scheduleNotify();
    });
    // Ask the MAIN helper to push current state (covers injection-order races).
    try { window.postMessage({ source: "mpris-iso", type: "request-state" }, "*"); } catch (_) {}
  }

  function readMetadata() {
    if (!XRAY) return bridgedMeta; // Chromium: supplied by content-main.js
    try {
      const ms = pageMediaSession();
      const md = ms && ms.metadata;
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
        playbackState: String((ms && ms.playbackState) || "none"),
      };
    } catch (_) { return null; }
  }

  // ---- media element discovery & hooking -----------------------------------
  const HOOKED = new WeakSet();
  let positionTimer = null;

  function startPositionTicker() {
    if (positionTimer) return;
    // The ONLY recurring timer, and only while something is actually playing.
    // Re-reads the whole track, so it also catches metadata changes mid-play.
    // Self-stops if playback has ended without a pause/ended/emptied event
    // (e.g. the element was removed from the DOM) so it can't leak.
    positionTimer = setInterval(() => {
      if (!anyPlaying()) { stopPositionTicker(); }
      scheduleNotify();
    }, 1000);
  }
  function stopPositionTicker() {
    if (!positionTimer) return;
    clearInterval(positionTimer);
    positionTimer = null;
  }

  let seekedSinceNotify = false;

  function onMediaEvent(ev) {
    const t = ev.type;
    if (t === "seeked") seekedSinceNotify = true;
    if (t === "play" || t === "playing") startPositionTicker();
    if (t === "pause" || t === "ended" || t === "emptied") {
      // Stop the ticker only if nothing else is still playing.
      if (!anyPlaying()) stopPositionTicker();
    }
    scheduleNotify();
  }

  function hookMedia(m) {
    if (HOOKED.has(m)) return;
    HOOKED.add(m);
    ["play", "pause", "playing", "timeupdate", "durationchange", "seeked",
     "volumechange", "ended", "loadedmetadata", "ratechange", "emptied"]
      .forEach((ev) => m.addEventListener(ev, onMediaEvent, true));
    if (!m.paused && !m.ended) startPositionTicker();
  }

  function allMedia() {
    try { return Array.from(document.querySelectorAll("video, audio")); }
    catch (_) { return []; }
  }
  function anyPlaying() {
    return allMedia().some((m) => !m.paused && !m.ended);
  }

  document.addEventListener("DOMContentLoaded", () => allMedia().forEach(hookMedia), { once: true });
  allMedia().forEach(hookMedia);

  const obs = new MutationObserver((muts) => {
    let found = false;
    for (const mu of muts) {
      if (!mu.addedNodes) continue;
      mu.addedNodes.forEach((n) => {
        if (n.nodeType !== 1) return;
        if (n.tagName === "VIDEO" || n.tagName === "AUDIO") { hookMedia(n); found = true; }
        if (typeof n.querySelectorAll === "function") {
          n.querySelectorAll("video, audio").forEach((c) => { hookMedia(c); found = true; });
        }
      });
    }
    if (found) scheduleNotify();
  });
  try { obs.observe(document.documentElement, { childList: true, subtree: true }); } catch (_) {}

  // ---- track building -------------------------------------------------------
  function parseSizeArea(s) {
    if (!s || typeof s !== "string") return 0;
    if (s === "any") return Number.MAX_SAFE_INTEGER;
    const m = /(\d+)x(\d+)/.exec(s);
    return m ? Number(m[1]) * Number(m[2]) : 0;
  }

  function bestArtwork(artwork) {
    if (!artwork || !artwork.length) return "";
    let best = artwork[0], area = parseSizeArea(best.sizes);
    for (let i = 1; i < artwork.length; i++) {
      const a = parseSizeArea(artwork[i].sizes);
      if (a > area) { area = a; best = artwork[i]; }
    }
    return best.src || "";
  }

  function youtubeFallbackArt() {
    if (!/^https?:\/\/([^.]+\.)?youtube\.com\/watch/.test(location.href)) return "";
    try {
      const v = new URL(location.href).searchParams.get("v");
      return v ? `https://i.ytimg.com/vi/${v}/maxresdefault.jpg` : "";
    } catch (_) { return ""; }
  }

  function faviconUrl() {
    try {
      const links = Array.from(document.querySelectorAll('link[rel~="icon"]'));
      if (!links.length) return "";
      let best = links[0], area = parseSizeArea(best.getAttribute("sizes"));
      for (const l of links) {
        const a = parseSizeArea(l.getAttribute("sizes"));
        if (a > area) { area = a; best = l; }
      }
      return best.href || "";
    } catch (_) { return ""; }
  }

  // A real "track" needs a finite duration of at least this long; shorter or
  // unknown-duration media (UI blips, notification sounds) shouldn't hijack the
  // player. Mirrors plasma-browser-integration's guard.
  const MIN_TRACK_SECONDS = 8;

  // Robust duration for a media element. YouTube and other MSE players report
  // the <video>.duration as 0/NaN (before metadata) or Infinity (some streaming
  // states) even mid-playback; the seekable range end is usually the true
  // duration in those windows. Returns 0 if genuinely unknown.
  function durationOf(m) {
    if (!m) return 0;
    const d = m.duration;
    if (isFinite(d) && d > 0) return d;
    try {
      const s = m.seekable;
      if (s && s.length > 0) {
        const end = s.end(s.length - 1);
        if (isFinite(end) && end > 0) return end;
      }
    } catch (_) {}
    return 0;
  }

  function pickActiveMedia() {
    const all = allMedia();
    if (!all.length) return null;
    const real = all.filter((m) => isFinite(m.duration) && m.duration >= MIN_TRACK_SECONDS);
    const pool = real.length ? real : all;
    const playing = pool.find((m) => !m.paused && !m.ended && (m.currentTime > 0 || m.readyState >= 2));
    if (playing) return playing;
    const withSrc = pool.filter((m) => m.currentSrc || m.src);
    if (!withSrc.length) return null;
    withSrc.sort((a, b) =>
      (b.videoWidth || 0) * (b.videoHeight || 0) - (a.videoWidth || 0) * (a.videoHeight || 0));
    return withSrc[0];
  }

  function buildTrack() {
    const media = pickActiveMedia();
    const meta = readMetadata();

    // What makes this page reportable at all:
    //   - a Media Session with real metadata, OR
    //   - a media element with a real duration.
    // A page that merely has a <title> is NOT a media player — the title
    // fallback below is only for DISPLAY once we've decided to report.
    const hasMeta = !!(meta && (meta.title || meta.artist || meta.album));

    // Duration: media element (or its seekable range) first, then the page's
    // setPositionState. Merging is the fix for the bar showing 0:26/0:26 — a
    // <video> whose duration momentarily reads 0/NaN/Infinity used to win and
    // suppress the full duration YouTube reports via setPositionState. The two
    // are on the same timeline, so element position + setPositionState duration
    // compose correctly.
    let duration = durationOf(media);
    if (!duration && positionState && isFinite(positionState.duration) && positionState.duration > 0) {
      duration = positionState.duration;
    }

    // Reportable: a real Media Session, or a media element with a known
    // duration. A page with only a <title> is not a player.
    const hasRealMedia = !!media && duration > 0;

    // Title: Media Session → (only when reporting via a media element)
    // document.title → hostname. The fallback never makes a page reportable on
    // its own; it just labels a bare-<video> player.
    let title = (meta && meta.title) || "";
    if (!title && hasRealMedia) title = (document.title || "").trim() || location.hostname || "";

    const artist = (meta && meta.artist) || "";
    const album = (meta && meta.album) || "";

    let artUrl = (meta && bestArtwork(meta.artwork)) || "";
    if (!artUrl && media && media.poster) artUrl = media.poster;
    if (!artUrl) artUrl = youtubeFallbackArt();
    if (!artUrl) artUrl = faviconUrl();

    // Position / rate / loop / volume: live media element first, with
    // setPositionState as the fallback (element-less players, or before the
    // element starts ticking).
    let position = 0, rate = 1, looping = false, volume = 1, playing;
    if (media) {
      position = isFinite(media.currentTime) ? media.currentTime : 0;
      if (!position && positionState && positionState.position > 0) position = positionState.position;
      rate = isFinite(media.playbackRate) && media.playbackRate > 0 ? media.playbackRate : 1;
      looping = !!media.loop;
      volume = isFinite(media.volume) ? (media.muted ? 0 : media.volume) : 1;
      playing = !media.paused && !media.ended;
    } else if (positionState) {
      position = positionState.position || 0;
      rate = positionState.rate || 1;
      playing = meta ? meta.playbackState === "playing" : false;
    } else {
      playing = meta ? meta.playbackState === "playing" : false;
    }

    const seeked = seekedSinceNotify;

    return {
      title, artist, album, artUrl, pageUrl: location.href,
      duration, position, playing, seeked, volume, rate, looping,
      canSeek: duration > 0,
      canGoNext: !!handlers.nexttrack,
      canGoPrevious: !!handlers.previoustrack,
      _hasMeta: hasMeta,
      _hasRealMedia: hasRealMedia,
    };
  }

  function shouldReport(t) {
    // Report only when there's an actual media source: a Media Session with
    // real metadata, or a media element with a real duration. A page that
    // just has a <title> is not a player. A paused element with finite
    // duration stays reported (the player persists across pause) — only true
    // media-gone removes it.
    return t._hasMeta || t._hasRealMedia;
  }

  // ---- reporting & dedupe ---------------------------------------------------
  // Position/seeked are deliberately excluded from the key: they don't map to a
  // pushed MPRIS property (Position is polled; Seeked is sent out-of-band), so
  // including them would defeat the steady-state no-traffic goal.
  let lastKey = "";
  let reported = false;

  function trackKey(t) {
    return [t.title, t.artist, t.album, t.artUrl,
      Math.round(t.duration * 10), t.playing ? 1 : 0,
      Math.round(t.volume * 100), Math.round(t.rate * 100),
      t.looping ? 1 : 0, t.canSeek ? 1 : 0,
      t.canGoNext ? 1 : 0, t.canGoPrevious ? 1 : 0].join("|");
  }

  function notify() {
    const track = buildTrack();
    if (shouldReport(track)) {
      const key = trackKey(track);
      const seeked = track.seeked;
      seekedSinceNotify = false;
      // Always send if a seek happened (position/Seeked is out-of-band), else
      // only on a real field change.
      if (key === lastKey && !seeked) { return; }
      lastKey = key;
      reported = true;
      delete track._hasMeta;
      delete track._hasRealMedia;
      browser.runtime.sendMessage({ kind: "update", track }).catch(() => {});
    } else if (reported) {
      reported = false; lastKey = "";
      browser.runtime.sendMessage({ kind: "remove" }).catch(() => {});
    }
  }

  let notifyTimer = null;
  function scheduleNotify() {
    if (notifyTimer) return;
    notifyTimer = setTimeout(() => { notifyTimer = null; notify(); }, 80);
  }

  // ---- commands from the host (via background) ------------------------------
  // Post a command to the MAIN-world helper (Chromium), which holds the actual
  // page-realm Media Session handlers and invokes them.
  function postToMain(msg) {
    try { window.postMessage({ source: "mpris-iso", ...msg }, "*"); } catch (_) {}
  }

  function callHandler(name) {
    const h = handlers[name];
    if (!h) return false;
    if (!XRAY) { postToMain({ type: "invoke", name }); return true; }
    try { h(); return true; } catch (_) { return false; }
  }

  // Invoke a MediaSession seek handler with its detail object, used when there
  // is no DOM media element to scrub. On Firefox the detail must be cloned into
  // the page compartment to cross the Xray; on Chromium content-main.js (which
  // lives in the page realm) invokes it. Best-effort either way.
  function callSeekHandler(name, detail) {
    const h = handlers[name];
    if (!h) return false;
    if (!XRAY) { postToMain({ type: "invoke-seek", name, detail }); return true; }
    try {
      const arg = (typeof cloneInto === "function") ? cloneInto(detail, window) : detail;
      h(arg);
      return true;
    } catch (_) { return false; }
  }

  function handleCommand(action, value) {
    const m = pickActiveMedia();
    switch (action) {
      case "play": if (!callHandler("play") && m) m.play().catch(() => {}); break;
      case "pause": if (!callHandler("pause") && m) m.pause(); break;
      case "playpause":
        if (m) { (m.paused || m.ended) ? m.play().catch(() => {}) : m.pause(); }
        else { callHandler("play") || callHandler("pause"); }
        break;
      case "stop":
        if (!callHandler("stop") && m) { m.pause(); try { m.currentTime = 0; } catch (_) {} }
        break;
      case "next": callHandler("nexttrack"); break;
      case "previous": callHandler("previoustrack"); break;
      case "seek":
        if (typeof value === "number" && isFinite(value)) {
          if (m) { try { m.currentTime = Math.max(0, m.currentTime + value); } catch (_) {} }
          else callSeekHandler(value < 0 ? "seekbackward" : "seekforward", { seekOffset: Math.abs(value) });
        }
        break;
      case "setposition":
        if (typeof value === "number" && isFinite(value)) {
          if (m) {
            const max = isFinite(m.duration) ? m.duration : value;
            try { m.currentTime = Math.max(0, Math.min(max, value)); } catch (_) {}
          } else {
            callSeekHandler("seekto", { seekTime: value });
          }
        }
        break;
      case "setvolume":
        if (m && typeof value === "number" && isFinite(value)) {
          try {
            if (value <= 0) { m.muted = true; }
            else { m.volume = Math.max(0, Math.min(1, value)); m.muted = false; }
          } catch (_) {}
        }
        break;
      case "setrate":
        if (m && typeof value === "number" && isFinite(value) && value > 0) {
          try { m.playbackRate = value; } catch (_) {}
        }
        break;
      case "setloop":
        if (m && typeof value === "number") { try { m.loop = value > 0.5; } catch (_) {} }
        break;
      case "__resync":
        lastKey = ""; reported = false; notify(); return;
    }
    scheduleNotify();
  }

  browser.runtime.onMessage.addListener((msg) => {
    if (!msg) return;
    if (msg.kind === "mpris-command") handleCommand(msg.action, msg.value);
    else if (msg.kind === "mpris-resync") handleCommand("__resync");
  });

  // ---- navigation & lifecycle ----------------------------------------------
  // SPA navigation (YouTube etc.) without a full reload — event-driven, no poll.
  function onNav() { lastKey = ""; scheduleNotify(); }
  if (window.navigation && typeof window.navigation.addEventListener === "function") {
    try { window.navigation.addEventListener("navigate", onNav); } catch (_) {}
  } else {
    // Fallback: wrap history pushState/replaceState + popstate.
    try {
      const wrap = (name) => {
        const orig = history[name];
        history[name] = function () { const r = orig.apply(this, arguments); onNav(); return r; };
      };
      wrap("pushState"); wrap("replaceState");
      window.addEventListener("popstate", onNav);
    } catch (_) {}
  }

  // bfcache: restoring from back/forward cache re-reports; suspending into it
  // is NOT a hard remove (the page may come back).
  window.addEventListener("pageshow", (e) => { if (e.persisted) { lastKey = ""; reported = false; scheduleNotify(); } });
  window.addEventListener("pagehide", (e) => {
    if (e.persisted) return; // going into bfcache — keep the player
    if (reported) { browser.runtime.sendMessage({ kind: "remove" }).catch(() => {}); reported = false; }
  });

  // Initial probe.
  scheduleNotify();
})();
