'use strict';
const $ = id => document.getElementById(id);
sessionStorage.removeItem('canna-session');
let currentUser;
let externalLanding=new URLSearchParams(location.search).has("import");
let devicesLanding=new URLSearchParams(location.search).has("devices");
const message = text => { $('message').textContent = text; };
async function api(path, options = {}) {
  const headers = new Headers(options.headers || {});
  const response = await fetch(`/api/v1/${path}`, {...options, headers});
  if (!response.ok) {
    if(response.status===401) location.replace(location.pathname);
    const error = await response.json().catch(() => ({}));
    throw new Error(error.error || `Request failed (${response.status})`);
  }
  return response;
}
async function json(path, data) {
  return (await api(path, {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(data)})).json();
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
function entry(item, kind) {
  const row = document.createElement('div'); row.className = 'entry';
  const info = document.createElement('div');
  const title = document.createElement('strong'); title.textContent = item.name;
  const detail = document.createElement('p'); detail.textContent = `${item.version || 'Modpack'} · ${libraryGameName(item)} · ${item.details?.authors || item.author}`;
  info.append(title, detail);
  if(item.description) { const description=document.createElement('p'); description.className='moddescription'; description.textContent=item.description;info.append(description); }
  if(item.details?.source_url) {const source=document.createElement('a');source.textContent='Original project · '+(item.details.provider || 'Family catalog');source.href=item.details.source_url;source.target='_blank';source.rel='noopener noreferrer';info.append(source);}
  row.append(info);
  const actions = document.createElement('div'); actions.className = 'row';
  if (kind === 'mods' && currentUser.admin) {
    const source = document.createElement('button'); source.textContent = 'View source';
    source.addEventListener('click', () => action(() => viewSource(item.id))); actions.append(source);
  }
  const pending = kind === 'mods' && item.review_status === 'pending';
  if(pending) {const note=document.createElement('p');note.textContent='Awaiting administrator review';info.append(note);if(currentUser.admin){const approve=document.createElement('button');approve.textContent='Approve mod';approve.addEventListener('click',()=>action(async()=>{if(!confirm(`Approve ${item.name} for community downloads? Review the archive and its author first. Approval does not certify it free of malware.`))return;await api(`mods/${item.id}/approve`,{method:'POST'});await refresh();}));actions.append(approve);}}
  const button = document.createElement('button'); button.textContent = 'Download';button.disabled=pending;
  button.addEventListener('click', () => action(() => downloadToApp(item,kind))); actions.append(button);
  if (kind === 'packs') {
    const copy = document.createElement('button'); copy.textContent = 'Copy link';
    copy.addEventListener('click', () => action(async () => { await navigator.clipboard.writeText(`https://cannamods.vip/packs/${item.id}`); message('Share link copied. Your family will need to sign in.'); })); actions.append(copy);
  }
  if (currentUser.admin || currentUser.username === item.author) {
    const remove = document.createElement('button'); remove.textContent = 'Delete';
    remove.addEventListener('click', () => action(async () => {
      if (!confirm(`Delete ${item.name}?`)) return;
      await api(`${kind}/${item.id}`, {method:'DELETE'}); await refresh();
    })); actions.append(remove);
  }
  row.append(actions); return row;
}
const libraryItems={mods:[],packs:[]};
async function loadLibrary() {
  if(!currentUser) currentUser=await (await api('me')).json();
  for (const kind of ['mods','packs']) {
    const items = await (await api(kind)).json();
    libraryItems[kind]=items;
  }
  renderLibrary();
}
async function refresh() {
  currentUser=await (await api('me')).json();
  $('admin').hidden=currentUser.role!=='owner';$('adminnav').hidden=!currentUser.admin;
  $('newinvite').disabled=!currentUser.can_invite;
  $('allowance').textContent=currentUser.role==='owner'?'Create individual invites or an invite wave. New members get one friend invite.':currentUser.role==='admin'?'Admins cannot issue invites. The Owner manages invitation waves.':`${currentUser.invites_remaining} friend invitation remaining. Each code works once and expires after seven days.`;
  $('rolebadge').textContent=currentUser.role.toUpperCase();$('welcome').textContent=currentUser.username;$('sideusername').textContent=currentUser.username;$('siderole').textContent=`${currentUser.role.toUpperCase()} · Canna community`;
  await loadLibrary();if(!$('forumview').hidden) await loadTopics();if(!$('moderation').hidden && currentUser.admin) await loadAdmin();updateNavigation();
  if(externalLanding){externalLanding=false;await showView("libraryview");openExternalImport();}
  if(devicesLanding){devicesLanding=false;await openProfile(currentUser.id);$('loggeddevices').scrollIntoView({block:'start'});}
}
$('logout').addEventListener('click',()=>action(async()=>{await api('logout',{method:'POST'});location.assign('/');}));
$('forgetdevices').addEventListener('click',()=>action(async()=>{if(!confirm('Forget all trusted devices and sign out everywhere?'))return;await api('trusted-devices',{method:'DELETE'});location.assign('/');}));
async function inviteAction(id,callback) {
  const btn=$(id);btn.disabled=true;$('invitestatus').textContent='Generating invitations…';
  try {await callback();$('inviteout').focus();}catch(error){$('invitestatus').textContent=error.message;message(error.message);}finally {btn.disabled=id==='newinvite' && !currentUser.can_invite;}
}
$('newinvite').addEventListener('click',()=>inviteAction('newinvite',async()=>{
  const result=await json('invites',{});$('inviteout').value=result.invite;$('waveid').value='';$('revokewave').disabled=true;await refresh();$('invitestatus').textContent='Invitation created. Copy the code below and give it to one person. It expires in seven days.';
}));
$('newwave').addEventListener('click',()=>inviteAction('newwave',async()=>{
  const count=Number($('wavecount').value);if(!Number.isInteger(count)||count<1||count>50)throw new Error('Choose 1–50 invitations.');
  const wave=await json('invite-waves',{count});$('inviteout').value=wave.invites.join('\n');$('waveid').value=wave.wave;$('revokewave').disabled=false;$('invitestatus').textContent='Invite wave created. Copy the codes below, one for each person. They expire in seven days.';
}));
$('revokewave').addEventListener('click',()=>action(async()=>{const result=await(await api(`invite-waves/${$('waveid').value}`,{method:'DELETE'})).json();$('inviteout').value='';$('revokewave').disabled=true;message(`${result.revoked} unused invitations revoked. Existing accounts remain active.`);}));
$('upload').addEventListener('submit',event=>{event.preventDefault();action(async()=>{
  const file=$('modfile').files[0];if(!file||file.size>128*1024*1024)throw new Error('Choose a ZIP no larger than 128 MiB.');
  const query=new URLSearchParams({app_id:$('game').value,name:$('modname').value,version:$('version').value,description:$('description').value});message('Uploading mod…');await api(`mods?${query}`,{method:'POST',headers:{'Content-Type':'application/zip'},body:file});$('upload').reset();await refresh();message('Mod uploaded. An administrator must approve it before downloads are enabled.');
});});
$('share').addEventListener('submit',event=>{event.preventDefault();action(async()=>{
  const file=$('packfile').files[0];if(!file||file.size>2*1024*1024)throw new Error('Choose a Canna modpack export no larger than 2 MiB.');const result=await json('packs',JSON.parse(await file.text()));await refresh();message(`Share link: ${result.url}`);
});});
document.addEventListener('DOMContentLoaded',()=>{refresh().catch(error=>message(error.message));});
if(location.pathname.startsWith('/packs/')) {
  const id=location.pathname.split('/')[2];const button=document.createElement('button');button.className='primary';button.textContent='Download shared pack';button.addEventListener('click',()=>action(()=>downloadToApp({id,name:'Shared modpack'},'packs')));$('forumview').hidden=true;$('libraryview').hidden=false;$('libraryview').prepend(button);
}
window.addEventListener('pageshow',event=>{if(event.persisted)location.reload();});
