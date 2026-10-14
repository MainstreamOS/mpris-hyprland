const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs'),vm=require('node:vm'),path=require('node:path');
const {media,content,background,extension}=require('./harness.cjs');

test('muting from desktop keeps bare media registered and unmute works',()=>{
 const m=media(),c=content([m]);c.T.notify();c.T.handleCommand('setvolume',0);c.T.notify();
 assert.equal(c.sent.at(-1).kind,'update');assert.equal(c.sent.at(-1).track.volume,0);
 c.T.handleCommand('setvolume',.7);c.T.notify();assert.equal(m.muted,false);assert.equal(c.sent.at(-1).track.volume,.7);
});
test('muted decorative video remains unreported',()=>{const c=content([media({muted:true})]);c.T.notify();assert.equal(c.sent.length,0);});
test('controlled mute does not switch commands to a different element',()=>{
 const a=media(),b=media({paused:true});const c=content([a,b]);c.T.handleCommand('setvolume',0);c.T.handleCommand('setvolume',.6);assert.equal(a.volume,.6);assert.equal(b.volume,1);
});
test('buffering freezes reported playback and resume sends a fresh position',()=>{
 const m=media(),c=content([m]);c.T.notify();m.emit('waiting');c.T.notify();assert.equal(c.sent.at(-1).track.playing,false);
 c.time.jump(5000);m.currentTime=21;m.emit('playing');c.T.notify();assert.equal(c.sent.at(-1).track.playing,true);assert.equal(c.sent.at(-1).track.position,21);
});
test('unannounced drift is corrected, normal rate-scaled playback stays quiet',()=>{
 const m=media({playbackRate:2}),c=content([m]);c.T.notify();c.time.jump(1000);m.currentTime=22;c.T.notify();assert.equal(c.sent.length,1);
 c.time.jump(5000);m.currentTime=22;c.T.notify();assert.equal(c.sent.length,2);assert.equal(c.sent.at(-1).track.seeked,true);
});
test('route-only navigation is not deduped',()=>{const c=content([media()]);c.T.notify();c.ctx.location.href='https://example.com/other';c.T.notify();assert.equal(c.sent.length,2);});
test('MSE duration fallback outranks short UI audio',()=>{
 const c=content([media({duration:1}),media({duration:NaN,currentTime:30})]);assert.equal(c.T.buildTrack().position,30);assert.equal(c.T.buildTrack().duration,120);
});
test('live streams have unknown length while preserving actual DVR capability',()=>{
 const c=content([media({duration:Infinity,currentTime:900,seekable:{length:1,start:()=>800,end:()=>1000}})]);
 assert.equal(c.T.buildTrack().duration,0);assert.equal(c.T.buildTrack().canSeek,true);c.T.handleCommand('seek',-500);assert.equal(c.T.buildTrack().position,800);
 const noDvr=content([media({duration:Infinity,seekable:{length:0}})]);assert.equal(noDvr.T.buildTrack().canSeek,false);
});
test('seek-to-zero does not resurrect stale Media Session position',()=>{const c=content([media({currentTime:0})]);c.ctx.T.setMeta({title:'Test'});assert.equal(c.T.buildTrack().position,0);});
test('Firefox PlayPause uses correct handler and action details',()=>{
 const c=content([media()],true),calls=[];c.session.setActionHandler('pause',details=>calls.push(details.action));c.T.handleCommand('playpause');assert.deepEqual(calls,['pause']);
});
test('element-less playing Media Session toggles to pause',()=>{
 const c=content([],true),calls=[];c.session.metadata={title:'Audio'};c.session.playbackState='playing';
 c.session.setActionHandler('play',details=>calls.push(details.action));c.session.setActionHandler('pause',details=>calls.push(details.action));c.T.handleCommand('playpause');assert.deepEqual(calls,['pause']);
});
test('seek handlers include action and seek data',()=>{
 const c=content([],true),calls=[];c.session.setActionHandler('seekforward',d=>calls.push(d));c.T.handleCommand('seek',10);assert.equal(calls[0].action,'seekforward');assert.equal(calls[0].seekOffset,10);
});
test('Chromium failure acknowledgement falls back to DOM controls',()=>{
 const m=media(),c=content([m]);c.T.handlers.pause=true;c.T.handleCommand('pause');const invoke=c.posts.at(-1);assert.equal(m.pauseCalls,undefined);
 c.emit('message',{source:c.ctx.window,data:{source:'mpris-main',type:'result',id:invoke.id,handled:false}});assert.equal(m.pauseCalls,1);
});
test('MAIN helper passes details and acknowledges thrown handlers',async()=>{
 const listeners={},posts=[],ms={setActionHandler(){},setPositionState(){},metadata:null};const window={addEventListener:(n,f)=>listeners[n]=f,postMessage:m=>posts.push(m)};
 vm.runInNewContext(fs.readFileSync(path.join(extension,'content-main.js'),'utf8'),{window,navigator:{mediaSession:ms},setTimeout:()=>1,console});
 let action;ms.setActionHandler('pause',d=>{action=d.action;});listeners.message({source:window,data:{source:'mpris-iso',type:'invoke',name:'pause',id:1}});await Promise.resolve();assert.equal(action,'pause');assert.equal(posts.at(-1).handled,true);
 ms.setActionHandler('pause',()=>{throw Error('failure');});listeners.message({source:window,data:{source:'mpris-iso',type:'invoke',name:'pause',id:2}});assert.equal(posts.at(-1).handled,false);
});
test('removing a paused element schedules cleanup',()=>{
 const elements=[media({paused:true})],c=content(elements);c.T.notify();const old=elements.pop();c.observers[0].fn([{addedNodes:[],removedNodes:[{nodeType:1,tagName:'VIDEO'}]}]);c.time.tick(100);assert.equal(c.sent.at(-1).kind,'remove');
});
test('back-forward cache suspends player and restores it on pageshow',()=>{
 const c=content([media()]);c.T.notify();c.emit('pagehide',{persisted:true});assert.equal(c.sent.at(-1).kind,'remove');const n=c.sent.length;c.time.tick(2000);assert.equal(c.sent.length,n);c.emit('pageshow',{persisted:true});c.time.tick(100);assert.equal(c.sent.at(-1).kind,'update');
});
test('oversized and blob artwork never reaches native host',()=>{
 const b=background();b.update(10,1,{artUrl:'data:image/png;base64,'+'x'.repeat(2**20),title:'a'.repeat(2**20)});const u=b.sent.at(-1);assert.equal(u.artUrl,'');assert.equal(u.title.length,4096);assert(Buffer.byteLength(JSON.stringify(u))<2**20);
 b.update(10,1,{artUrl:'blob:https://example.com/foo'});assert.equal(b.sent.at(-1).artUrl,'');
});
test('tab update with changed window removes the old player',()=>{const b=background();b.update(10,1);b.update(10,2);assert(b.sent.some(m=>m.type==='remove'&&m.tabId===1));assert.equal(b.sent.at(-1).tabId,2);});
test('tab detach/attach resynchronizes both windows',()=>{const b=background();b.update(10,1);b.tabs.onDetached.emit(10,{oldWindowId:1});assert.equal(b.sent.at(-1).type,'remove');b.tabs.onAttached.emit(10,{newWindowId:2});assert.equal(b.sent.at(-1).tabId,2);});
test('discard, navigation, replacement and close remove players',()=>{
 for(const action of ['discard','navigation','replace','close']){const b=background();b.update(10,1);if(action==='discard')b.tabs.onUpdated.emit(10,{discarded:true});if(action==='navigation')b.tabs.onUpdated.emit(10,{status:'loading'});if(action==='replace')b.tabs.onReplaced.emit(11,10);if(action==='close')b.tabs.onRemoved.emit(10);assert.equal(b.sent.at(-1).type,'remove',action);}
});
test('late removal from previous document cannot remove new frame',()=>{
 const b=background();b.update(10,1,{},'old');b.update(10,1,{title:'New'},'new');const n=b.sent.length;b.runtime.onMessage.emit({kind:'remove',documentId:'old'},{tab:{id:10,windowId:1},frameId:0});assert.equal(b.sent.length,n);
});
test('repeated early disconnects retain exponential reconnect backoff',()=>{
 const b=background();b.ports[0].onDisconnect.emit();b.time.tick(500);assert.equal(b.ports.length,2);b.ports[1].onDisconnect.emit();b.time.tick(500);assert.equal(b.ports.length,2);b.time.tick(500);assert.equal(b.ports.length,3);
});

