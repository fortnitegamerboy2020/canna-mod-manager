'use strict';
async function loadSharedPack() {
 const id=location.pathname.split('/')[2];
 if(!/^[a-f0-9-]{36}$/i.test(id||''))throw new Error('Invalid shared pack link.');
 const info=await(await api(`packs/${id}/info`)).json(), pack=info.manifest, root=$('packview');root.replaceChildren();
 const back=document.createElement('button');back.textContent='← My library';back.addEventListener('click',()=>navigatePage('/library'));
 const title=document.createElement('h1');title.textContent=pack.name;
 const meta=document.createElement('p');meta.textContent=`${pack.game.name} · by ${info.author} · Revision ${info.revision}`;
 const description=document.createElement('p');description.className='moddescription';description.textContent=pack.description||'No description provided.';
 const actions=document.createElement('div');actions.className='row';
 const install=document.createElement('button');install.className='primary';install.textContent='Install in Canna';install.disabled=!info.ready;install.addEventListener('click',()=>action(()=>downloadToApp({id,name:pack.name},'packs')));
 const copy=document.createElement('button');copy.textContent='Copy link';copy.addEventListener('click',()=>action(async()=>{await navigator.clipboard.writeText(info.url);message('Shared pack link copied.');}));actions.append(install,copy);
 const guidance=document.createElement('p');guidance.textContent=info.ready?'Open the imported pack in Canna → Modpacks, then Apply modpack. Keep your saved revision or use Check pack updates to review and accept a newer one.': 'Installation is unavailable while included mods are awaiting approval or blocked. Check back after review.';
 if(info.requires_rebound){const beta=document.createElement('p');beta.textContent='Canna Bliss Beta required: each player must have server-verified Beta access, enable Bliss in Settings, and use public ROUNDS 1.1.2. The link contains original mod archives, not the protected support DLLs.';root.append(beta);}
 root.append(back,title,meta,description,actions,guidance);
 if(info.can_update){const owner=document.createElement('p');owner.textContent='You created this pack. Edit it in Canna and use Publish pack update to update this same link. Friends choose whether to update their saved copy.';root.append(owner);}
 const heading=document.createElement('h2');heading.textContent=`Included mods (${pack.mods.length})`;root.append(heading);
 const list=document.createElement('div');list.className='provider-results';
 for(const mod of pack.mods){const card=document.createElement('article');card.className='card';const name=document.createElement('h3');name.textContent=mod.name;const detail=document.createElement('p');detail.textContent=`${mod.version} · ${mod.enabled?'Enabled':'Disabled'} · ${mod.provenance?.authors||'Creator not recorded'}`;const desc=document.createElement('p');desc.textContent=mod.description;card.append(name,detail,desc);
  if(mod.dependencies?.length){const deps=document.createElement('p');deps.textContent='Requires: '+mod.dependencies.join(', ');card.append(deps);}
  if(/^https:\/\//.test(mod.provenance?.source_url||'')){const source=document.createElement('a');source.href=mod.provenance.source_url;source.textContent='Original project';source.target='_blank';source.rel='noopener noreferrer';card.append(source);}list.append(card);
 }
 if(!pack.mods.length)list.textContent='This pack currently has no mods.';root.append(list);
}
