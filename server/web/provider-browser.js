'use strict';
(()=>{
 const section=document.createElement('section');section.className='card provider-browser';
 const heading=document.createElement('h2');heading.textContent='Browse mod providers';
 const note=document.createElement('p');note.textContent='Browsing loads listings only. Download retrieves the selected version and dependencies, caches them for seven days and subscribes you to the project.';
 const form=document.createElement('form');form.className='row';
 function select(label,options){const wrap=document.createElement('label');wrap.textContent=label;const el=document.createElement('select');el.setAttribute('aria-label',label);for(const [v,text]of options)el.add(new Option(text,v));wrap.append(el);form.append(wrap);return el;}
 const provider=select('Source',[['modrinth','Modrinth'],['curseforge','CurseForge'],['thunderstore','Thunderstore']]);
 const game=select('Game',[['minecraft','Minecraft']]);
 const search=document.createElement('input');search.type='search';search.placeholder='Search mods or authors';search.maxLength=120;search.setAttribute('aria-label','Search provider mods');form.append(search);
 const order=select('Sort',[['updated','Recently updated'],['downloads','Most downloaded'],['rating','Top rated / popular'],['newest','Newest']]);
 const category=select('Category',[['','All categories']]);
 const type=select('Content',[['','All content'],['mod','Mods'],['resourcepack','Resource packs'],['shader','Shaders'],['datapack','Data packs']]);
 const loader=select('Loader',[['','All loaders'],['fabric','Fabric'],['forge','Forge'],['neoforge','NeoForge'],['quilt','Quilt']]);
 const version=document.createElement('input');version.placeholder='Game version (optional)';version.maxLength=40;version.setAttribute('aria-label','Game version');form.append(version);
 const go=document.createElement('button');go.type='submit';go.textContent='Search';form.append(go);
 const status=document.createElement('p');status.setAttribute('role','status');
 const results=document.createElement('div');results.className='provider-results';
 const pager=document.createElement('div');pager.className='row';const prev=document.createElement('button'),next=document.createElement('button'),pageLabel=document.createElement('span');prev.textContent='Previous';next.textContent='Next';prev.disabled=next.disabled=true;pager.append(prev,pageLabel,next);
 section.append(heading,note,form,status,results,pager);$('providerbrowser').append(section);
 let profiles=[],page=1,generation=0,loaded=false,firstBrowse=true;
 const initial=new URLSearchParams(location.search);
 if(['modrinth','curseforge','thunderstore'].includes(initial.get('provider')))provider.value=initial.get('provider');
 search.value=initial.get('q')||'';order.value=initial.get('order')||'downloads';version.value=initial.get('version')||'';type.value=initial.get('content_type')||'';loader.value=initial.get('loader')||'';
 page=Math.max(1,Math.min(500,Number(initial.get('page'))||1));
 function games(){const old=game.value;game.replaceChildren();if(provider.value!=='thunderstore')game.add(new Option('Minecraft','minecraft'));if(provider.value!=='modrinth')for(const p of profiles){if(provider.value==='curseforge'&&!p.curseforge)continue;game.add(new Option(p.name,p.community));}if([...game.options].some(o=>o.value===old))game.value=old;version.hidden=provider.value==='thunderstore'||game.value!=='minecraft';type.parentElement.hidden=loader.parentElement.hidden=version.hidden;category.replaceChildren(new Option('All categories',''));}
 async function initialize(){if(loaded)return;const data=await(await api('providers/games')).json();profiles=data.games;games();if(initial.has('game')&&[...game.options].some(o=>o.value===initial.get('game')))game.value=initial.get('game');loaded=true;}
 function link(label,url){const a=document.createElement('a');a.textContent=label;try{const parsed=new URL(url);if(parsed.protocol==='https:'){a.href=parsed.href;a.target='_blank';a.rel='noopener noreferrer';}}catch{}return a;}
 function card(item){const box=document.createElement('article');box.className='card provider-result';
 if(/^https:\/\/(cdn\.thunderstore\.io|ccdn\.thunderstore\.io|gcdn\.thunderstore\.io|cdn\.modrinth\.com|media\.forgecdn\.net)\//.test(item.icon_url||'')){const img=document.createElement('img');img.src=item.icon_url;img.alt=item.name+' original artwork';img.loading='lazy';img.referrerPolicy='no-referrer';img.className='modart';box.append(img);}
 const title=document.createElement('h3');title.textContent=item.name;const author=document.createElement('p');author.append('By ',link(item.authors||'Author',item.author_url));const description=document.createElement('p');description.textContent=item.description;const stats=document.createElement('p');stats.textContent=`${Number(item.downloads||0).toLocaleString()} downloads${item.rating==null?'':` · ${Number(item.rating).toLocaleString()} ${item.rating_label||'ratings'}`}`;
 const add=document.createElement('button');add.type='button';add.className='primary';add.textContent='Download & subscribe';const feedback=document.createElement('p');feedback.setAttribute('role','status');
 add.addEventListener('click',async()=>{add.disabled=true;let unavailable=false;feedback.textContent='Checking the mod and its dependencies…';try{
 const preview=await json('mods/external/preview',{url:item.source_url});
 const selectedVersion=version.hidden?'':version.value.trim();
 const chosen=preview.versions.find(v=>(!selectedVersion||v.game_versions.includes(selectedVersion))&&(loader.parentElement.hidden||!loader.value||v.loaders.some(l=>l.toLowerCase()===loader.value)));
 if(!chosen)throw new Error('No compatible downloadable version. Choose another game version.');
 const result=await json('mods/external/import',{url:item.source_url,version:chosen.id,game_version:selectedVersion,loader:loader.parentElement.hidden?'':loader.value,include_optional:false});
 if(result.approved){feedback.textContent='Subscribed. Connecting to Canna…';await downloadToApp({id:result.id,name:preview.name,version:chosen.name},'mods');}
 else{feedback.textContent='Subscribed. Files were retrieved and are awaiting analysis or review. Download becomes available in Subscriptions when approved.';const button=document.createElement('button');button.textContent='Subscriptions';button.addEventListener('click',()=>$('subscriptionsnav').click());feedback.append(' ',button);}
 }catch(e){feedback.textContent=e.message;unavailable=['This mod is deprecated','This mod is unavailable','This mod is deprecated or unavailable'].includes(e.message);if(unavailable)add.textContent='Unavailable';}finally{add.disabled=unavailable;}});
 box.append(title,author,description,stats,link('Original project',item.source_url),document.createTextNode(' '),add,feedback);return box;}
 async function browse(){const request=++generation;go.disabled=true;status.textContent='Loading provider results…';prev.disabled=next.disabled=true;try{await initialize();
 const params=new URLSearchParams({provider:provider.value,game:game.value,q:search.value.trim(),order:order.value,category:category.value,version:version.hidden?'':version.value.trim(),page:String(page),loader:loader.parentElement.hidden?'':loader.value,content_type:type.parentElement.hidden?'':type.value});
 const data=await(await api('providers/search?'+params)).json();if(request!==generation)return;
 if(location.pathname==='/mods')history.replaceState(null,'','/mods?'+params);
 results.replaceChildren(...data.items.map(card));const selected=category.value;category.replaceChildren(new Option('All categories',''),...(data.categories||[]).map(c=>new Option(c.name||c.slug,String(c.slug||c.id))));category.value=selected;
 prev.disabled=page<=1;next.disabled=!data.has_more;pageLabel.textContent=`Page ${page}`;status.textContent=data.stale?'Showing cached results; the provider is currently unavailable.':data.items.length?'':'No mods match these filters.';
 }catch(e){if(request===generation){results.replaceChildren();status.textContent=e.message;}}finally{if(request===generation)go.disabled=false;}}
 form.addEventListener('submit',event=>{event.preventDefault();page=1;browse();});provider.addEventListener('change',()=>{games();page=1;browse();});game.addEventListener('change',()=>{version.hidden=provider.value==='thunderstore'||game.value!=='minecraft';type.parentElement.hidden=loader.parentElement.hidden=version.hidden;category.replaceChildren(new Option('All categories',''));page=1;browse();});prev.addEventListener('click',()=>{page--;browse();});next.addEventListener('click',()=>{page++;browse();});
 window.loadProviderBrowser=async()=>{if(firstBrowse){firstBrowse=false;await browse();}};
 for(const control of [order,category,type,loader])control.addEventListener('change',()=>{page=1;browse();});
})();

(()=>{
 let page=1,generation=0,timer;
 window.loadSubscriptions=async()=>{const request=++generation;$('subscriptionstatus').textContent='Loading subscriptions…';try{
 const data=await(await api('mods/subscriptions?'+new URLSearchParams({page:String(page),q:$('subscriptionsearch').value.trim()}))).json();if(request!==generation)return;
 $('subscriptionitems').replaceChildren(...data.items.map(item=>{const box=document.createElement('article');box.className='card';const title=document.createElement('h3');title.textContent=item.name;const state=document.createElement('p');state.textContent=`${item.provider} · ${item.version} · ${item.approved?'Ready to download':item.download_status?.message||(item.id?'Awaiting review':'Unavailable')}`;
 const download=document.createElement('button');download.textContent='Download';download.disabled=!item.approved;download.addEventListener('click',()=>action(()=>downloadToApp(item,'mods')));
 const remove=document.createElement('button');remove.textContent='Unsubscribe';remove.addEventListener('click',()=>action(async()=>{await api('mods/subscriptions',{method:'DELETE',headers:{'Content-Type':'application/json'},body:JSON.stringify({source:item.source})});await loadSubscriptions();}));box.append(title,state,download,remove);return box;}));
 $('subscriptionpage').textContent=`Page ${data.page}`;$('previoussubscriptions').disabled=page<=1;$('nextsubscriptions').disabled=!data.has_more;$('subscriptionstatus').textContent=data.items.length?'':'No subscriptions yet. Download a project from Mods to follow it.';
 }catch(e){if(request===generation)$('subscriptionstatus').textContent=e.message;}};
 $('refreshsubscriptions').addEventListener('click',()=>loadSubscriptions());
 $('previoussubscriptions').addEventListener('click',()=>{page--;loadSubscriptions();});$('nextsubscriptions').addEventListener('click',()=>{page++;loadSubscriptions();});
 $('subscriptionsearch').addEventListener('input',()=>{clearTimeout(timer);timer=setTimeout(()=>{page=1;loadSubscriptions();},250);});
 setInterval(()=>{if(currentUser&&!$('subscriptionsview').hidden)loadSubscriptions();},30000);
})();
