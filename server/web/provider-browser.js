'use strict';
(()=>{
 const section=document.createElement('section');section.className='card provider-browser';
 const heading=document.createElement('h2');heading.textContent='Browse mod providers';
 const note=document.createElement('p');note.textContent='Search provider catalogs, then add a mod and its required dependencies to Canna. Provider archives are cached for seven days; the same button retrieves an expired version again.';
 const form=document.createElement('form');form.className='row';
 function select(label,options){const wrap=document.createElement('label');wrap.textContent=label;const el=document.createElement('select');for(const [v,text]of options)el.add(new Option(text,v));wrap.append(el);form.append(wrap);return el;}
 const provider=select('Provider',[['thunderstore','Thunderstore'],['modrinth','Modrinth'],['curseforge','CurseForge']]);
 const game=select('Game',[['rounds','ROUNDS']]);
 const search=document.createElement('input');search.type='search';search.placeholder='Search mods or authors';search.maxLength=120;search.setAttribute('aria-label','Search provider mods');form.append(search);
 const order=select('Sort',[['updated','Recently updated'],['downloads','Most downloaded'],['rating','Top rated / popular'],['newest','Newest']]);
 const category=select('Category',[['','All categories']]);
 const version=document.createElement('input');version.placeholder='Game version (optional)';version.maxLength=40;version.setAttribute('aria-label','Game version');form.append(version);
 const go=document.createElement('button');go.type='submit';go.textContent='Search';form.append(go);
 const status=document.createElement('p');status.setAttribute('role','status');
 const results=document.createElement('div');results.className='provider-results';
 const pager=document.createElement('div');pager.className='row';const prev=document.createElement('button'),next=document.createElement('button'),pageLabel=document.createElement('span');prev.textContent='Previous';next.textContent='Next';prev.disabled=next.disabled=true;pager.append(prev,pageLabel,next);
 section.append(heading,note,form,status,results,pager);$('externalimport').before(section);
 let profiles=[],page=1,generation=0,loaded=false;
 function games(){const old=game.value;game.replaceChildren();if(provider.value!=='thunderstore')game.add(new Option('Minecraft','minecraft'));if(provider.value!=='modrinth')for(const p of profiles)game.add(new Option(p.name,p.community));if([...game.options].some(o=>o.value===old))game.value=old;version.hidden=provider.value==='thunderstore';category.replaceChildren(new Option('All categories',''));}
 async function initialize(){if(loaded)return;const data=await(await api('providers/games')).json();profiles=data.games;games();loaded=true;}
 function link(label,url){const a=document.createElement('a');a.textContent=label;try{const parsed=new URL(url);if(parsed.protocol==='https:'){a.href=parsed.href;a.target='_blank';a.rel='noopener noreferrer';}}catch{}return a;}
 function card(item){const box=document.createElement('article');box.className='card provider-result';
 if(/^https:\/\/(cdn\.thunderstore\.io|gcdn\.thunderstore\.io|cdn\.modrinth\.com|media\.forgecdn\.net)\//.test(item.icon_url||'')){const img=document.createElement('img');img.src=item.icon_url;img.alt=item.name+' original artwork';img.loading='lazy';img.referrerPolicy='no-referrer';img.className='modart';box.append(img);}
 const title=document.createElement('h3');title.textContent=item.name;const author=document.createElement('p');author.append('By ',link(item.authors||'Author',item.author_url));const description=document.createElement('p');description.textContent=item.description;const stats=document.createElement('p');stats.textContent=`${Number(item.downloads||0).toLocaleString()} downloads${item.rating==null?'':` · ${Number(item.rating).toLocaleString()} ${item.rating_label||'ratings'}`}`;
 const add=document.createElement('button');add.type='button';add.className='primary';add.textContent='Add to Canna & download';const feedback=document.createElement('p');feedback.setAttribute('role','status');
 add.addEventListener('click',async()=>{add.disabled=true;feedback.textContent='Checking the mod and its dependencies…';try{
 const preview=await json('mods/external/preview',{url:item.source_url});
 const selectedVersion=version.hidden?'':version.value.trim();
 const chosen=preview.versions.find(v=>(!selectedVersion||v.game_versions.includes(selectedVersion)))||(!selectedVersion?preview.versions[0]:null);
 if(!chosen)throw new Error('No compatible downloadable version. Choose another game version.');
 const result=await json('mods/external/import',{url:item.source_url,version:chosen.id,game_version:selectedVersion,include_optional:false});
 if(result.approved){feedback.textContent='Ready. Connecting to Canna…';await downloadToApp({id:result.id,name:preview.name,version:chosen.name},'mods');}
 else{feedback.textContent='Added. Scanning or moderator review is required before download. Find its status in My submissions.';const button=document.createElement('button');button.textContent='My submissions';button.addEventListener('click',()=>$('submissionsnav').click());feedback.append(' ',button);}
 }catch(e){feedback.textContent=e.message;}finally{add.disabled=false;}});
 box.append(title,author,description,stats,link('Original project',item.source_url),document.createTextNode(' '),add,feedback);return box;}
 async function browse(){const request=++generation;go.disabled=true;status.textContent='Loading provider results…';prev.disabled=next.disabled=true;try{await initialize();
 const params=new URLSearchParams({provider:provider.value,game:game.value,q:search.value.trim(),order:order.value,category:category.value,version:version.hidden?'':version.value.trim(),page:String(page)});
 const data=await(await api('providers/search?'+params)).json();if(request!==generation)return;
 results.replaceChildren(...data.items.map(card));const selected=category.value;category.replaceChildren(new Option('All categories',''),...(data.categories||[]).map(c=>new Option(c.name||c.slug,String(c.slug||c.id))));category.value=selected;
 prev.disabled=page<=1;next.disabled=!data.has_more;pageLabel.textContent=`Page ${page}`;status.textContent=data.stale?'Showing cached results; the provider is currently unavailable.':data.items.length?'':'No mods match these filters.';
 }catch(e){if(request===generation){results.replaceChildren();status.textContent=e.message;}}finally{if(request===generation)go.disabled=false;}}
 form.addEventListener('submit',event=>{event.preventDefault();page=1;browse();});provider.addEventListener('change',()=>{games();page=1;browse();});game.addEventListener('change',()=>{category.replaceChildren(new Option('All categories',''));page=1;browse();});prev.addEventListener('click',()=>{page--;browse();});next.addEventListener('click',()=>{page++;browse();});
 const observer=new IntersectionObserver(entries=>{if(entries.some(e=>e.isIntersecting)){initialize().catch(e=>status.textContent=e.message);observer.disconnect();}});observer.observe(section);
})();
