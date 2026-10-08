const fs=require('node:fs'),vm=require('node:vm'),assert=require('node:assert/strict');
const html=fs.readFileSync('server/web/index.html','utf8');
const scripts=[...html.matchAll(/<script src="\/([^\"]+)" defer>/g)].map(m=>m[1]);
for(const file of fs.readdirSync('server/web').filter(f=>f.endsWith('.js')))new vm.Script(fs.readFileSync('server/web/'+file,'utf8'),{filename:file});
async function fixture(path){
 const nodes=new Map(),all=[],navigation=[],requests=[];
 function node(tag='div'){
  const n={tag,children:[],events:{},hidden:false,querySelector(q){return this.children.find(c=>c.tag===q)||node(q);},value:'',textContent:'',dataset:{},style:{},classList:{toggle(){},add(){},remove(){}},insertBefore(n){this.children.push(n);},after(){},append(...v){this.children.push(...v);for(const child of v)if(child&&typeof child==='object')child.parentElement=this;},prepend(...v){this.children.unshift(...v);},replaceChildren(...v){this.children=v;},addEventListener(k,f){this.events[k]=f;},setAttribute(){},removeAttribute(){},closest(){return this;},focus(){},scrollIntoView(){},reset(){},showModal(){this.open=true;},close(){this.open=false;},remove(){},add(v){this.children.push(v);},get options(){return this.children;},get lastChild(){return this.children.at(-1);}};
  Object.defineProperty(n,'id',{set(v){this._id=v;nodes.set(v,this);},get(){return this._id;}});all.push(n);return n;
 }
 for(const m of html.matchAll(/<([a-z]+)[^>]*\bid="([^\"]+)"[^>]*>/g)){const n=node(m[1]);n.id=m[2];n.hidden=/\bhidden\b/.test(m[0]);}
 const user={id:1,username:'Owner',kash:321,role:'owner',admin:true,can_invite:true,can_publish_guides:true,invites_remaining:1};
 const section={id:'help',name:'Modding help',description:'Help with mods',active:true,vip_only:false};
 const topic={id:'thread-one',title:'A discussion',category:'help',app_id:1686940,posts:1,author:'Owner',updated:1,locked:false,pinned:false};
 const profile={...user,rank:'Noob',points:0,avatar:false,status:'',bio:'',ratings_count:0,posts_count:1,comments:[]};
 const get=id=>{assert(nodes.has(id),'Missing page element '+id);return nodes.get(id);};
 const location={origin:'https://cannamods.vip',pathname:path,search:'',assign(url){navigation.push(url);},replace(url){navigation.push(url);}};
 const doc={createElement:node,createTextNode:t=>Object.assign(node(),{textContent:t}),getElementById:id=>nodes.get(id),addEventListener(){},querySelectorAll:q=>q==='.admintabs button'?all.filter(n=>n.dataset.tab):all.filter(n=>n.id?.startsWith('admin-')),body:node(),head:node()};
 let ctx,updateRetry=0;doc.head.append=n=>{if(n.src==='/admin.js'){vm.runInContext(fs.readFileSync('server/web/admin.js','utf8'),ctx);n.onload();}};
 function data(p){
  if(/^packs\/.+\/info$/.test(p))return {id:'11111111-1111-4111-8111-111111111111',url:'https://cannamods.vip/packs/11111111-1111-4111-8111-111111111111',author:'Owner',revision:1,ready:true,can_update:true,manifest:{name:'Game night',description:'Shared selections',game:{name:'Bopl Battle'},mods:[]}};
  if(p==='catalog')return {games:[{app_id:1686940,name:'Bopl Battle'},{app_id:1557740,name:'ROUNDS'}]};
  if(p.startsWith('play?'))return {reports:[],rooms:[],channels:[],releases:[],total:0,channel_total:0};if(p.startsWith('invites?'))return {items:[],total:0,page:1,has_more:false};if(p==='providers/games')return {games:[]};if(p.startsWith('providers/search'))return {items:[],categories:[],has_more:false};if(p.startsWith('mods/subscriptions'))return {items:[],page:1,has_more:false};
  if(p==='mods/updates/status')return {retry_at:updateRetry,checks:[]};if(p==='mods/updates/check'){updateRetry=Math.floor(Date.now()/1000)+120;return {queued:true,retry_at:updateRetry};}
  if(p.includes("?page=")){const rows=data(p.split("?")[0]);return {items:rows,total:rows.length,page:1,page_size:50};}
  if(p==='chat/history')return ['first sent','/daily'];if(p==='chat')return [{id:1,user_id:1,username:'Owner',role:'owner',body:'first sent',created:Date.now()/1000,bot:false},{id:2,user_id:2,username:'Other',role:'member',body:'other sent',created:Date.now()/1000,bot:false},{id:3,user_id:1,username:'Owner',role:'owner',body:'/daily',created:Date.now()/1000,bot:false},{id:4,user_id:1,username:'Owner',role:'owner',body:'bot reply',created:Date.now()/1000,bot:true}];if(p==='me')return user;if(p==='sections')return {revision:1,sections:[section]};
  if(p.startsWith('topics?'))return [topic];if(p==='topics/thread-one')return {...topic,posts:[]};
  if(p==='profiles')return [profile];if(p==='profiles/1')return profile;
  if(p==='admin/wallets')return [{id:1,username:'Owner',balance:321,earned:500}];if(p==='admin/overview')return {storage_bytes:0,version:'fixture'};
  if(p==='announcement')return {revision:0,active:false,body:''};return [];
 }
 ctx=vm.createContext({document:doc,window:{addEventListener(){},open(){},scrollTo(){}},location,sessionStorage:{removeItem(){}},navigator:{},history:{replaceState(){},pushState(_s,_t,url){navigation.push(url);const u=new URL(url,"https://cannamods.vip");location.pathname=u.pathname;location.search=u.search;}},URL,console,crypto:require('node:crypto'),structuredClone,URLSearchParams,Headers,Date,setTimeout:()=>0,setInterval:()=>0,clearInterval(){},clearTimeout(){},Option:function(text,value){return Object.assign(node('option'),{textContent:text,value});},fetch:async(url,options={})=>{requests.push(url);return {ok:true,json:async()=>url.endsWith('/admin/sections/review')?{token:'fixture-review',layout:JSON.parse(options.body)}:data(url.replace('/api/v1/','')),clone(){return this;}};}});
 for(const file of scripts)vm.runInContext(fs.readFileSync('server/web/'+file,'utf8'),ctx,{filename:file});
 ctx.loadProviderBrowser=ctx.window.loadProviderBrowser;ctx.loadSubscriptions=ctx.window.loadSubscriptions;
 vm.runInContext('setupLounge()',ctx);
 await vm.runInContext('refresh()',ctx);
 assert.equal(get('message').textContent,'','Page startup failed for '+path);
 return {ctx,get,navigation,requests};
}
(async()=>{
 const browse=await fixture('/mods');assert.equal(browse.get('browseview').hidden,false);assert.equal(browse.get('libraryview').hidden,true);assert(browse.requests.some(p=>p.startsWith('/api/v1/providers/search')));assert(!browse.requests.includes('/api/v1/mods'));
 const subs=await fixture('/subscriptions');assert.equal(subs.get('subscriptionsview').hidden,false);assert(subs.requests.some(p=>p.startsWith('/api/v1/mods/subscriptions')));
 const library=await fixture('/library');
 const shared=await fixture('/packs/11111111-1111-4111-8111-111111111111');assert.equal(shared.get('packview').hidden,false);assert.equal(shared.get('libraryview').hidden,true);assert(shared.requests.some(p=>p.endsWith('/info')));assert(!shared.requests.includes('/api/v1/mods'));
 const example={id:'fixture-mod',name:'Fixture mod',version:'1.0',description:'A visible original description',author:'Author',details:{game:'Minecraft',provider:'modrinth',icon_data:'fixture',author_links:[{name:'Author',url:'https://modrinth.com/user/Author'}],game_versions:['1.21.1'],loaders:['Fabric'],dependencies:['Fabric API']}};
 assert.deepEqual(JSON.parse(fs.readFileSync('server/web/source-recommendations.json','utf8')),[]);
 library.ctx.curatedExample=example;
 const row=vm.runInContext("entry(curatedExample,'mods')",library.ctx);
 function descendants(n){return [n,...(n.children||[]).flatMap(c=>typeof c==='object'?descendants(c):[])];}
 const cells=descendants(row);
 assert(cells.some(n=>n.tag==='img'&&n.src.startsWith('data:image/')));
 assert(cells.some(n=>n.tag==='a'&&n.href===example.details.author_links[0].url));
 assert(!cells.some(n=>n.textContent==='Original description'||n.textContent==='Subscribe on Steam Workshop'));
 cells.find(n=>n.textContent==='Show more').events.click();
 const dialog=library.ctx.document.body.children.at(-1);assert.equal(dialog.open,true);const detailCells=descendants(dialog);assert(detailCells.some(n=>n.textContent==='Minecraft versions: 1.21.1'));assert(detailCells.some(n=>n.textContent==='Loaders: Fabric'));assert(detailCells.some(n=>n.textContent===example.description));
 assert(cells.some(n=>n.textContent===example.description));
 assert(html.includes('#space[data-booting]{display:none}'));
 const directory=await fixture('/members');assert(directory.requests.includes('/api/v1/profiles?page=1&search='));assert(!directory.requests.includes('/api/v1/packs'));
 const warmed=await fixture('/forums');await vm.runInContext("warmPage('/members')",warmed.ctx);const count=warmed.requests.filter(p=>p.startsWith('/api/v1/profiles?page=')).length;const auth=warmed.requests.filter(p=>p==='/api/v1/me').length;await vm.runInContext("navigatePage('/members')",warmed.ctx);assert.equal(warmed.requests.filter(p=>p.startsWith('/api/v1/profiles?page=')).length,count);assert(warmed.requests.filter(p=>p==='/api/v1/me').length>auth);assert.equal(warmed.get('profilesview').hidden,false);await vm.runInContext("api('notifications/read',{method:'POST'})",warmed.ctx);assert.equal(vm.runInContext('pageWarm.size',warmed.ctx),0);await vm.runInContext("listPages.get('people').page=2;loadPeople()",warmed.ctx);assert(warmed.requests.includes('/api/v1/profiles?page=2&search='));warmed.get('membersearch').value='Owner';await vm.runInContext("listPages.get('people').page=1;loadPeople()",warmed.ctx);assert(warmed.requests.includes('/api/v1/profiles?page=1&search=Owner'));
 const grouped=await fixture('/admin');await vm.runInContext('loadSectionEditor()',grouped.ctx);await grouped.get('addgroup').events.click();const groupInput=grouped.get('groupdraft').children[1].children[0].children[0];groupInput.value='Minecraft';groupInput.events.input();await grouped.get('groupdraft').children[1].children[1].children[2].events.click();assert.equal(vm.runInContext('sectionDraft.at(-1).group===groupDraft[1].id',grouped.ctx),true);vm.runInContext('forumGroups=structuredClone(groupDraft);forumCategories=structuredClone(sectionDraft);renderCategories()',grouped.ctx);assert.equal(grouped.get('categories').children.length,2);assert.equal(grouped.get('categories').children[1].children[0].children[0].textContent,'Minecraft');
 await grouped.get('reviewsections').events.click();assert.match(grouped.get('sectionstatus').textContent,/name before previewing/);vm.runInContext("sectionDraft.at(-1).name='Minecraft help'",grouped.ctx);await grouped.get('reviewsections').events.click();assert.equal(grouped.get('sectionreview').open,true);assert.equal(grouped.get('sectionpreview').children.length,2);assert(!grouped.requests.some(p=>p.endsWith('/admin/sections/apply')));grouped.get('cancelsections').events.click();assert.equal(grouped.get('sectionreview').open,false);
 const home=await fixture('/forums');assert.equal(home.get('forumindex').hidden,false);assert.equal(home.get('discussionlist').hidden,true);
 for(const [id,path] of [['forumnav','/forums'],['browsenav','/mods'],['librarynav','/library'],['subscriptionsnav','/subscriptions'],['submissionsnav','/submissions'],['notificationsnav','/notifications'],['peoplenav','/members'],['myprofilenav','/members/1'],['adminnav','/admin']]){await home.get(id).events.click();assert.equal(home.navigation.at(-1),path);}
 await vm.runInContext("navigatePage('/forums')",home.ctx);await home.get('categories').children[0].children[1].children[1].children[0].events.click();assert.equal(home.navigation.at(-1),'/forums/sections/help');
 const section=await fixture('/forums/sections/help');assert.equal(section.get('forumindex').hidden,true);assert.equal(section.get('discussionlist').hidden,false);assert(section.requests.includes('/api/v1/topics?offset=0&category=help'));
 await section.get('topics').children[0].children[0].children[0].events.click();assert.equal(section.navigation.at(-1),'/forums/topics/thread-one');
 const thread=await fixture('/forums/topics/thread-one');assert.equal(thread.get('thread').hidden,false);assert.equal(thread.get('discussionlist').hidden,true);await thread.get('closethread').events.click();assert.equal(thread.navigation.at(-1),'/forums/sections/help');
 for(const [path,id] of [['/mods','browseview'],['/library','libraryview'],['/subscriptions','subscriptionsview'],['/submissions','submissionsview'],['/notifications','notificationsview'],['/members','profilesview'],['/members/1','profilesview'],['/admin','moderation']]){const page=await fixture(path);assert.equal(page.get(id).hidden,false,path);assert.equal(page.navigation.length,0,'Unexpected redirect '+path);}
 const admin=await fixture('/admin');assert.equal(admin.get('kashbalance').textContent,'321 Kash');await vm.runInContext("selectAdminTab('logs')",admin.ctx);assert(admin.requests.some(p=>p.startsWith('/api/v1/admin/audit?page=')));await vm.runInContext("selectAdminTab('economy')",admin.ctx);assert(admin.requests.some(p=>p.startsWith('/api/v1/admin/wallets?page=')));assert.equal(admin.get('walletlist').children.length,1);
 await vm.runInContext('loadChat()',home.ctx);const input=home.get('chatbody');input.value='current draft';const key=k=>input.events.keydown({key:k,preventDefault(){}});key('ArrowUp');assert.equal(input.value,'/daily');key('ArrowUp');assert.equal(input.value,'first sent');key('ArrowUp');assert.equal(input.value,'first sent');key('ArrowDown');assert.equal(input.value,'/daily');key('ArrowDown');assert.equal(input.value,'current draft');
 const alerts=await fixture('/notifications');alerts.ctx.cannaConfirm=async()=>false;await alerts.get('clearnotifications').events.click();assert(!alerts.requests.includes('/api/v1/notifications/clear'));alerts.ctx.cannaConfirm=async()=>true;await alerts.get('clearnotifications').events.click();assert(alerts.requests.includes('/api/v1/notifications/clear'));
 const compose=await fixture('/forums/new');assert.equal(compose.get('newtopic').hidden,false);
 const updates=await fixture('/library');await vm.runInContext('checkUpdates.events.click()',updates.ctx);assert(updates.requests.includes('/api/v1/mods/updates/check'));assert.equal(vm.runInContext('checkUpdates.disabled',updates.ctx),true);assert(vm.runInContext('updateStatus.textContent',updates.ctx).includes('Queued'));
 const lab=await fixture('/play');assert.equal(lab.get('playview').hidden,false);assert.equal(lab.get('forumview').hidden,true);assert(lab.requests.some(p=>p.startsWith('/api/v1/play?')));
 lab.ctx.testPack={format:'canna_modpack',game:{app_id:550,framework:'source-vpk'},repository:{secret:'do-not-copy'},mods:[{name:'Hop',version:'1',sha256:'a'.repeat(64),file:'C:/Users/Private/mod.dll',provenance:{token:'do-not-copy'}}]};
 const manifest=vm.runInContext('playNormalize(testPack)',lab.ctx);assert(!JSON.stringify(manifest).includes('Private'));assert(!JSON.stringify(manifest).includes('do-not-copy'));
 lab.ctx.testLog='Loading Hop\\nC:/Users/Private/game\\nIP 192.168.1.2\\nToken: private';assert(!vm.runInContext('playCleanNote(testLog)',lab.ctx).includes('Private'));
 vm.runInContext('playManifest=playNormalize(testPack)',lab.ctx);lab.get('playnote').value='Successful keyboard session';lab.get('playoutcome').value='worked';await lab.get('playpreviewbutton').events.click();assert.equal(lab.get('playshare').disabled,false);const preview=vm.runInContext('JSON.stringify(playPreview)',lab.ctx);assert(preview.includes('Successful keyboard session'));lab.get('playnote').value='changed note';lab.get('playnote').events.input();assert.equal(lab.get('playshare').disabled,true);assert.equal(vm.runInContext('playPreview',lab.ctx),null);
 await home.get('playnav').events.click();assert.equal(home.navigation.at(-1),'/play');
 console.log('All website scripts parse; shared-script startup, every top-bar tab, dedicated sections/discussions/composer, category filtering and back navigation passed.');
})().catch(error=>{console.error(error);process.exitCode=1;});
