const fs=require('node:fs'),vm=require('node:vm'),assert=require('node:assert/strict');
const html=fs.readFileSync('server/web/index.html','utf8');
const scripts=[...html.matchAll(/<script src="\/([^\"]+)" defer>/g)].map(m=>m[1]);
for(const file of fs.readdirSync('server/web').filter(f=>f.endsWith('.js')))new vm.Script(fs.readFileSync('server/web/'+file,'utf8'),{filename:file});
async function fixture(path){
 const nodes=new Map(),all=[],navigation=[],requests=[];
 function node(tag='div'){
  const n={tag,children:[],events:{},hidden:false,value:'',textContent:'',dataset:{},style:{},classList:{toggle(){},add(){},remove(){}},insertBefore(n){this.children.push(n);},after(){},append(...v){this.children.push(...v);},prepend(...v){this.children.unshift(...v);},replaceChildren(...v){this.children=v;},addEventListener(k,f){this.events[k]=f;},setAttribute(){},closest(){return this;},focus(){},scrollIntoView(){},reset(){},showModal(){},close(){},remove(){},add(v){this.children.push(v);},get options(){return this.children;},get lastChild(){return this.children.at(-1);}};
  Object.defineProperty(n,'id',{set(v){this._id=v;nodes.set(v,this);},get(){return this._id;}});all.push(n);return n;
 }
 for(const m of html.matchAll(/<([a-z]+)[^>]*\bid="([^\"]+)"[^>]*>/g)){const n=node(m[1]);n.id=m[2];n.hidden=/\bhidden\b/.test(m[0]);}
 const user={id:1,username:'Owner',kash:321,role:'owner',admin:true,can_invite:true,can_publish_guides:true,invites_remaining:1};
 const section={id:'help',name:'Modding help',description:'Help with mods',active:true,vip_only:false};
 const topic={id:'thread-one',title:'A discussion',category:'help',app_id:1686940,posts:1,author:'Owner',updated:1,locked:false,pinned:false};
 const profile={...user,rank:'Noob',points:0,avatar:false,status:'',bio:'',ratings_count:0,posts_count:1,comments:[]};
 const get=id=>{assert(nodes.has(id),'Missing page element '+id);return nodes.get(id);};
 const location={pathname:path,search:'',assign(url){navigation.push(url);},replace(url){navigation.push(url);}};
 const doc={createElement:node,createTextNode:t=>Object.assign(node(),{textContent:t}),getElementById:id=>nodes.get(id),addEventListener(){},querySelectorAll:q=>q==='.admintabs button'?all.filter(n=>n.dataset.tab):all.filter(n=>n.id?.startsWith('admin-')),body:node(),head:node()};
 let ctx;doc.head.append=n=>{if(n.src==='/admin.js'){vm.runInContext(fs.readFileSync('server/web/admin.js','utf8'),ctx);n.onload();}};
 function data(p){
  if(p==='chat/history')return ['first sent','/daily'];if(p==='chat')return [{id:1,user_id:1,username:'Owner',role:'owner',body:'first sent',created:Date.now()/1000,bot:false},{id:2,user_id:2,username:'Other',role:'member',body:'other sent',created:Date.now()/1000,bot:false},{id:3,user_id:1,username:'Owner',role:'owner',body:'/daily',created:Date.now()/1000,bot:false},{id:4,user_id:1,username:'Owner',role:'owner',body:'bot reply',created:Date.now()/1000,bot:true}];if(p==='me')return user;if(p==='sections')return {revision:1,sections:[section]};
  if(p.startsWith('topics?'))return [topic];if(p==='topics/thread-one')return {...topic,posts:[]};
  if(p==='profiles')return [profile];if(p==='profiles/1')return profile;
  if(p==='admin/wallets')return [{id:1,username:'Owner',balance:321,earned:500}];if(p==='admin/overview')return {storage_bytes:0,version:'fixture'};
  if(p==='announcement')return {revision:0,active:false,body:''};return [];
 }
 ctx=vm.createContext({document:doc,window:{addEventListener(){},open(){}},location,sessionStorage:{removeItem(){}},navigator:{},console,structuredClone,URLSearchParams,Headers,Date,setTimeout:()=>0,setInterval:()=>0,clearInterval(){},clearTimeout(){},Option:function(text,value){return Object.assign(node('option'),{textContent:text,value});},fetch:async(url)=>{requests.push(url);return {ok:true,json:async()=>data(url.replace('/api/v1/',''))};}});
 for(const file of scripts)vm.runInContext(fs.readFileSync('server/web/'+file,'utf8'),ctx,{filename:file});
 vm.runInContext('setupLounge()',ctx);
 await vm.runInContext('refresh()',ctx);
 assert.equal(get('message').textContent,'','Page startup failed for '+path);
 return {ctx,get,navigation,requests};
}
(async()=>{
 const home=await fixture('/forums');assert.equal(home.get('forumindex').hidden,false);assert.equal(home.get('discussionlist').hidden,true);
 for(const [id,path] of [['forumnav','/forums'],['librarynav','/mods'],['submissionsnav','/submissions'],['notificationsnav','/notifications'],['peoplenav','/members'],['myprofilenav','/members/1'],['adminnav','/admin']]){await home.get(id).events.click();assert.equal(home.navigation.at(-1),path);}
 await home.get('categories').children[0].children[1].children[0].events.click();assert.equal(home.navigation.at(-1),'/forums/sections/help');
 const section=await fixture('/forums/sections/help');assert.equal(section.get('forumindex').hidden,true);assert.equal(section.get('discussionlist').hidden,false);assert(section.requests.includes('/api/v1/topics?offset=0&category=help'));
 await section.get('topics').children[0].children[0].children[0].events.click();assert.equal(section.navigation.at(-1),'/forums/topics/thread-one');
 const thread=await fixture('/forums/topics/thread-one');assert.equal(thread.get('thread').hidden,false);assert.equal(thread.get('discussionlist').hidden,true);await thread.get('closethread').events.click();assert.equal(thread.navigation.at(-1),'/forums/sections/help');
 for(const [path,id] of [['/mods','libraryview'],['/submissions','submissionsview'],['/notifications','notificationsview'],['/members','profilesview'],['/members/1','profilesview'],['/admin','moderation']]){const page=await fixture(path);assert.equal(page.get(id).hidden,false,path);assert.equal(page.navigation.length,0,'Unexpected redirect '+path);}
 const admin=await fixture('/admin');assert.equal(admin.get('kashbalance').textContent,'321 Kash');await vm.runInContext("selectAdminTab('logs')",admin.ctx);assert(admin.requests.includes('/api/v1/admin/audit'));await vm.runInContext("selectAdminTab('economy')",admin.ctx);assert(admin.requests.includes('/api/v1/admin/wallets'));assert.equal(admin.get('walletlist').children.length,1);
 await vm.runInContext('loadChat()',home.ctx);const input=home.get('chatbody');input.value='current draft';const key=k=>input.events.keydown({key:k,preventDefault(){}});key('ArrowUp');assert.equal(input.value,'/daily');key('ArrowUp');assert.equal(input.value,'first sent');key('ArrowUp');assert.equal(input.value,'first sent');key('ArrowDown');assert.equal(input.value,'/daily');key('ArrowDown');assert.equal(input.value,'current draft');
 const alerts=await fixture('/notifications');alerts.ctx.cannaConfirm=async()=>false;await alerts.get('clearnotifications').events.click();assert(!alerts.requests.includes('/api/v1/notifications/clear'));alerts.ctx.cannaConfirm=async()=>true;await alerts.get('clearnotifications').events.click();assert(alerts.requests.includes('/api/v1/notifications/clear'));
 const compose=await fixture('/forums/new');assert.equal(compose.get('newtopic').hidden,false);
 console.log('All website scripts parse; shared-script startup, every top-bar tab, dedicated sections/discussions/composer, category filtering and back navigation passed.');
})().catch(error=>{console.error(error);process.exitCode=1;});
