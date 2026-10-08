'use strict';
let members = [], profileId = 0,profileRequestSerial=0;
async function loadPeople() {
  if(location.pathname!=='/members'){return navigatePage('/members');}
  const serial=++profileRequestSerial,signal=typeof navigationSignal==='function'?navigationSignal():undefined;
  profileId = 0; $('profilecard').hidden = true;
  await showView('profilesview',true);
  if(serial!==profileRequestSerial||location.pathname!=='/members'||signal?.aborted)return;
  const result=await pagedList('profiles','people',loadPeople,'membersearch');
  if(serial!==profileRequestSerial||location.pathname!=='/members'||signal?.aborted)return;
  members=result;renderPeople();
}
function renderPeople() {
  const filtered = members.filter(member => member.username.toLowerCase().includes($('membersearch').value.toLowerCase()));
  $('people').replaceChildren(...filtered.map(member => {
    const row = document.createElement('div'); row.className = 'membercard';
    if (member.avatar) { const image = document.createElement('img'); image.className = 'avatar'; image.src = `/api/v1/profiles/${member.id}/avatar`; image.width = image.height = 48; image.alt = ''; image.loading='lazy'; row.append(image); }
    const info = document.createElement('div'); const title = document.createElement('strong'); title.textContent = member.username;
    const status = document.createElement('p'); status.textContent = `${memberRoleLabel(member)}${member.status ? ` · ${member.status}` : ''}`;
    info.append(title,status);const link=button('View profile',() => openProfile(member.id));link.dataset.page='/members/'+member.id;row.append(info,link); return row;
  }));
  if (!filtered.length) $('people').textContent = 'No members found.';
}
$('peoplenav').addEventListener('click',() => action(loadPeople));
$('myprofilenav').addEventListener('click',() => action(() => openProfile(currentUser.id)));

