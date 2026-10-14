const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const extension = path.join(__dirname, '..', 'extension');
function event() {
  const listeners = [];
  return { addListener: fn => listeners.push(fn), emit: (...args) => listeners.forEach(fn => fn(...args)) };
}
function clock() {
  let now = 1000, seq = 0;
  const timers = new Map();
  class DateMock extends Date { static now() { return now; } }
  const add = (fn, delay, interval) => { const id = ++seq; timers.set(id, { fn, at: now + delay, delay, interval }); return id; };
  return { Date: DateMock, setTimeout: (fn, delay) => add(fn, delay, false), setInterval: (fn, delay) => add(fn, delay, true), clearTimeout: id => timers.delete(id), clearInterval: id => timers.delete(id),
    tick(ms) { const end = now + ms; for (;;) { const next = [...timers.entries()].filter(([, t]) => t.at <= end).sort((a,b) => a[1].at-b[1].at)[0]; if (!next) break; const [id,t]=next; now=t.at; if (t.interval) t.at+=t.delay; else timers.delete(id); t.fn(); } now=end; },
    jump(ms) { now += ms; }, timers };
}
function media(overrides = {}) {
  const listeners = {};
  return Object.assign({ duration: 120, currentTime: 20, paused: false, ended: false, muted: false, volume: 1, playbackRate: 1, readyState: 4, currentSrc: 'https://example.com/movie.mp4', loop: false,
    seekable: { length: 1, start: () => 0, end: () => 120 },
    addEventListener(name, fn) { (listeners[name] ??= []).push(fn); },
    emit(type) { for (const fn of listeners[type] ?? []) fn({type, target:this}); },
    play() { this.playCalls=(this.playCalls ?? 0)+1; this.paused=false; return Promise.resolve(); },
    pause() { this.pauseCalls=(this.pauseCalls ?? 0)+1; this.paused=true; }
  }, overrides);
}
function content(elements = [], firefox = false) {
  const time=clock(), sent=[], posts=[], listeners={}, observers=[];
  const window={addEventListener:(name,fn)=>(listeners[name] ??= []).push(fn), postMessage:m=>posts.push(m)};
  const incoming=event();
  const ctx={...time, window, chrome:{runtime:{sendMessage:m=>{sent.push(structuredClone(m));return Promise.resolve();},onMessage:incoming}},document:{title:'Example',documentElement:{},querySelectorAll:s=>s==='video, audio'?elements:[],addEventListener(){}},location:{href:'https://example.com/watch',hostname:'example.com'},history:{pushState(){},replaceState(){}},MutationObserver:class{constructor(fn){this.fn=fn;observers.push(this);}observe(){}},console,URL};
  const session={metadata:null,playbackState:'none',setActionHandler(){},setPositionState(){}};
  if (firefox) { ctx.exportFunction=fn=>fn;ctx.cloneInto=v=>v;window.wrappedJSObject={navigator:{mediaSession:session}}; }
  vm.createContext(ctx);
  const src=fs.readFileSync(path.join(extension,'content.js'),'utf8').replace(/\}\)\(\);\s*$/, 'globalThis.T={notify,buildTrack,handleCommand,handlers,setMeta:v=>bridgedMeta=v};})();');
  vm.runInContext(src,ctx);
  const emit=(name,value)=>{for(const fn of listeners[name]??[])fn(value);};
  return {T:ctx.T,time,sent,posts,ctx,emit,observers,incoming,session};
}
function background() {
  const time=clock(),sent=[],ports=[],runtime={onMessage:event(),onStartup:event(),onInstalled:event(),getManifest:()=>({version:'0.2.3'})};
  runtime.connectNative=()=>{const p={onMessage:event(),onDisconnect:event(),postMessage:m=>sent.push(structuredClone(m))};ports.push(p);return p;};
  const tabs={query:()=>Promise.resolve([]),sendMessage:()=>Promise.resolve(),onRemoved:event(),onDetached:event(),onAttached:event(),onUpdated:event(),onReplaced:event()};
  const ctx={...time,chrome:{runtime,tabs,windows:{onRemoved:event()}},console};vm.createContext(ctx);vm.runInContext(fs.readFileSync(path.join(extension,'background.js'),'utf8'),ctx);
  const update=(tabId,windowId,track={},documentId='a')=>runtime.onMessage.emit({kind:'update',track:{title:'Track',playing:true,...track},documentId},{tab:{id:tabId,windowId},frameId:0});
  return {time,sent,ports,tabs,update,runtime,windows:ctx.chrome.windows};
}
module.exports={event,clock,media,content,background,extension};