test('detach cleans stored window even when browser reports a newer window ID',()=>{const b=background();b.update(10,1);b.tabs.onDetached.emit(10,{oldWindowId:29});assert(b.sent.some(m=>m.type==='remove'&&m.tabId===1));});

test('window closure clears its source even without tab-removal events',()=>{const b=background();b.update(10,1);b.windows.onRemoved.emit(1);assert.equal(b.sent.at(-1).type,'remove');});

test('DOM controls are advertised only while a real media element exists',()=>{
 const elements=[media()],c=content(elements);c.T.setMeta({title:'Session'});c.T.notify();
 for(const key of ['canSetVolume','canSetRate','canSetLoop'])assert.equal(c.sent.at(-1).track[key],true);
 elements.length=0;c.T.notify();
 for(const key of ['canSetVolume','canSetRate','canSetLoop'])assert.equal(c.sent.at(-1).track[key],false);
});
test('background validates control capabilities rather than trusting truthy strings',()=>{
 const b=background();b.update(10,1,{canSetVolume:'true',canSetRate:true,canSetLoop:1});
 assert.equal(b.sent.at(-1).canSetVolume,false);assert.equal(b.sent.at(-1).canSetRate,true);assert.equal(b.sent.at(-1).canSetLoop,false);
});
test('unusable high-resolution artwork cannot mask usable alternatives',()=>{
 const c=content([media({poster:'https://example.com/poster.png'})]);
 c.T.setMeta({title:'Art',artwork:[{src:'blob:https://example.com/art',sizes:'any'},{src:'https://example.com/cover.png',sizes:'64x64'}]});
 assert.equal(c.T.buildTrack().artUrl,'https://example.com/cover.png');
 c.T.setMeta({title:'Art',artwork:[{src:'data:image/png;base64,'+'x'.repeat(256*1024),sizes:'any'}]});
 assert.equal(c.T.buildTrack().artUrl,'https://example.com/poster.png');
});
test('invalid posters fall back to a usable favicon',()=>{
 const c=content([media({poster:'blob:https://example.com/art'})]);
 c.ctx.document.querySelectorAll=s=>s==='video, audio'?[media({poster:'blob:bad'})]:[
  {href:'blob:bad',getAttribute:()=> 'any'}, {href:'https://example.com/icon.png',getAttribute:()=> '32x32'}];
 assert.equal(c.T.buildTrack().artUrl,'https://example.com/icon.png');
});