async function openProfile(id) {
  if(location.pathname!==`/members/${id}`){return navigatePage(`/members/${id}`);}
  const serial=++profileRequestSerial,route=`/members/${id}`,signal=typeof navigationSignal==='function'?navigationSignal():undefined;
  const current=()=>serial===profileRequestSerial&&location.pathname===route&&!signal?.aborted;
  const profile = await (await api(`profiles/${id}`,{signal})).json();
  if(!current())return;profileId = id;
  $('people').replaceChildren();
  await showView('profilesview',true);if(!current())return; $('profilecard').hidden = false;
  $('profilename').textContent = profile.username;
  $('profilebadges').textContent = `${memberRoleLabel(profile)} · ${profile.rank} · ${profile.points} XP${profile.banned ? ' · Banned' : ''}`;
  $('profilestatus').textContent = profile.status; $('profilebio').textContent = profile.bio;
  $('profileavatar').hidden = !profile.avatar;
  if (profile.avatar) $('profileavatar').src = `/api/v1/profiles/${id}/avatar?t=${Date.now()}`;
  applyProfileCosmetics(profile);
  $('profilerep').textContent = profile.ratings_count ? `★ ${profile.stars.toFixed(1)} / 5 · ${profile.ratings_count} rating${profile.ratings_count === 1 ? '' : 's'} · ${profile.posts_count} forum posts` : `No ratings yet · ${profile.posts_count} forum posts`;
  const own = id === currentUser.id;
  renderProfileCosmeticActions(own,profile);
  $('editprofile').hidden = $('avatarform').hidden = !own;
  $('loggeddevices').hidden=!own;
  if(own)await loadDevices();
  if(!current())return;
  $('ratingform').hidden = own || profile.banned; $('profilecommentform').hidden = profile.banned;
  $('editstatus').value = profile.status; $('editbio').value = profile.bio;
  $('ratingstars').value = String(profile.my_rating || 5);
  $('profilecomments').replaceChildren(...profile.comments.map(comment => {
    const row = document.createElement('article'); row.className = 'card';
    const name = button(comment.author,() => openProfile(comment.author_id),'/members/'+comment.author_id);
    const date = document.createElement('small'); date.textContent = ` ${new Date(comment.created*1000).toLocaleString()}`;
    const body = document.createElement('p'); body.className = 'postbody'; body.textContent = comment.body;
    row.append(name,date,body);
    if (own || currentUser.admin || comment.author_id === currentUser.id) row.append(button('Remove',async () => {
      if (!await cannaConfirm('Remove this profile comment?')) return;
      await api(`profile-comments/${comment.id}`,{method:'DELETE'}); await openProfile(id);
    }));
    return row;
  }));
  if (!profile.comments.length) $('profilecomments').textContent = 'No comments yet. Say hello.';
  $('profilecard').scrollIntoView({behavior:'smooth',block:'start'});
}
function applyProfileCosmetics(profile){
 const card=$('profilecard');let banner=$('profilebanner');if(!banner){banner=document.createElement('div');banner.id='profilebanner';banner.className='profilebanner';card.prepend(banner);}
 const item=profile.cosmetics?.banner?.paused?null:profile.cosmetics?.banner;banner.className='profilebanner';banner.replaceChildren();banner.hidden=!item;
 if(item){banner.setAttribute('aria-label',item.name+(item.animated?' animated':'')+' profile banner');banner.setAttribute('role','img');banner.title=item.name;const image=profileCosmeticImage(item);if(image){if(item.collection==='mw2'){banner.classList.add('mw2-banner');const width=callingCardDisplayWidth(item);if(width)image.style.width=width+'px';}banner.append(image);}else{if(/^[a-z0-9_-]{1,80}$/.test(item.style||item.id||''))banner.classList.add('cosmetic-'+(item.style||item.id));}}
 const avatar=$('profileavatar');let portrait=$('profileportrait');if(!portrait){portrait=document.createElement('div');portrait.id='profileportrait';portrait.className='profileportrait';avatar.before(portrait);portrait.append(avatar);const initial=document.createElement('span');initial.id='profileinitial';portrait.append(initial);}
 $('profileinitial').textContent=profile.username.slice(0,1).toUpperCase();$('profileinitial').hidden=!!profile.avatar;
 let frame=$('profileframe');if(!frame){frame=document.createElement('div');frame.id='profileframe';portrait.append(frame);}frame.className='profileframe';frame.replaceChildren();frame.hidden=!profile.cosmetics?.frame;
 if(profile.cosmetics?.frame){const cosmetic=profile.cosmetics.frame;frame.setAttribute('aria-label',cosmetic.name+' avatar frame');const image=profileCosmeticImage(cosmetic);if(image)frame.append(image);else{frame.className='profileframe';if(/^[a-z0-9_-]{1,80}$/.test(cosmetic.style||cosmetic.id||''))frame.classList.add('cosmetic-'+(cosmetic.style||cosmetic.id));}}
}
function renderProfileCosmeticActions(own,profile){
 let tools=$('profilecosmetictools');if(!tools){tools=document.createElement('div');tools.id='profilecosmetictools';tools.className='row profilecosmetictools';const manage=button('Manage avatar frame & banner',async()=>{await navigatePage('/gambling');if(location.pathname==='/gambling')selectGamblingTab('collection');});manage.id='profile-cosmetic-manage';manage.type='button';manage.dataset.page='/gambling';tools.append(manage,artworkMotionControl());$('profilebio').before(tools);}
 const animated=!!profile?.cosmetics?.banner?.animated||!!profile?.cosmetics?.frame?.animated;
 $('profile-cosmetic-manage').hidden=!own;$('cosmetic-motion-toggle').hidden=!animated;tools.hidden=!own&&!animated;refreshCosmeticMotion();
}
let cosmeticMotionQuery=null,cosmeticMotionReady=false;
let cosmeticMotionPaused=false;
try{cosmeticMotionPaused=localStorage.getItem('canna-cosmetic-motion')==='paused';}catch{}
function cosmeticAssetUrl(asset,hash){return typeof asset==='string'&&/^\/api\/v1\/cosmetics\/assets\/[a-zA-Z0-9_-]+$/.test(asset)?asset+(/^[a-f0-9]{64}$/.test(hash||'')?'?v='+hash:''):null;}
function cosmeticAnimationsPaused(){return cosmeticMotionPaused||!!cosmeticMotionQuery?.matches;}
function updateCosmeticMotionImage(image){const url=cosmeticAnimationsPaused()?image.dataset.cosmeticPoster:image.dataset.cosmeticAnimation;if(url&&image.getAttribute('src')!==url)image.src=url;}
function refreshCosmeticMotion(){
 for(const image of document.querySelectorAll('img[data-cosmetic-animation]'))updateCosmeticMotionImage(image);
 const control=$('cosmetic-motion-toggle');if(control){control.setAttribute('aria-pressed',String(cosmeticAnimationsPaused()));control.textContent=cosmeticMotionQuery?.matches?'Artwork animations paused by device':cosmeticMotionPaused?'Resume artwork animations':'Pause artwork animations';control.disabled=!!cosmeticMotionQuery?.matches;control.title=cosmeticMotionQuery?.matches?'Your device requests reduced motion. Artwork uses still images.':'Applies to profile banners and cosmetic previews in this browser.';}
}
function setupCosmeticMotion(){
 if(cosmeticMotionReady)return;cosmeticMotionReady=true;
 if(typeof window.matchMedia==='function'){cosmeticMotionQuery=window.matchMedia('(prefers-reduced-motion: reduce)');if(cosmeticMotionQuery.addEventListener)cosmeticMotionQuery.addEventListener('change',refreshCosmeticMotion);else cosmeticMotionQuery.addListener?.(refreshCosmeticMotion);}
 window.addEventListener?.('storage',event=>{if(event.key!=='canna-cosmetic-motion')return;cosmeticMotionPaused=event.newValue==='paused';refreshCosmeticMotion();});
}
function bindCosmeticMotion(image,item){
 if(!item.animated)return;
 const animation=cosmeticAssetUrl(item.asset,item.sha256),poster=cosmeticAssetUrl(item.poster_asset,item.poster_sha256);
 if(!animation||!poster)return;
 setupCosmeticMotion();image.dataset.cosmeticAnimation=animation;image.dataset.cosmeticPoster=poster;updateCosmeticMotionImage(image);
}
function artworkMotionControl(){
 setupCosmeticMotion();const control=document.createElement('button');control.id='cosmetic-motion-toggle';control.type='button';
 control.addEventListener('click',()=>{cosmeticMotionPaused=!cosmeticMotionPaused;try{localStorage.setItem('canna-cosmetic-motion',cosmeticMotionPaused?'paused':'auto');}catch{}refreshCosmeticMotion();});
 return control;
}
function profileCosmeticImage(item){const asset=cosmeticAssetUrl(item.asset,item.sha256);if(!asset)return null;const image=document.createElement('img');image.src=asset;image.alt='';image.decoding='async';bindCosmeticMotion(image,item);return image;}
function callingCardDisplayWidth(item){return item.collection==='mw2'&&Number.isInteger(item.width)&&item.width>0&&item.width<=4096?item.width*2:0;}
$('editprofile').addEventListener('submit',event => { event.preventDefault(); action(async () => {
  await json('profiles/me',{status:$('editstatus').value,bio:$('editbio').value}); await openProfile(currentUser.id); message('Profile saved.');
}); });
$('avatarform').addEventListener('submit',event => { event.preventDefault(); action(async () => {
  const file = $('avatarfile').files[0]; if (!file || file.size > 2*1024*1024) throw new Error('Choose a picture no larger than 2 MiB.');
  await api('profiles/me/avatar',{method:'POST',headers:{'Content-Type':file.type},body:file}); $('avatarform').reset(); await openProfile(currentUser.id); message('Profile picture updated.');
}); });
$('ratingform').addEventListener('submit',event => { event.preventDefault(); action(async () => {
  await json(`profiles/${profileId}/rating`,{stars:Number($('ratingstars').value)}); await openProfile(profileId); message('Rating saved. You can update your rating anytime.');
}); });
$('profilecommentform').addEventListener('submit',event => { event.preventDefault(); action(async () => {
  await json(`profiles/${profileId}/comments`,{body:$('profilecomment').value}); $('profilecommentform').reset(); await openProfile(profileId);
}); });

