'use strict';
const $ = id => document.getElementById(id);
sessionStorage.removeItem('canna-session');
let currentUser;
function adoptCurrentUser(user){
  if(currentUser&&currentUser.id!==user.id){
    pageWarm.clear();navigationController?.abort();location.reload();
    const error=new Error('Account changed. Reloading this page.');error.name='AbortError';throw error;
  }
  currentUser=user;
}
let communityPageReady=false;
let externalLanding=new URLSearchParams(location.search).has("import");
let devicesLanding=new URLSearchParams(location.search).has("devices");
const message = text => { $('message').textContent = text; };
async function api(path, options = {}) {
  const headers = new Headers(options.headers || {});
  const method=options.method||'GET';
  if(method!=='GET')pageWarm.clear();
  const member=currentUser?.id;
  const warm=method==='GET' && path!=='me' ? pageWarm.get(path) : null;
  const response=warm && warm.member===member && warm.until>Date.now() ? await warm.promise.then(r=>r.clone()).catch(()=>fetch(`/api/v1/${path}`, {...options, headers})) : await fetch(`/api/v1/${path}`, {...options, headers});
  if(warm&&warm.member===member&&member!==currentUser?.id){
    const error=new Error('Account changed. Reload this page.');error.name='AbortError';throw error;
  }
  if (!response.ok) {
    if(response.status===401) location.replace(location.pathname);
    const error = await response.json().catch(() => ({}));
    const failure = new Error(error.error || `Request failed (${response.status})`);
    failure.status=response.status;
    throw failure;
  }
  return response;
}
async function json(path, data, options = {}) {
  const headers=new Headers(options.headers||{});headers.set('Content-Type','application/json');
  return (await api(path, {...options,method:'POST',headers,body:JSON.stringify(data)})).json();
}
async function action(callback) {
  message('');
  try { await callback(); } catch (error) { message(error.message); }
}
async function download(path, filename) {
  const response = await api(path);
  const url = URL.createObjectURL(await response.blob());
  const link = document.createElement('a'); link.href = url; link.download = filename; link.click();
  setTimeout(() => URL.revokeObjectURL(url), 30000);
}
function openModDetails(item) {
  const dialog=document.createElement('dialog');dialog.className='moddetaildialog';
  const close=document.createElement('button');close.textContent='Close';close.addEventListener('click',()=>dialog.close());
  const title=document.createElement('h2');title.textContent=item.name;
  const meta=document.createElement('p');meta.textContent=`${item.version || 'Modpack'} · ${libraryGameName(item)} · ${item.details?.authors || 'Creator not recorded'}`;
  const description=document.createElement('p');description.className='moddescription';description.textContent=item.description || 'No description provided.';
  dialog.append(close,title,meta,description);
  for(const [label,key] of [['Minecraft versions','game_versions'],['Loaders','loaders'],['Required dependencies','dependencies']]) {
    const values=item.details?.[key];if(values?.length){const line=document.createElement('p');line.textContent=label+': '+values.join(', ');dialog.append(line);}
  }
  if(item.details?.install_notes){const notes=document.createElement('p');notes.textContent=item.details.install_notes;dialog.append(notes);}
  for(const author of item.details?.author_links || []) {if(!/^https:\/\//.test(author.url))continue;const link=document.createElement('a');link.href=author.url;link.textContent=author.name;link.target='_blank';link.rel='noopener noreferrer';dialog.append(link,document.createTextNode(' '));}
  if(/^https:\/\//.test(item.details?.source_url || '')){const source=document.createElement('p'),link=document.createElement('a');link.href=item.details.source_url;link.textContent='Original project';link.target='_blank';link.rel='noopener noreferrer';source.append(link);dialog.append(source);}
  dialog.addEventListener('close',()=>dialog.remove(),{once:true});document.body.append(dialog);dialog.showModal();
}
function entry(item, kind) {
  const row = document.createElement('div'); row.className = 'entry';
  const info = document.createElement('div');
  const external=!!item.details?.external_only;
  if(item.details?.icon_data) {const img=document.createElement('img');img.className='modart';img.src='data:image/jpeg;base64,'+item.details.icon_data;img.alt=item.name+' original artwork';img.loading='lazy';row.append(img);}
  else if(item.details?.icon_url && /^https:\/\/(cdn\.thunderstore\.io|ccdn\.thunderstore\.io|gcdn\.thunderstore\.io|cdn\.modrinth\.com|media\.forgecdn\.net|images\.steamusercontent\.com)\//.test(item.details.icon_url)){const img=document.createElement('img');img.className='modart';img.src=item.details.icon_url;img.alt=item.name+' original artwork';img.loading='lazy';img.referrerPolicy='no-referrer';row.append(img);}
  const title = document.createElement('strong'); title.textContent = item.name;
  const detail = document.createElement('p'); detail.textContent = `${item.version || 'Modpack'} · ${libraryGameName(item)} · ${item.details?.authors || (kind==='packs'?item.author:'Creator not recorded')}`;
  info.append(title, detail);if(kind==='mods' && item.uploader) {const uploader=document.createElement('small');uploader.textContent='Added by '+item.uploader;info.append(uploader);}
  if(item.details?.author_links?.length) {const authors=document.createElement('p');authors.append('By ');for(const author of item.details.author_links){const link=document.createElement('a');link.textContent=author.name;link.href=author.url;link.target='_blank';link.rel='noopener noreferrer';authors.append(link,' ');}info.append(authors);}
  if(item.description) {const description=document.createElement('p');description.className='moddescription modpreview';description.textContent=item.description;info.append(description);}
  const more=document.createElement('button');more.textContent=kind==='packs'?'View shared pack':'Show more';more.addEventListener('click',()=>kind==='packs'?navigatePage(`/packs/${item.id}`):openModDetails(item));info.append(more);
  if(item.details?.dependencies?.length) {const deps=document.createElement('p');deps.textContent='Required: '+item.details.dependencies.join(', ');info.append(deps);}
  if(item.details?.install_notes){const notes=document.createElement('p');notes.textContent=item.details.install_notes;info.append(notes);}
  if(item.details?.source_url) {const source=document.createElement('a');source.textContent='Original project · '+(item.details.provider || 'Family catalog');source.href=item.details.source_url;source.target='_blank';source.rel='noopener noreferrer';info.append(source);}
  row.append(info);
  const actions = document.createElement('div'); actions.className = 'row';
  if (!external && kind === 'mods' && currentUser.admin) {
    const source = document.createElement('button'); source.textContent = 'View source';
    source.addEventListener('click', () => action(() => viewSource(item.id))); actions.append(source);
  }
  const pending = kind === 'mods' && item.review_status === 'pending';
  if(pending) {const note=document.createElement('p');note.textContent='Awaiting administrator review';info.append(note);if(currentUser.admin){const approve=document.createElement('button');approve.textContent='Approve mod';approve.addEventListener('click',()=>action(async()=>{if(!await cannaConfirm(`Approve ${item.name} for community downloads? Review the archive and its author first. Approval does not certify it free of malware.`))return;await api(`mods/${item.id}/approve`,{method:'POST'});await refresh();}));actions.append(approve);}}
  const button = document.createElement('button'); button.textContent = external?'Subscribe on Steam Workshop':'Download';button.disabled=pending;
  button.addEventListener('click', () => external?window.open(item.details.source_url,'_blank','noopener,noreferrer'):action(() => downloadToApp(item,kind))); actions.append(button);
  if(external){const note=document.createElement('p');note.textContent='Official-site download. Workshop subscriptions are managed by Steam, separately from Canna modpacks.';info.append(note);}
  if (kind === 'packs') {
    const copy = document.createElement('button'); copy.textContent = 'Copy link';
    copy.addEventListener('click', () => action(async () => { await navigator.clipboard.writeText(`https://cannamods.vip/packs/${item.id}`); message('Share link copied. Your family will need to sign in.'); })); actions.append(copy);
  }
  if (!external && (currentUser.admin || currentUser.username === (kind==='mods'?item.uploader:item.author))) {
    const remove = document.createElement('button'); remove.textContent = 'Delete';
    remove.addEventListener('click', () => action(async () => {
      if (!await cannaConfirm(`Delete ${item.name}?`)) return;
      await api(`${kind}/${item.id}`, {method:'DELETE'}); await refresh();
    })); actions.append(remove);
  }
  row.append(actions); return row;
}
const libraryItems={mods:[],packs:[]};
async function loadLibrary() {
  if(!currentUser) adoptCurrentUser(await (await api('me')).json());
  const catalog=await (await api('catalog')).json();
  librarySupportedGames.clear();librarySupportedGames.set('minecraft','Minecraft');
  for(const game of catalog.games) librarySupportedGames.set(game.app_id===4294967295?'minecraft':String(game.app_id),game.name);
  for (const kind of ['mods','packs']) {
    const items = await (await api(kind)).json();
    libraryItems[kind]=items;
  }
  renderLibrary();
  if(typeof loadUpdateStatus==='function')await loadUpdateStatus();
}
async function refresh() {
  adoptCurrentUser(await (await api('me')).json());
  $('admin').hidden=currentUser.role!=='owner';$('adminnav').hidden=!currentUser.admin;
  $('newinvite').disabled=!currentUser.can_invite;
  $('betainvitelabel').hidden=!(currentUser.role==='owner'||currentUser.can_rebound);
  if($('betainvitelabel').hidden)$('betainvite').checked=false;
  $('newwave').disabled=!currentUser.can_invite;
  $('allowance').textContent=currentUser.role==='owner'?(currentUser.can_invite?'Create individual invites or an invite wave. New members get one friend invite.':'Invitation creation is paused. Resume it in Admin panel → Invitations.'):currentUser.role==='admin'?'Admins cannot issue invites. The Owner manages invitation waves.':`${currentUser.invites_remaining} friend invitation remaining. Each code works once and expires after seven days.`;
  renderAccountBadges();
  if(!communityPageReady){await openCommunityPage();communityPageReady=true;}
  else if(!$('libraryview').hidden)await loadLibrary();
  else if(!$('moderation').hidden && currentUser.admin)await loadAdmin();
  else if(!$('gamblingview').hidden)await loadGambling();
  updateNavigation();
  $('space').removeAttribute('data-booting');$('bootstatus').hidden=true;
  setupPagePrefetch();
  if(externalLanding){externalLanding=false;await showView("libraryview",true);openExternalImport();}
  if(devicesLanding){devicesLanding=false;await openProfile(currentUser.id);$('loggeddevices').scrollIntoView({block:'start'});}
}
function memberRoleLabel(member){
  const primary=String(member.role||'member');
  const roles=[primary,...(Array.isArray(member.roles)?member.roles:[])];
  return [...new Set(roles.filter(role=>['owner','admin','vip','member','beta'].includes(role)))].map(role=>role.toUpperCase()).join(' · ');
}
function renderAccountBadges(){
  $('adminnav').hidden=!currentUser.admin;
  $('rolebadge').textContent=memberRoleLabel(currentUser);$('welcome').textContent=currentUser.username;$('kashbalance').textContent=(currentUser.kash||0).toLocaleString()+' Kash';$('sideusername').textContent=currentUser.username;$('siderole').textContent=`${memberRoleLabel(currentUser)} · Canna community`;
}
$('logout').addEventListener('click',()=>action(async()=>{await api('logout',{method:'POST'});location.assign('/');}));
$('forgetdevices').addEventListener('click',()=>action(async()=>{if(!await cannaConfirm('Forget all trusted devices and sign out everywhere?'))return;await api('trusted-devices',{method:'DELETE'});location.assign('/');}));
async function inviteAction(id,callback) {
  const btn=$(id);btn.disabled=true;$('invitestatus').textContent='Generating invitations…';
  try {await callback();$('inviteout').focus();}catch(error){$('invitestatus').textContent=error.message;message(error.message);}finally {btn.disabled=!currentUser.can_invite;}
}
function invitationLink(code){
 if(typeof code!=='string'||!/^[A-Za-z0-9_-]{1,64}$/.test(code))throw new Error('Invalid invitation code.');
 return new URL('/#invite='+encodeURIComponent(code),location.origin).href;
}
function renderInvitationOutputs(codes){
 $('inviteout').value=codes.join('\n');$('invitelinks').value=codes.map(invitationLink).join('\n');
 $('copyinvitelinks').disabled=$('copyinvitecodes').disabled=!codes.length;
}
$('copyinvitelinks').addEventListener('click',()=>action(async()=>{await navigator.clipboard.writeText($('invitelinks').value);message('Invitation links copied. Give one link to each person.');}));
$('copyinvitecodes').addEventListener('click',()=>action(async()=>{await navigator.clipboard.writeText($('inviteout').value);message('Invitation codes copied. Give one code to each person.');}));
$('newinvite').addEventListener('click',()=>inviteAction('newinvite',async()=>{
  const result=await json('invites',{beta:$('betainvite').checked});renderInvitationOutputs([result.invite]);$('waveid').value='';$('revokewave').disabled=true;await refresh();if(typeof loadInvitationHistory==='function')await loadInvitationHistory();$('invitestatus').textContent='Invitation created. Share its link or code with one person. Both expire together in seven days.';
}));
$('newwave').addEventListener('click',()=>inviteAction('newwave',async()=>{
  const count=Number($('wavecount').value);if(!Number.isInteger(count)||count<1||count>50)throw new Error('Choose 1–50 invitations.');
  const wave=await json('invite-waves',{count,mode:$('invitemode')?.value||'wave',label:$('invitelabel')?.value||'',beta:$('wavebeta')?.checked||false});renderInvitationOutputs(wave.invites);$('waveid').value=wave.wave;$('revokewave').disabled=!wave.wave;if(typeof loadInvitationHistory==='function')await loadInvitationHistory();$('invitestatus').textContent=wave.delivered_to?`Delivered ${wave.invites.length} Beta invite links privately to ${wave.delivered_to} Beta members. They expire in seven days.`:`${wave.beta?'Beta-enabled':'Standard'} invitations created and saved in history. Share one link per person; expires in seven days.`;
}));
$('revokewave').addEventListener('click',()=>action(async()=>{const result=await(await api(`invite-waves/${$('waveid').value}`,{method:'DELETE'})).json();renderInvitationOutputs([]);$('revokewave').disabled=true;message(`${result.revoked} unused invitations revoked. Existing accounts remain active.`);}));
$('upload').addEventListener('submit',event=>{event.preventDefault();action(async()=>{
  const file=$('modfile').files[0];if(!file||file.size>128*1024*1024)throw new Error('Choose a ZIP no larger than 128 MiB.');
  const query=new URLSearchParams({app_id:$('game').value,name:$('modname').value,version:$('version').value,description:$('description').value});message('Uploading mod…');const result=await(await api(`mods?${query}`,{method:'POST',headers:{'Content-Type':'application/zip'},body:file})).json();$('upload').reset();await refresh();message('Mod uploaded. Automatic analysis runs first; a staff member must manually approve this upload. Check My submissions for its status.');
});});
$('share').addEventListener('submit',event=>{event.preventDefault();action(async()=>{
  const file=$('packfile').files[0];if(!file||file.size>2*1024*1024)throw new Error('Choose a Canna modpack export no larger than 2 MiB.');const result=await json('packs',JSON.parse(await file.text()));await refresh();message(`Share link: ${result.url}`);
});});
document.addEventListener('DOMContentLoaded',()=>{refresh().catch(error=>{$('bootstatus').textContent='Unable to load this page. Reload to try again.';message(error.message);});});
window.addEventListener('pageshow',event=>{if(event.persisted)location.reload();});

// Only the displayed page is rendered; searches run across the server's full list.
const listPages=new Map();
async function pagedList(path,id,reload,searchId,onPage) {
 let state=listPages.get(id);
 if(!state){
  state={page:1,serial:0};listPages.set(id,state);
  const controls=document.createElement('div');controls.className='pagination';
  const search=searchId ? $(searchId) : document.createElement('input');
  if(!searchId){search.placeholder='Search this list…';search.setAttribute('aria-label','Search this list');controls.append(search);}
  search.maxLength=100;state.search=search;
  search.addEventListener('input',()=>{clearTimeout(state.timer);state.page=1;state.timer=setTimeout(()=>action(reload),250);});
  state.previous=button('← Previous',()=>{state.page=Math.max(1,state.page-1);return reload();});
  state.next=button('Next →',()=>{state.page++;return reload();});
  state.label=document.createElement('span');state.label.setAttribute('role','status');
  controls.append(state.previous,state.label,state.next);$(id).after(controls);
 }
 const serial=++state.serial;
 const result=await(await api(path+'?'+new URLSearchParams({page:state.page,search:state.search.value||''}))).json();
 if(serial!==state.serial)throw new Error('List changed. Loading the latest search…');
 if(onPage)onPage(result);
 const rows=Array.isArray(result)?result:result.items;
 const total=Array.isArray(result)?rows.length:result.total;
 state.previous.disabled=state.page<=1;state.next.disabled=state.page*50>=total;
 state.label.textContent=total ? `${(state.page-1)*50+1}–${Math.min(state.page*50,total)} of ${total.toLocaleString()}` : 'No results';
 return rows;
}


let prefetchReady=false;
const pageWarm=new Map();
function pageReads(path){
 const url=new URL(path,location.origin),p=url.pathname;
 if(p==='/play')return ['play?page=1&channel='];
 if(p==='/members')return ['profiles?page=1&search='];
 if(/^\/members\/\d+$/.test(p))return ['profiles/'+p.split('/')[2]];
 if(p==='/mods')return ['providers/games'];
 if(p==='/library')return ['mods','packs'];
 if(p==='/subscriptions')return ['mods/subscriptions?page=1&q='];
 if(p==='/admin' && currentUser.admin)return ['admin/overview'];
 if(p==='/notifications')return ['notifications'];
 if(p==='/submissions')return ['submissions'];
 if(p.startsWith('/forums')){const section=p.startsWith('/forums/sections/')?decodeURIComponent(p.slice(17)):'';return ['sections',`topics?offset=0&category=${encodeURIComponent(section)}`,...(p.startsWith('/forums/topics/')?['topics/'+p.slice(15)]:[])];}
 return [];
}
function warmPage(path){
 for(const key of pageReads(path)){
  if(pageWarm.get(key)?.member===currentUser?.id&&pageWarm.get(key)?.until>Date.now())continue;
  if(pageWarm.size>=16)pageWarm.delete(pageWarm.keys().next().value);
  const item={member:currentUser?.id,until:Date.now()+5000,promise:fetch('/api/v1/'+key).then(r=>{if(!r.ok&&pageWarm.get(key)===item)pageWarm.delete(key);return r;})};
  pageWarm.set(key,item);item.promise.catch(()=>{if(pageWarm.get(key)===item)pageWarm.delete(key);});
 }
}
let navigating=false,navigationGeneration=0,navigationController=null,navigationPending=null,navigationPromise=null;
function navigationSignal(){return navigationController?.signal;}
async function navigatePage(path,back=false){
 navigationPending={path,back,generation:++navigationGeneration};navigationController?.abort();
 if(navigating)return navigationPromise;
 navigating=true;navigationPromise=drainNavigationRequests();
 try{return await navigationPromise;}
 finally{
  navigating=false;navigationPromise=null;
  if(navigationPending){const pending=navigationPending;navigationPending=null;await navigatePage(pending.path,pending.back);}
 }
}
async function drainNavigationRequests(){
 while(navigationPending){
 const {path,back,generation}=navigationPending;navigationPending=null;
 const controller=new AbortController();navigationController=controller;
 try{
  // Revalidate the session even when route data was warmed on hover.
  const user=await(await api('me',{signal:controller.signal})).json();
  if(generation!==navigationGeneration||controller.signal.aborted)continue;
  adoptCurrentUser(user);
  renderAccountBadges();
  if(!back)history.pushState(null,'',path);
  openThread='';threadData=undefined;topicPage=0;profileId=0;
  $('topicfilter').value='';$('thread').hidden=true;$('newtopic').hidden=true;$('profilecard').hidden=true;
  $('composetopic').hidden=false;$('forumheading').textContent='Forums';$('forumdescription').textContent='Ask questions. Share your work. Help each other build.';
  await openCommunityPage();if(generation===navigationGeneration&&!controller.signal.aborted)window.scrollTo({top:0});
 }catch(error){if(error.name!=='AbortError'&&generation===navigationGeneration)throw error;}
 }
}
function setupPagePrefetch(){
 const paths={playnav:'/play',forumnav:'/forums',browsenav:'/mods',gamblingnav:'/gambling',subscriptionsnav:'/subscriptions',librarynav:'/library',peoplenav:'/members',adminnav:'/admin',submissionsnav:'/submissions',notificationsnav:'/notifications',myprofilenav:'/members/'+currentUser.id,welcome:'/members/'+currentUser.id,forumback:'/forums',latestdiscussions:'/forums/latest'};
 for(const [id,path] of Object.entries(paths))$(id).dataset.page=path;
 if(prefetchReady)return;prefetchReady=true;
 const intent=event=>{
  const node=event.target.closest('a[href],button[data-page]');if(!node || navigator.connection?.saveData)return;
  const path=node.dataset.page || node.getAttribute('href');
  if(!path || !/^\/(forums(?:\/|$)|members(?:\/|$)|mods$|library$|subscriptions$|admin$|submissions$|notifications$|help$|support$|review\/mods\/)/.test(path) || path.includes('?'))return;
  clearTimeout(node._prefetchTimer);
  node._prefetchTimer=setTimeout(()=>{
   if(pageReads(path).length){warmPage(path);return;}
   const link=document.createElement('link');link.rel='prefetch';link.as='document';link.href=path;document.head.append(link);setTimeout(()=>link.remove(),10000);
  },120);
 };
 document.addEventListener('pointerover',intent);document.addEventListener('focusin',intent);
 document.addEventListener('pointerout',event=>{const node=event.target.closest('a[href],button[data-page]');if(node)clearTimeout(node._prefetchTimer);});
 window.addEventListener('popstate',()=>action(()=>navigatePage(location.pathname+location.search,true)));
}

$('refreshmyinvites').addEventListener('click',()=>action(async()=>{
 const data=await(await api('my-invites')).json();
 $('myinvites').replaceChildren(...data.items.map(item=>{
  const card=document.createElement('article');card.className='entry';
  const label=document.createElement('span');label.textContent=`${item.beta?'Beta-enabled':'Standard'} invite · expires ${new Date(item.expires*1000).toLocaleString()}`;
  card.append(label,button('Copy invite link',async()=>{await navigator.clipboard.writeText(invitationLink(item.code));message('Invitation link copied.');}));return card;
 }));if(!data.items.length)$('myinvites').textContent='No active invitations. Expired or revoked links disappear here.';
}));
