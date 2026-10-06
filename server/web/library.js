'use strict';
const gameNames={'1686940':'Bopl Battle','1557740':'ROUNDS','550':'Left 4 Dead 2','500':'Left 4 Dead','892970':'Valheim','220200':'Kerbal Space Program','255710':'Cities: Skylines','632360':'Risk of Rain 2'};
function libraryGameId(item) {return item.details?.game==='Minecraft'?'minecraft':String(item.app_id || item.game?.app_id || '');}
function libraryGameName(item) {return item.details?.game || item.game?.name || gameNames[libraryGameId(item)] || `Steam game ${libraryGameId(item)}`;}
function updateUploadGame() {
 const selected=$('uploadgame').value,custom=selected==='other';
 $('customgame').hidden=!custom;$('customgamelabel').hidden=!custom;$('customgame').required=custom;
 $('game').value=custom?$('customgame').value:selected;
 $('uploadgamehelp').textContent=['550','500'].includes(selected)?'Source games: upload a ZIP containing self-contained VPK addons. Canna uses -insecure for modded practice launches.':'Unity games: upload a ZIP containing your mod plugins.';
}
$('uploadgame').addEventListener('change',updateUploadGame);
$('customgame').addEventListener('input',updateUploadGame);
$('upload').addEventListener('reset',()=>queueMicrotask(updateUploadGame));
updateUploadGame();
let initialLibraryFilters=true;
function renderLibrary() {
  const selected=$('librarygame').value, games=new Map([['1686940','Bopl Battle'],['1557740','ROUNDS'],['550','Left 4 Dead 2'],['500','Left 4 Dead'],['minecraft','Minecraft']]);
  for(const item of [...libraryItems.mods,...libraryItems.packs]) games.set(libraryGameId(item),libraryGameName(item));
  if(new URLSearchParams(location.search).get('game')==='minecraft') games.set('minecraft','Minecraft');
  $('librarygame').replaceChildren(new Option('All games',''),...[...games].sort((a,b)=>a[1].localeCompare(b[1])).map(([id,name])=>new Option(name,id)));
  if(games.has(selected)) $('librarygame').value=selected;
  if(initialLibraryFilters) {
    initialLibraryFilters=false;
    const params=new URLSearchParams(location.search), requested=params.get('game');
    if(requested && games.has(requested)) $('librarygame').value=requested;
    $('libraryversion').value=params.get('mcversion') || '';
    const loader=params.get('loader');
    if([...$('libraryloader').options].some(o=>o.value===loader)) $('libraryloader').value=loader;
  }
  const query=$('librarysearch').value.toLowerCase(),game=$('librarygame').value,type=$('librarytype').value,provider=$('libraryprovider').value;
  let count=0;
  for(const kind of ['mods','packs']) {
    const section=$(kind).closest('section');section.hidden=!!type && (['mods','packs'].includes(type) ? type!==kind : kind!=='mods');
    const loader=$('libraryloader').value,version=$('libraryversion').value.trim();
    const items=libraryItems[kind].filter(m=>(!type || ['mods','packs'].includes(type) || m.details?.content_type===type) && (!loader || m.details?.loaders?.some(value=>value.toLowerCase()===loader)) && (!version || m.details?.game_versions?.includes(version)) && (!game || libraryGameId(m)===game) && (!query || `${m.name} ${m.description || ''} ${m.author}`.toLowerCase().includes(query)) && (!provider || (m.details?.provider || 'uploaded')===provider));
    $(kind).replaceChildren(...items.map(m=>entry(m,kind)));
    if(!items.length) $(kind).textContent=libraryItems[kind].length ? 'No matches for these filters.' : kind==='mods' ? 'No mods yet.' : 'No shared modpacks yet.';
    if(!section.hidden) count+=items.length;
  }
  $('librarycount').textContent=`${count} ${count===1?'item':'items'}`;
}
for(const id of ['librarysearch','librarygame','librarytype','libraryprovider','libraryloader','libraryversion']) $(id).addEventListener(['librarysearch','libraryversion'].includes(id)?'input':'change',renderLibrary);
function openExternalImport() {
 $('externalimport').open=true;
 $('externalimport').scrollIntoView({behavior:'smooth',block:'start'});
 $('externalurl').focus();
}
$('openexternalimport').addEventListener('click',openExternalImport);
let externalProject,externalLink;
$('externalurl').addEventListener('input',()=>{$('externalresult').hidden=true;externalProject=null;});
$('externalform').addEventListener('submit',async event=>{
  event.preventDefault();$('externalresult').hidden=true;$('externalstatus').textContent='Checking the project…';$('externalpreview').disabled=true;
  try {
    externalLink=$('externalurl').value;externalProject=await json('mods/external/preview',{url:externalLink});
    $('externaltitle').textContent=`${externalProject.name} · ${externalProject.game}`;
    $('externaldescription').textContent=`${externalProject.description} By ${externalProject.authors}.`;
    $('externalversion').replaceChildren(...externalProject.versions.map(v=>new Option(`${v.name} · ${v.loaders.join(', ')} · ${v.game_versions.join(', ')}`,v.id)));
    $('externalresult').hidden=false;$('externaladd').disabled=!externalProject.versions.length;
    $('externalstatus').textContent=externalProject.versions.length?'Choose the file you want to import.':'No downloadable files are available.';externalVersionInfo();
  } catch(e) {$('externalstatus').textContent=e.message;}
  finally {$('externalpreview').disabled=false;}
});
function externalVersionInfo() {const v=externalProject?.versions.find(v=>v.id===$('externalversion').value);const count=Array.isArray(v?.dependencies)?v.dependencies.length:0;$('externaldependencies').textContent=count?`${count} dependency references. Required dependencies and their dependencies are imported automatically. Optional dependencies can be included below.`:'No dependency references listed by the provider.';}
$('externalversion').addEventListener('change',externalVersionInfo);
$('externaladd').addEventListener('click',async()=>{
  if(!externalProject || externalLink!==$('externalurl').value) return;
  $('externaladd').disabled=true;$('externalstatus').textContent='Downloading, verifying and saving the mod…';
  try {const result=await json('mods/external/import',{url:externalLink,version:$('externalversion').value,game_version:$('libraryversion').value.trim(),loader:$('libraryloader').value,include_optional:$('externaloptional').checked});await loadLibrary();$('externalstatus').textContent=`${result.existing?'Mod already in library.':'Mod imported.'} ${result.dependencies_added||0} dependencies added (${result.dependency_count||0} linked). ${result.approved?'Published and ready to download.':'Automatic analysis is running; findings may require review. Check My submissions.'}`;}
  catch(e) {$('externalstatus').textContent=e.message;}finally {$('externaladd').disabled=false;}
});
let bridge,bridgeTimer,bridgeBusy=false;
async function downloadToApp(item,kind) {
  if(bridgeBusy) return;
  bridgeBusy=true;
  try {
    const result=await json('download-tickets',{id:item.id,kind});
    bridge={...result,item,kind,started:Date.now()};
    $('downloadtitle').textContent=`Download ${item.name}`;$('downloadstatus').textContent='Opening Canna and waiting for it to connect…';$('downloadmanual').hidden=true;
    $('downloadbridge').showModal();openBridge();
    clearInterval(bridgeTimer);bridgeTimer=setInterval(pollBridge,1000);
  } finally {bridgeBusy=false;}
}
function openBridge() {if(bridge) {const a=document.createElement('a');a.href=bridge.uri;a.click();}}
async function pollBridge() {
  if(!bridge) return;
  const current=bridge;
  try {
    const result=await (await api(`download-tickets/${current.ticket}`)).json();if(bridge!==current)return;
    if(result.state==='complete') {$('downloadstatus').textContent='Downloaded and verified in Canna. Open Downloads in the app to add it to a modpack.';clearInterval(bridgeTimer);}
    else if(result.state==='connected' || result.state==='downloading') {$('downloadstatus').textContent='Canna connected. Downloading and verifying the file…';}
    else if(['failed','expired'].includes(result.state) || (result.state==='waiting' && Date.now()-current.started>6000)) {$('downloadmanual').hidden=false;$('downloadstatus').textContent=result.state==='failed'?'The app download failed. Would you like to download the file manually?':result.state==='waiting'?'Canna has not connected. Would you like to download manually instead?':'The app connection expired. Try opening Canna again or download manually.';if(result.state!=='waiting')clearInterval(bridgeTimer);}
  } catch(e) {$('downloadstatus').textContent=e.message;$('downloadmanual').hidden=false;clearInterval(bridgeTimer);}
}
$('downloadopen').addEventListener('click',openBridge);
$('downloadmanual').addEventListener('click',()=>action(async()=>{if(!bridge)return;const {item,kind}=bridge;await download(`${kind}/${item.id}`,kind==='packs'?'Shared-Modpack.canna.json':item.details?.filename || `${item.name.replace(/[^a-z0-9_-]/gi,'_')}.zip`);$('downloadstatus').textContent='Manual download started.';}));
$('downloadclose').addEventListener('click',()=>$('downloadbridge').close());
$('downloadbridge').addEventListener('close',()=>{clearInterval(bridgeTimer);bridge=null;});

$('connectdesktop').addEventListener('click',()=>{location.href='/connect';});