$('welcome').addEventListener('click',()=>action(()=>openProfile(currentUser.id)));

async function loadDevices() {
 const id=profileId,signal=typeof navigationSignal==='function'?navigationSignal():undefined;
 const devices=await(await api('devices',{signal})).json();
 if(profileId!==id||location.pathname!==`/members/${id}`||id!==currentUser.id||signal?.aborted)return;
 $('devicelist').replaceChildren(...devices.map(device=>{
  const row=document.createElement('article');row.className='entry';
  const info=document.createElement('div');const name=document.createElement('strong');name.textContent=device.name+(device.current?' · This device':'');
  const state=document.createElement('p');state.textContent=`${device.kind} · ${device.state} · Last activity ${new Date(device.last_seen*1000).toLocaleString()} · Signed in ${new Date(device.created*1000).toLocaleString()}`;info.append(name,state);row.append(info);
  row.append(button('Rename',async()=>{const name=prompt('Device name',device.name);if(name===null)return;await api(`devices/${device.id}`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({name})});await loadDevices();}));
  row.append(button('Log out',async()=>{if(!await cannaConfirm(`Log out ${device.name}${device.current?' (this device)':''}?`))return;await api(`devices/${device.id}`,{method:'DELETE'});if(device.current){location.replace('/');return;}await loadDevices();message('Device logged out.');}));return row;
 }));
}
$('refreshdevices').addEventListener('click',()=>action(loadDevices));
$('logoutothers').addEventListener('click',()=>action(async()=>{if(!await cannaConfirm('Log out every other device? This device stays signed in.'))return;await api('devices/logout-others',{method:'POST'});await loadDevices();message('All other devices logged out.');}));
setInterval(()=>{if(!$('loggeddevices').hidden && profileId===currentUser?.id)action(loadDevices);},30000);
