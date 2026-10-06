'use strict';
let liveSource, liveReady=false, liveRunning=false, liveTimer, liveStopped=false;
const livePending=new Set();
let knownMembers=new Set();
async function refreshLiveViews() {
  if(liveRunning || liveStopped) return;
  liveRunning=true;
  const kinds=new Set(livePending);livePending.clear();
  try {
    if(!currentUser) currentUser=await (await api('me')).json();
    await loadNotifications();if(!$('submissionsview').hidden)await loadSubmissions();
    if(kinds.has('refresh') || kinds.has('chat'))await loadChat();
    if(kinds.has('refresh') || kinds.has('announcement'))await loadAnnouncement();
    if(kinds.has('refresh') || kinds.has('members')) {
      const fresh=await (await api('profiles')).json();
      const joined=fresh.filter(m=>!knownMembers.has(m.id));
      if(knownMembers.size && joined.length && kinds.has('members')) $('livenotice').textContent=joined.map(m=>m.username).join(', ')+' joined the community.';
      knownMembers=new Set(fresh.map(m=>m.id));members=fresh;
      if(!$('profilesview').hidden) renderPeople();
    }
    if(kinds.has('refresh') || kinds.has('topics') || kinds.has('sections')) {
      if(!$('forumview').hidden) {
        await loadTopics();
        if(openThread) {
          const thread=openThread;
          try {await loadThread(thread,true);}
          catch(error) {if(error.message==='Discussion not found' && openThread===thread){closeThread();$('livenotice').textContent='This discussion is no longer available.';}else throw error;}
        }
      }
    }
    if(kinds.has('refresh') || kinds.has('library')) {
      if(!$('libraryview').hidden) await loadLibrary();
      if(currentUser?.admin && !$('moderation').hidden && typeof loadModReviews==='function'){await loadModReviews();await loadAdminOverview();}
    }
  } catch(error) { $('liveconnection').textContent='Live refresh interrupted. Reconnecting…'; }
  finally {liveRunning=false;if(livePending.size) liveTimer=setTimeout(refreshLiveViews,100);}
}
function queueLive(kind) {
  livePending.add(kind);clearTimeout(liveTimer);liveTimer=setTimeout(refreshLiveViews,100);
}
function stopLive() {liveStopped=true;clearTimeout(liveTimer);liveSource?.close();}
document.addEventListener('DOMContentLoaded',()=>{
  liveSource=new EventSource('/api/v1/events');
  liveSource.addEventListener('ready',()=>{
    $('liveconnection').textContent='● Live updates connected';
    if(liveReady) $('livenotice').textContent='Reconnected. Catching up with the community…';
    liveReady=true;queueLive('refresh');
  });
  liveSource.addEventListener('change',event=>{
    const change=JSON.parse(event.data);
    if(change.kind==='topics') $('livenotice').textContent=change.action==='created' ? 'A new discussion was posted.' : 'A discussion was updated.';
    if(change.kind==='sections') $('livenotice').textContent='Forum sections were updated.';
    if(change.kind==='library') $('livenotice').textContent='The mod library was updated.';
    queueLive(change.kind);
  });
  liveSource.addEventListener('auth-expired',()=>{stopLive();location.replace(location.pathname);});
  liveSource.onerror=async()=>{
    $('liveconnection').textContent='Live updates reconnecting…';
    try {await api('me');} catch(error) { /* api redirects expired sessions; EventSource retries temporary failures. */ }
  };
});
window.addEventListener('pagehide',stopLive);
document.addEventListener('visibilitychange',()=>{if(!document.hidden && !liveStopped) queueLive('refresh');});
