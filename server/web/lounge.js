 'use strict';
let loungeReady=false,announcementRevision=0;
function loungeNode(tag,text,className){const n=document.createElement(tag);if(text!==undefined)n.textContent=text;if(className)n.className=className;return n;}
function setupLounge(){
 if(loungeReady)return;loungeReady=true;
 const banner=loungeNode('aside',undefined,'announcementbanner');banner.id='announcementbanner';banner.hidden=true;banner.setAttribute('aria-label','Community announcement');
 const box=loungeNode('details',undefined,'globalchat');box.id='globalchat';box.open=true;
 const summary=loungeNode('summary','Community live chat');summary.append(loungeNode('small','Messages expire after 24 hours'));
 const list=loungeNode('div',undefined,'chatmessages');list.id='chatmessages';list.setAttribute('aria-label','Recent chat messages');
 const form=loungeNode('form');form.id='chatform';const input=loungeNode('input');input.id='chatbody';input.maxLength=1000;input.required=true;input.placeholder='Say something to the community…';input.setAttribute('aria-label','Chat message');
 const send=loungeNode('button','Send');send.className='primary';send.type='submit';const status=loungeNode('p','');status.id='chatstatus';status.setAttribute('role','status');
 form.append(input,send);form.addEventListener('submit',e=>{e.preventDefault();action(async()=>{send.disabled=true;try{await json('chat',{body:input.value});input.value='';status.textContent='';await loadChat(true);}catch(error){status.textContent=error.message;}finally{send.disabled=false;}});});
 const commands=loungeNode('div',undefined,'chatcommands');commands.append(loungeNode('small','CannaBot'));for(const command of ['/help','/fish','/daily','/balance','/collection','/badges']){const b=loungeNode('button',command);b.type='button';b.addEventListener('click',()=>{input.value=command;input.focus();});commands.append(b);}box.append(summary,list,commands,form,status);$('space').insertBefore(banner,$('space').children[1]);banner.after(box);
}
async function loadChat(forceBottom=false){
 setupLounge();const rows=await(await api('chat')).json(),list=$('chatmessages');const bottom=forceBottom||list.scrollTop+list.clientHeight>=list.scrollHeight-32;
 list.replaceChildren(...rows.filter(m=>m.created*1000>Date.now()-86400000).map(m=>{
   const row=loungeNode('div',undefined,'chatline');row.dataset.created=m.created;
   const who=loungeNode('button',m.bot?'CannaBot':m.username,'chatwho');who.type='button';if(!m.bot)who.addEventListener('click',()=>action(()=>openProfile(m.user_id)));else who.disabled=true;
   const time=loungeNode('time',new Date(m.created*1000).toLocaleTimeString([], {hour:'2-digit',minute:'2-digit'}));time.dateTime=new Date(m.created*1000).toISOString();
   row.append(time,who);if(!m.bot && m.badge && m.badge!=='none')row.append(loungeNode('small',({angler:'🎣 Angler',emerald:'◆ Emerald',legend:'★ Legend'})[m.badge]||'','chatbadge'));row.append(loungeNode('span',m.body,'chattext'));
   if(currentUser?.admin || m.user_id===currentUser?.id){const remove=loungeNode('button','×','chatdelete');remove.type='button';remove.setAttribute('aria-label',`Delete ${m.username}'s message`);remove.addEventListener('click',()=>action(async()=>{if(!await cannaConfirm('Delete this chat message?'))return;await api(`chat/${m.id}`,{method:'DELETE'});await loadChat();}));row.append(remove);}return row;
 }));if(!list.children.length)list.append(loungeNode('p','No messages yet. Start a conversation.','sidehint'));if(bottom)list.scrollTop=list.scrollHeight;
}
async function loadAnnouncement(){setupLounge();const v=await(await api('announcement')).json();const banner=$('announcementbanner');banner.textContent=v.body;banner.hidden=!v.active;return v;}
async function loadLoungeAdmin(){const v=await(await api('announcement')).json();announcementRevision=v.revision;$('announcementbody').value=v.body;$('announcementactive').checked=v.active;}
function setupLoungeAdmin(){
 const pane=$('admin-community');pane.append(loungeNode('h3','Announcement banner'));const form=loungeNode('form');
 const label=loungeNode('label','Announcement text');label.htmlFor='announcementbody';const text=loungeNode('textarea');text.id='announcementbody';text.maxLength=2000;text.rows=4;
 const active=loungeNode('label');active.className='check';const toggle=loungeNode('input');toggle.type='checkbox';toggle.id='announcementactive';active.append(toggle,document.createTextNode('Show banner to signed-in members'));
 const send=loungeNode('button','Review & apply banner');send.className='primary';send.type='submit';form.append(label,text,active,send);
 form.addEventListener('submit',e=>{e.preventDefault();action(async()=>{const draft={body:text.value,active:toggle.checked,revision:announcementRevision};if(!await cannaConfirm(draft.active?`Publish this announcement?\n\n${draft.body}`:'Hide the announcement banner?',{title:'Review announcement',label:'Apply announcement'}))return;send.disabled=true;try{await json('admin/announcement',draft);await loadLoungeAdmin();await loadAnnouncement();message('Announcement updated.');}finally{send.disabled=false;}});});
 pane.append(form,loungeNode('h3','Live chat moderation'),loungeNode('p','Delete individual messages in the chat, or clear the current chat history. Daily anti-spam allowances remain in place.'),button('Clear global chat',async()=>{if(!await cannaConfirm('Delete all current global chat messages?',{title:'Clear community chat',label:'Clear chat'}))return;await json('admin/chat/clear',{});await loadChat();}));
}
setInterval(()=>{const list=$('chatmessages');if(list)for(const row of Array.from(list.children))if(row.dataset.created && Number(row.dataset.created)*1000<=Date.now()-86400000)row.remove();},30000);
