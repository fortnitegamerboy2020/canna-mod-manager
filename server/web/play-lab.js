'use strict';
// No telemetry or raw log upload. A previewed, allowlisted manifest is the only shared setup data.
let playManifest=null,playPreview=null,playRoom='',playInvite='',playChannel='',playPage=1,playTimer=0;
function playNode(tag,text){const node=document.createElement(tag);if(text!==undefined)node.textContent=text;return node;}
function playCleanNote(text){return text.split('\n').slice(0,30).map(line=>/[\\/@]|:\/|::|\b(?:token|password|authorization|cookie|secret|sessionid|session_id|session\s*[:=]|steamid|username|hostname|email|api.?key)\b|\b\d{1,3}(?:\.\d{1,3}){3}\b|[a-fA-F0-9]{16,}/i.test(line)?'[private line removed]':line.replace(/[\u0000-\u001f]/g,'').slice(0,500)).join('\n').slice(0,2000);}
function playNormalize(raw){
 const fromPack=raw.format==='canna_modpack';
 const m=fromPack?{game:raw.game?.app_id,branch:'',build:'',loader:raw.game?.framework||'',manager:'',mods:(raw.mods||[]).filter(m=>m.enabled!==false).map(m=>({name:m.name,version:m.version,sha256:m.sha256||'',dependencies:m.dependencies||[]})),shared_configs:{}}:raw;
 if(!m||!Number.isInteger(m.game)||m.game<=0||!Array.isArray(m.mods)||m.mods.length>1000)throw new Error('Choose a Canna pack manifest or an exported Play Lab manifest.');
 const out={game:m.game,branch:String(m.branch||''),build:String(m.build||''),loader:String(m.loader||''),manager:String(m.manager||''),mods:m.mods.map(item=>({name:String(item.name||''),version:String(item.version||''),sha256:String(item.sha256||''),dependencies:Array.isArray(item.dependencies)?item.dependencies.map(String):[]})),shared_configs:{}};
 for(const [key,value] of Object.entries(m.shared_configs||{})){if(!/^[a-zA-Z0-9 _-]{1,100}$/.test(key)||!/^[a-fA-F0-9]{64}$/.test(value))throw new Error('Invalid shared configuration digest');out.shared_configs[key]=value;}
 return out;
}
function playCurrent(){if(!playManifest)throw new Error('Load a local manifest first. The desktop app can export a measured Play Lab manifest.');return {...structuredClone(playManifest),branch:$('playbranch').value,build:$('playbuild').value,loader:$('playloader').value,manager:$('playmanager').value};}
function playPayload(action,extra={}){return {action,...extra};}
async function playPost(action,extra={}){return json('play/action',playPayload(action,extra));}
function playButton(text,callback){return button(text,callback);}
function playDisplayRoom(data){
 playRoom=data.id;if(data.invite){playInvite=data.invite;$('playinvite').value=data.invite;}
 const target=$('playmembers');target.replaceChildren();
 for(const member of data.members||[]){const row=playNode('section');row.className='entry';const info=playNode('div');info.append(playNode('strong',`${member.alias} · ${member.ready?'Matched':member.active?'Needs checks':'Offline / stale'}`));for(const check of member.checks||[])if(check.level!=='matched')info.append(playNode('p',check.message));row.append(info);target.append(row);}
 $('playclose').textContent=data.host?'Close lobby':'Leave lobby';$('playclose').dataset.action=data.host?'close':'leave';$('playclose').disabled=false;$('playupdate').disabled=false;
 const comparison=playNode('details');comparison.append(playNode('summary','Host manifest'),playNode('pre',JSON.stringify(data.manifest,null,2)));target.append(comparison);
 clearInterval(playTimer);playTimer=setInterval(()=>{if(!$('playview').hidden&&playRoom)action(async()=>playDisplayRoom(await playPost('read',{id:playRoom})));},20000);
}
async function loadPlayLab(){
 const data=await(await api('play?'+new URLSearchParams({page:playPage,channel:playChannel}))).json();
 const reports=$('playreports');reports.replaceChildren();
 for(const report of data.reports||[]){const row=playNode('section');row.className='entry';const info=playNode('div');info.append(playNode('strong',`${report.outcome==='worked'?'Successful session':'Reported failure'} · Game ${report.manifest?.game} · Build ${report.manifest?.build||'unknown'}`),playNode('p',report.note),playNode('small',new Date(report.created*1000).toLocaleString()));const details=playNode('details');details.append(playNode('summary','Exact tested manifest'),playButton('Load exact manifest',async()=>{const value=await playPost('read-report',{id:report.id});details.append(playNode('pre',JSON.stringify(value.manifest,null,2)));}));info.append(details);row.append(info);if(report.can_remove)row.append(playButton('Remove report',async()=>{if(await cannaConfirm('Remove this compatibility report?')){await playPost('remove-report',{id:report.id});await loadPlayLab();}}));reports.append(row);}
 if(!data.reports?.length)reports.append(playNode('p','No compatibility reports yet. Unknown compatibility remains unknown.'));
 $('playpage').textContent=`Page ${playPage} · ${data.total||0} reports`;$('playprevious').disabled=playPage<=1;$('playnext').disabled=playPage*50>=Math.max(data.total||0,data.channel_total||0);
 const channels=$('playchannels');channels.replaceChildren();
 for(const channel of data.channels||[]){const row=playNode('div');row.className='row';row.append(playButton(channel.name,async()=>{playChannel=channel.id;$('playchannel').value=channel.id;await loadPlayLab();}),playNode('small',channel.id+(channel.you?' · Your channel':'')));channels.append(row);}
 const releases=$('playreleases');releases.replaceChildren();
 for(const release of data.releases||[]){const row=playNode('section');row.className='entry';const info=playNode('div');info.append(playNode('strong',release.stage+' · '+release.id));const details=playNode('details');details.append(playNode('summary','Release manifest'),playNode('pre',JSON.stringify(release.manifest,null,2)));info.append(details);row.append(info);row.append(playButton('Download manifest',async()=>{const value=await playPost('read-release',{id:release.id});const blob=new Blob([JSON.stringify(value.manifest,null,2)],{type:'application/json'}),url=URL.createObjectURL(blob),a=playNode('a');a.href=url;a.download='canna-play-manifest.json';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);}));if(release.you){row.append(playButton('Promote to stable',async()=>{if(await cannaConfirm('Promote this exact tested release to stable?')){await playPost('promote',{id:release.id});await loadPlayLab();}}),playButton('Remove release',async()=>{if(await cannaConfirm('Remove this release manifest?')){await playPost('remove-release',{id:release.id});await loadPlayLab();}}));}releases.append(row);}
 const rooms=$('playrooms');rooms.replaceChildren();for(const room of data.rooms||[])rooms.append(playButton((room.host?'Hosted lobby ':'Joined lobby ')+room.id,async()=>playDisplayRoom(await playPost('read',{id:room.id}))));
}
function setupPlayLab(){
 $('playnav').addEventListener('click',()=>action(()=>showView('playview')));
 $('playfile').addEventListener('change',()=>action(async()=>{const file=$('playfile').files[0];if(!file||file.size>512*1024)throw new Error('Choose a manifest no larger than 512 KiB');playManifest=playNormalize(JSON.parse(await file.text()));for(const key of ['branch','build','loader','manager'])$('play'+key).value=playManifest[key];playPreview=null;$('playpreview').textContent='Manifest loaded locally. Review it before sharing.';$('playshare').disabled=true;$('playissue').disabled=true;}));
 for(const id of ['playbranch','playbuild','playloader','playmanager','playnote','playoutcome'])$(id).addEventListener('input',()=>{playPreview=null;$('playshare').disabled=true;$('playissue').disabled=true;});
 $('playcreate').addEventListener('click',()=>action(async()=>playDisplayRoom(await playPost('create',{alias:$('playalias').value,manifest:playCurrent()}))));
 $('playjoin').addEventListener('click',()=>action(async()=>{const [id,code,...rest]=$('playinvite').value.trim().split(':');if(!id||!code||rest.length)throw new Error('Paste the complete lobby invitation');playDisplayRoom(await playPost('join',{id,code,alias:$('playalias').value,manifest:playCurrent()}));}));
 $('playupdate').addEventListener('click',()=>action(async()=>playDisplayRoom(await playPost('update',{id:playRoom,manifest:playCurrent()}))));
 $('playclose').addEventListener('click',()=>action(async()=>{if(await cannaConfirm('Leave or close this lobby?')){await playPost($('playclose').dataset.action,{id:playRoom});playRoom='';playInvite='';clearInterval(playTimer);$('playmembers').replaceChildren();$('playclose').disabled=true;$('playupdate').disabled=true;await loadPlayLab();}}));
 $('playpreviewbutton').addEventListener('click',()=>action(()=>{playPreview={manifest:playCurrent(),outcome:$('playoutcome').value,note:playCleanNote($('playnote').value)};$('playpreview').textContent=JSON.stringify(playPreview,null,2);$('playshare').disabled=false;$('playissue').disabled=false;}));
 $('playshare').addEventListener('click',()=>action(async()=>{if(!playPreview)throw new Error('Preview your report first');await playPost('report',playPreview);playPreview=null;$('playshare').disabled=true;$('playissue').disabled=true;await loadPlayLab();message('Compatibility report shared without an account name.');}));
 $('playissue').addEventListener('click',()=>action(async()=>{if(!playPreview)throw new Error('Preview your report first');const {manifest,note}=playPreview;const result=await playPost('issue',{manifest,note});playPreview=null;$('playshare').disabled=true;$('playissue').disabled=true;message('Private support ticket created: '+result.ticket);}));
 $('playpublish').addEventListener('click',()=>action(async()=>{const manifest=playCurrent();if(!await cannaConfirm('Publish this manifest as an experimental release? Paths, logs and configuration values are excluded.'))return;const result=await playPost('publish',{id:$('playchannel').value.trim(),alias:$('playchannelname').value,manifest});playChannel=result.channel;$('playchannel').value=playChannel;await loadPlayLab();}));
 $('playrefresh').addEventListener('click',()=>action(async()=>{playChannel=$('playchannel').value.trim();await loadPlayLab();}));
 $('playprevious').addEventListener('click',()=>action(async()=>{playPage=Math.max(1,playPage-1);await loadPlayLab();}));$('playnext').addEventListener('click',()=>action(async()=>{playPage++;await loadPlayLab();}));
 window.addEventListener('pagehide',()=>{clearInterval(playTimer);playManifest=null;playPreview=null;playInvite='';});
}
setupPlayLab();
