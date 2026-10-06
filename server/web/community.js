'use strict';
let openThread = '', threadData, topicPage = 0, topicItems = [], sourceItems = [];
let forumCategories = [], sectionRevision = 0;
function categoryName(key) { return forumCategories.find(c => c.id === key)?.name || key; }
async function loadSections() {
  const data = await (await api('sections')).json(); forumCategories = data.sections; sectionRevision = data.revision;
  const filter = $('topicfilter').value, selected = $('topiccategory').value;
  $('topicfilter').replaceChildren(new Option('All sections',''),...forumCategories.map(s => new Option(s.name + (s.active ? '' : ' · Closed'),s.id)));
  $('topicfilter').value = forumCategories.some(s=>s.id===filter) ? filter : '';
  const allowed=forumCategories.filter(s=>s.active && (!s.vip_only || currentUser.can_publish_guides));
  $('topiccategory').replaceChildren(...allowed.map(s=>new Option(s.name,s.id)));
  if(allowed.some(s=>s.id===selected)) $('topiccategory').value=selected;
  $('composetopic').disabled=!allowed.length;
}
function activity(seconds) {
  if (!seconds) return '—';
  return new Date(seconds*1000).toLocaleString(undefined,{month:'short',day:'numeric',hour:'numeric',minute:'2-digit'});
}
function updateNavigation() {
  const views = {librarynav:'libraryview',forumnav:'forumview',peoplenav:'profilesview',myprofilenav:'profilesview',adminnav:'moderation'};
  for (const [nav,view] of Object.entries(views)) {
    const active = !$(view).hidden && (nav !== 'myprofilenav' || profileId === currentUser.id) && (nav !== 'peoplenav' || profileId !== currentUser.id);
    $(nav).classList.toggle('active',active); $(nav).setAttribute('aria-current',active ? 'page' : 'false');
  }
}
function button(label, callback) {
  const node = document.createElement('button'); node.textContent = label;
  node.addEventListener('click', () => action(callback)); return node;
}
function showView(name) {
  for (const id of ['libraryview','forumview','moderation','profilesview']) $(id).hidden = id !== name;
  updateNavigation();
  if(name==='forumview') return (async()=>{await loadTopics();if(openThread) await loadThread(openThread,true);})();
  if(name==='libraryview') return loadLibrary();
  if (name === 'moderation') return loadAdmin();
}
$('librarynav').addEventListener('click', () => action(() => showView('libraryview')));
$('forumnav').addEventListener('click', () => action(() => showView('forumview')));
$('adminnav').addEventListener('click', () => action(() => showView('moderation')));
async function loadTopics() {
  await loadSections();
  topicItems = await (await api(`topics?offset=${topicPage}`)).json(); renderTopics();
  renderCategories();
  $('oldertopics').disabled = topicItems.length < 50;
  $('newertopics').disabled = topicPage === 0;
  const selectedMod=$('topicmod').value;
  const mods = await (await api('mods')).json();
  $('topicmod').replaceChildren(new Option('No linked mod',''), ...mods.map(mod => new Option(mod.name,mod.id)));
  if(mods.some(m=>m.id===selectedMod)) $('topicmod').value=selectedMod;
}
function renderCategories() {
  $('categories').replaceChildren(...forumCategories.map(({id:key,name,description,active,vip_only}) => {
    const glyph = key === 'help' ? '?' : key === 'showcase' ? '+' : key === 'guides' ? '≡' : '#';
    const row = document.createElement('div'); row.className = 'categoryrow';
    const icon = document.createElement('span'); icon.className = 'categoryglyph'; icon.textContent = glyph; icon.setAttribute('aria-hidden','true');
    const info = document.createElement('div');
    const link = button(name,async () => { $('topicfilter').value = key; renderTopics(); $('discussionlist').scrollIntoView({behavior:'smooth',block:'start'}); }); link.className = 'categoryname';
    const detail = document.createElement('p'); detail.textContent = description + (active ? (vip_only ? ' · VIP+ posting' : '') : ' · Closed to new discussions'); info.append(link,detail);
    const count = document.createElement('div'); count.className = 'countcell'; count.textContent = String(topicItems.filter(t => t.category === key).length);
    const label = document.createElement('small'); label.textContent = 'on this page'; count.append(label);
    row.append(icon,info,count); return row;
  }));
  $('sidecount').textContent = String(topicItems.length);
  $('sideposts').textContent = String(topicItems.reduce((total,t)=>total + t.posts,0));
  $('recenttopics').replaceChildren(...[...topicItems].sort((a,b)=>(b.updated || 0)-(a.updated || 0)).slice(0,4).map(t => {
    const link = button(t.title,() => loadThread(t.id)); link.className = 'sidetopic'; return link;
  }));
  if (!topicItems.length) $('recenttopics').textContent = 'Your next discussion could start here.';
}
function renderTopics() {
  const filtered = topicItems.filter(item => (!($('topicfilter').value) || item.category === $('topicfilter').value) && item.title.toLowerCase().includes($('topicsearch').value.toLowerCase()));
  $('topics').replaceChildren(...filtered.map(item => {
    const row = document.createElement('div'); row.className = 'topicrow';
    const info = document.createElement('div'); const title = button(item.title,() => loadThread(item.id)); title.className = 'topiclink';
    if (item.pinned) { const tag = document.createElement('span'); tag.className = 'topiclabel'; tag.textContent = 'PINNED'; info.append(tag); }
    info.append(title);
    const meta = document.createElement('p'); meta.textContent = `${categoryName(item.category)} · ${item.app_id === 1686940 ? 'Bopl Battle' : `Game ${item.app_id}`}${item.locked ? ' · Locked' : ''}`;
    info.append(meta);
    const replies = document.createElement('span'); replies.className = 'topiccount'; replies.textContent = String(Math.max(0,item.posts-1));
    const author = document.createElement('span'); author.className = 'topicmeta'; author.textContent = item.author;
    const date = document.createElement('span'); date.className = 'topicmeta'; date.textContent = activity(item.updated);
    row.append(info,replies,author,date); return row;
  }));
  $('listtitle').textContent = $('topicfilter').value ? categoryName($('topicfilter').value) : 'Latest discussions';
  $('topiccount').textContent = `${filtered.length} on this page`;
  if (!filtered.length) { const empty = document.createElement('div'); empty.className = 'empty'; empty.textContent = topicItems.length ? 'No discussions match your filters.' : 'No discussions yet. Start the first one.'; $('topics').append(empty); }
}
function compose(show) {
  $('newtopic').hidden = !show; $('forumindex').hidden = $('discussionlist').hidden = show;
  $('thread').hidden = true; openThread = '';
  if (show) { const choice=forumCategories.find(s=>s.id===$('topicfilter').value && s.active && (!s.vip_only || currentUser.can_publish_guides)); if(choice) $('topiccategory').value=choice.id; $('topictitle').focus(); }
}
$('composetopic').addEventListener('click',() => compose(true));
$('cancelcompose').addEventListener('click',() => compose(false));
$('topicsearch').addEventListener('input',renderTopics); $('topicfilter').addEventListener('change',renderTopics);
$('oldertopics').addEventListener('click', () => action(async () => { topicPage += 50; await loadTopics(); }));
$('newertopics').addEventListener('click', () => action(async () => { topicPage = Math.max(0,topicPage-50); await loadTopics(); }));
$('newtopic').addEventListener('submit', event => { event.preventDefault(); action(async () => {
  const result = await json('topics',{title:$('topictitle').value,body:$('topicbody').value,category:$('topiccategory').value,app_id:Number($('topicgame').value),mod_id:$('topicmod').value || null});
  $('newtopic').reset(); topicPage = 0; await loadTopics(); await loadThread(result.id);
}); });
async function loadThread(id,liveUpdate=false) {
  const updated=await (await api(`topics/${id}`)).json();
  if(liveUpdate && (openThread!==id || $('forumview').hidden)) return;
  threadData=updated; openThread=id;
  $('thread').hidden = false; $('newtopic').hidden = true;
  $('forumindex').hidden = $('discussionlist').hidden = true;
  $('threadtitle').textContent = threadData.title;
  $('replyform').hidden = threadData.locked && !currentUser.admin;
  const memberInfo = await (await api('profiles')).json();
  $('threadposts').replaceChildren(...threadData.posts.map((post,index) => {
    const row = document.createElement('article'); row.className = 'threadpost';
    const author = document.createElement('aside'); author.className = 'postauthor';
    const member = memberInfo.find(m => m.id === post.user_id);
    const avatar = document.createElement(member?.avatar ? 'img' : 'span'); avatar.className = 'authorinitial';
    if (member?.avatar) { avatar.src = `/api/v1/profiles/${post.user_id}/avatar`; avatar.alt = `${post.author}'s profile picture`; }
    else avatar.textContent = post.author.slice(0,1).toUpperCase();
    const title = button(post.author,() => openProfile(post.user_id));
    const role = document.createElement('small'); role.textContent = post.role;
    author.append(avatar,title,role);
    const content = document.createElement('div'); content.className = 'postcontent';
    const meta = document.createElement('div'); meta.className = 'postmeta';
    const date = document.createElement('span'); date.textContent = new Date(post.created*1000).toLocaleString();
    const number = document.createElement('span'); number.textContent = `#${index+1}`; meta.append(date,number);
    const body = document.createElement('p'); body.className = 'postbody'; body.textContent = post.body;
    const actions = document.createElement('div'); actions.className = 'postactions';
    if (currentUser.admin || currentUser.id === post.user_id) actions.append(button('Remove post',async () => {
      if (!confirm('Remove this post?')) return;
      await api(`posts/${post.id}`,{method:'DELETE'}); await loadThread(id);
    }));
    content.append(meta,body,actions); row.append(author,content);
    return row;
  }));
  $('threadtools').replaceChildren();
  if (threadData.mod_id) $('threadtools').append(button('Download linked mod',() => download(`mods/${threadData.mod_id}`,'Linked-Mod.zip')));
  if (currentUser.admin) {
    const change = async (locked,pinned) => { await json(`topics/${id}/moderate`,{locked,pinned}); await loadThread(id); await loadTopics(); };
    $('threadtools').append(button(threadData.locked ? 'Unlock' : 'Lock',() => change(!threadData.locked,threadData.pinned)),button(threadData.pinned ? 'Unpin' : 'Pin',() => change(threadData.locked,!threadData.pinned)),button('Delete discussion',async () => {
      if (!confirm('Delete the discussion and all its posts?')) return;
      await api(`topics/${id}`,{method:'DELETE'}); closeThread(); await loadTopics();
    }));
  }
  if(!liveUpdate) $('thread').scrollIntoView({behavior:'smooth',block:'start'});
}
function closeThread() { compose(false); }
$('closethread').addEventListener('click',closeThread);
$('replyform').addEventListener('submit',event => { event.preventDefault(); action(async () => {
  await json(`topics/${openThread}/reply`,{body:$('replybody').value}); $('replyform').reset(); await loadThread(openThread); await loadTopics();
}); });
async function loadAdmin() {
  $('sectionmanager').hidden=currentUser.role !== 'owner';
  if(currentUser.role === 'owner') await loadSectionEditor();
  $('ownercontrols').hidden=currentUser.role !== 'owner';
  if (currentUser.role === 'owner') $('ownercontrols').append($('invitationcontrols'));
  const members = await (await api('admin/users')).json();
  $('adminsummary').textContent = `${members.length} accounts · ${members.filter(u => u.verified).length} verified · ${members.filter(u => u.banned).length} banned`;
  $('memberlist').replaceChildren(...members.map(member => {
    const row = document.createElement('div'); row.className = 'entry';
    const info = document.createElement('div'); const title = document.createElement('strong'); title.textContent = member.username;
    const meta = document.createElement('p'); meta.textContent = `${member.role.toUpperCase()} · ${member.banned ? 'Banned' : member.verified ? 'Active' : 'Awaiting email verification'}`;
    info.append(title,meta); const tools = document.createElement('div'); tools.className = 'row';
    if (member.verified) tools.append(button('Profile',() => openProfile(member.id)));
    if (member.id !== currentUser.id && member.role !== 'owner' && (currentUser.role === 'owner' || member.role !== 'admin')) tools.append(button(member.banned ? 'Unban' : 'Ban',async () => {
      if (!confirm(`${member.banned ? 'Unban' : 'Ban'} ${member.username}?`)) return;
      await json(`admin/users/${member.id}/ban`,{banned:!member.banned}); await loadAdmin();
    }));
    if (currentUser.role === 'owner' && member.role !== 'owner' && member.verified && !member.banned) {
      const roles = document.createElement('select'); roles.setAttribute('aria-label',`Role for ${member.username}`);
      for (const role of ['member','vip','admin']) roles.add(new Option(role.toUpperCase(),role,role === member.role,role === member.role));
      tools.append(roles,button('Save role',async () => { await json(`admin/users/${member.id}/role`,{role:roles.value}); await loadAdmin(); message('Role updated. The member must sign in again.'); }),button('Transfer ownership',async () => {
        if (!confirm(`Make ${member.username} the Owner? You will become an Admin and both accounts will be signed out.`)) return;
        await json('admin/transfer-owner',{user_id:member.id}); location.assign('/');
      }));
    }
    row.append(info,tools); return row;
  }));
  $('ownerlog').hidden = currentUser.role !== 'owner';
  if (currentUser.role === 'owner') {
    const events = await (await api('admin/audit')).json();
    $('auditlog').replaceChildren(...events.map(event => { const row = document.createElement('p'); row.textContent = `${new Date(event.created*1000).toLocaleString()} · ${event.actor} · ${event.action} · ${event.target}`; return row; }));
    if (!events.length) $('auditlog').textContent = 'No moderation actions yet.';
  }
}
async function viewSource(id) {
  const result = await (await api(`mods/${id}/source`)).json(); sourceItems = result.files;
  $('sourcenote').textContent = result.note;
  $('sourcefiles').replaceChildren(...sourceItems.map((file,index) => new Option(file.name,String(index))));
  $('sourcetext').textContent = sourceItems[0]?.text || 'No source files were included in this archive.';
  $('sourceviewer').showModal();
}
$('sourcefiles').addEventListener('change',() => { $('sourcetext').textContent = sourceItems[Number($('sourcefiles').value)]?.text || ''; });
$('closesource').addEventListener('click',() => $('sourceviewer').close());
