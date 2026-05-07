/*
 * Page-context script. Runs in the same realm as the page so it can read
 * `navigator.mediaSession.metadata` (a MediaMetadata object whose fields the
 * content-script realm can't reach directly across the X-ray wrapper) and
 * see live `<video>`/`<audio>` element state.
 *
 * Communicates with the content script via window.postMessage, tagged so
 * other page scripts ignore our chatter.
 */

(function () {
  if (window.__mprisFxHooked) return;
  window.__mprisFxHooked = true;

  const TAG_OUT = "mpris-fx";
  const TAG_IN = "mpris-fx-cmd";

  /** Action handlers the page has registered via setActionHandler. */
  const handlers = Object.create(null);
  const ms = navigator.mediaSession;

  if (!ms) {
    // Browser without Media Session API — bail. Should never happen on
    // modern Firefox.
    return;
  }

  // Patch setActionHandler so we know which actions the page supports
  // (drives canGoNext / canGoPrevious / canPlay / canPause).
  try {
    const origSet = ms.setActionHandler.bind(ms);
    ms.setActionHandler = function (action, handler) {
      if (handler) {
        handlers[action] = handler;
      } else {
        delete handlers[action];
      }
      scheduleNotify();
      return origSet(action, handler);
    };
  } catch (e) {
    console.warn("[mpris-fx] setActionHandler patch failed:", e);
  }

  function pickActiveMedia() {
    const all = Array.from(document.querySelectorAll("video, audio"));
    if (!all.length) return null;
    // Prefer something actually playing.
    const playing = all.find(
      (m) => !m.paused && !m.ended && (m.currentTime > 0 || m.readyState >= 2)
    );
    if (playing) return playing;
    // Otherwise, the largest one with a source — usually the main player on
    // pages that have decorative or hidden media too.
    const withSrc = all.filter((m) => m.currentSrc || m.src);
    if (!withSrc.length) return null;
    withSrc.sort(
      (a, b) =>
        (b.videoWidth || 0) * (b.videoHeight || 0) -
        (a.videoWidth || 0) * (a.videoHeight || 0)
    );
    return withSrc[0];
  }

  function pickArtworkUrl(artwork) {
    if (!artwork || !artwork.length) return "";
    let best = artwork[0];
    let bestArea = parseSizeStr(best.sizes);
    for (let i = 1; i < artwork.length; i++) {
      const a = parseSizeStr(artwork[i].sizes);
      if (a > bestArea) {
        bestArea = a;
        best = artwork[i];
      }
    }
    return best.src || "";
  }

  function parseSizeStr(s) {
    if (!s || typeof s !== "string") return 0;
    if (s === "any") return Number.MAX_SAFE_INTEGER;
    const m = /(\d+)x(\d+)/.exec(s);
    return m ? Number(m[1]) * Number(m[2]) : 0;
  }

  /**
   * Some sites (notably YouTube) ship low-res "still" thumbnails in the
   * MediaSession artwork. If the page URL is a watch URL we can build a
   * higher-quality thumbnail from the video id when no large artwork is
   * advertised.
   */
  function youtubeFallbackArt() {
    if (!/^https?:\/\/([^.]+\.)?youtube\.com\/watch/.test(location.href))
      return "";
    try {
      const u = new URL(location.href);
      const v = u.searchParams.get("v");
      if (!v) return "";
      return `https://i.ytimg.com/vi/${v}/maxresdefault.jpg`;
    } catch (e) {
      return "";
    }
  }

  function buildTrack() {
    const media = pickActiveMedia();
    const meta = ms.metadata;
    const title = (meta && meta.title) || "";
    const artist = (meta && meta.artist) || "";
    const album = (meta && meta.album) || "";
    let artUrl = (meta && pickArtworkUrl(meta.artwork)) || "";
    if (!artUrl) {
      artUrl = youtubeFallbackArt();
    }

    const duration =
      media && isFinite(media.duration) && media.duration > 0
        ? media.duration
        : 0;
    const position = media && isFinite(media.currentTime) ? media.currentTime : 0;
    const playing = media
      ? !media.paused && !media.ended
      : ms.playbackState === "playing";
    const volume =
      media && isFinite(media.volume)
        ? media.muted
          ? 0
          : media.volume
        : 1.0;

    return {
      title,
      artist,
      album,
      artUrl,
      pageUrl: location.href,
      duration,
      position,
      playing,
      volume,
      canSeek: !!media && isFinite(media.duration) && media.duration > 0,
      canGoNext: !!handlers.nexttrack,
      canGoPrevious: !!handlers.previoustrack,
    };
  }

  function shouldReportTrack(track) {
    // Only report if we have something meaningful — title OR a media element
    // with a real duration. Otherwise the page is just a static video element
    // sitting around.
    if (track.title) return true;
    if (track.duration > 0 && (track.playing || track.position > 0)) return true;
    return false;
  }

  let lastTrackKey = "";
  let reported = false;

  function trackKey(t) {
    return [
      t.title,
      t.artist,
      t.album,
      t.artUrl,
      Math.round(t.duration * 10),
      Math.round(t.position * 4),
      t.playing ? 1 : 0,
      Math.round(t.volume * 100),
      t.canSeek ? 1 : 0,
      t.canGoNext ? 1 : 0,
      t.canGoPrevious ? 1 : 0,
    ].join("|");
  }

  function notify() {
    const track = buildTrack();
    if (shouldReportTrack(track)) {
      const key = trackKey(track);
      if (key === lastTrackKey) return;
      lastTrackKey = key;
      reported = true;
      window.postMessage({ tag: TAG_OUT, kind: "update", track }, "*");
    } else if (reported) {
      reported = false;
      lastTrackKey = "";
      window.postMessage({ tag: TAG_OUT, kind: "remove" }, "*");
    }
  }

  let notifyTimer = null;
  function scheduleNotify() {
    if (notifyTimer) return;
    notifyTimer = setTimeout(() => {
      notifyTimer = null;
      notify();
    }, 80);
  }

  // Hook media element events.
  const HOOKED = new WeakSet();
  function hookMedia(m) {
    if (HOOKED.has(m)) return;
    HOOKED.add(m);
    [
      "play",
      "pause",
      "playing",
      "timeupdate",
      "durationchange",
      "seeked",
      "volumechange",
      "ended",
      "loadedmetadata",
      "ratechange",
      "emptied",
    ].forEach((ev) => m.addEventListener(ev, scheduleNotify, true));
  }

  document.querySelectorAll("video, audio").forEach(hookMedia);
  const obs = new MutationObserver((mutations) => {
    let any = false;
    for (const m of mutations) {
      if (!m.addedNodes) continue;
      m.addedNodes.forEach((n) => {
        if (n.nodeType !== 1) return;
        if (n.tagName === "VIDEO" || n.tagName === "AUDIO") {
          hookMedia(n);
          any = true;
        }
        if (typeof n.querySelectorAll === "function") {
          n.querySelectorAll("video, audio").forEach((c) => {
            hookMedia(c);
            any = true;
          });
        }
      });
    }
    if (any) scheduleNotify();
  });
  obs.observe(document.documentElement, { childList: true, subtree: true });

  // Poll metadata at low frequency — there's no event when the page assigns
  // navigator.mediaSession.metadata = new MediaMetadata({...}).
  setInterval(scheduleNotify, 500);
  // Periodic refresh while playing so the host gets fresh position samples
  // for its interpolation logic. The trackKey filter prevents spam when
  // nothing actually changed.
  setInterval(() => {
    const media = pickActiveMedia();
    if (media && !media.paused) scheduleNotify();
  }, 1000);
  // Belt-and-braces heartbeat: every 10 seconds, drop the dedupe key and
  // force a re-send. This guarantees recovery from any silent host respawn
  // (idle reaping, extension reload, host crash) within 10s, even if the
  // background's resync message didn't reach us.
  setInterval(() => {
    if (reported) {
      lastTrackKey = "";
      scheduleNotify();
    }
  }, 10000);

  // Inbound commands from the content script.
  window.addEventListener("message", (e) => {
    if (e.source !== window) return;
    const d = e.data;
    if (!d || d.tag !== TAG_IN) return;
    handleCmd(d.action, d.value);
  });

  function handleCmd(action, value) {
    // __resync: the background script reconnected to a fresh native host
    // (which has empty state). Forget our dedupe key so the next notify()
    // unconditionally re-sends the current track.
    if (action === "__resync") {
      lastTrackKey = "";
      reported = false;
      notify();
      return;
    }
    const m = pickActiveMedia();
    switch (action) {
      case "play":
        if (callHandler("play")) break;
        if (m) m.play().catch(() => {});
        break;
      case "pause":
        if (callHandler("pause")) break;
        if (m) m.pause();
        break;
      case "playpause":
        if (m) {
          if (m.paused || m.ended) m.play().catch(() => {});
          else m.pause();
        } else {
          callHandler("play") || callHandler("pause");
        }
        break;
      case "stop":
        if (callHandler("stop")) break;
        if (m) {
          m.pause();
          try {
            m.currentTime = 0;
          } catch (_) {}
        }
        break;
      case "next":
        callHandler("nexttrack");
        break;
      case "previous":
        callHandler("previoustrack");
        break;
      case "seek":
        if (m && typeof value === "number" && isFinite(value)) {
          try {
            m.currentTime = Math.max(0, m.currentTime + value);
          } catch (_) {}
        }
        break;
      case "setposition":
        if (m && typeof value === "number" && isFinite(value)) {
          const max = isFinite(m.duration) ? m.duration : value;
          try {
            m.currentTime = Math.max(0, Math.min(max, value));
          } catch (_) {}
        }
        break;
      case "setvolume":
        if (m && typeof value === "number" && isFinite(value)) {
          try {
            m.volume = Math.max(0, Math.min(1, value));
            m.muted = false;
          } catch (_) {}
        }
        break;
    }
    scheduleNotify();
  }

  function callHandler(name) {
    const h = handlers[name];
    if (!h) return false;
    try {
      h();
      return true;
    } catch (e) {
      console.warn("[mpris-fx] handler", name, "threw:", e);
      return false;
    }
  }

  // Page navigation in SPAs (YouTube does this) — not needed for content
  // script unload, but we want to clear stale state when the URL changes
  // dramatically.
  let lastHref = location.href;
  setInterval(() => {
    if (location.href !== lastHref) {
      lastHref = location.href;
      lastTrackKey = "";
      scheduleNotify();
    }
  }, 700);

  // Initial probe.
  scheduleNotify();
})();
