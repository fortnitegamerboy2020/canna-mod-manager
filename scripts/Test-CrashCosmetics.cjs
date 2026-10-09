// Real Edge DOM, production UI handlers and synthetic delayed HTTP. No live bets.
const fs=require('node:fs'),assert=require('node:assert/strict');
const {chromium}=require(process.env.CANNA_PLAYWRIGHT_MODULE||'playwright');
const catalog=JSON.parse(fs.readFileSync('server/web/cosmetics/catalog.json','utf8'));
const items=[catalog.items.find(i=>i.kind==='frame'),catalog.items.find(i=>i.collection==='cod-ranks'),...catalog.items.filter(i=>i.kind==='name_effect')];
const assets=new Map(catalog.items.map(i=>[i.id,i]));
for(const item of catalog.items)if(item.poster_filename)assets.set(item.id+'-poster',{filename:item.poster_filename});
(async()=>{
 const browser=await chromium.launch({headless:true,channel:'msedge'});
 try{for(const width of [1280,390]){
  const page=await browser.newPage({viewport:{width,height:900}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.route('**/*',async route=>{
   const url=new URL(route.request().url());if(url.hostname!=='canna-fixture.invalid')throw Error('Fixture tried external HTTP');
   const id=url.pathname.split('/').at(-1),item=assets.get(id);
   if(item)return route.fulfill({status:200,contentType:({png:'image/png',jpg:'image/jpeg',webp:'image/webp',svg:'image/svg+xml'})[item.filename.split('.').at(-1)],body:fs.readFileSync('server/web/cosmetics/'+item.filename)});
   if(url.pathname==='/forum.css'||url.pathname==='/admin-games.css')return route.fulfill({status:200,contentType:'text/css',body:fs.readFileSync('server/web'+url.pathname)});
   return route.fulfill({status:200,contentType:'text/html',body:'<html></html>'});
  });
  await page.goto('https://canna-fixture.invalid/gambling');
  await page.setContent(fs.readFileSync('server/web/index.html','utf8').replace(/<script[^>]*>[\s\S]*?<\/script>/g,''));
  await page.evaluate(({items,allItems})=>{
   window.$=id=>document.getElementById(id);$('space').removeAttribute('data-booting');$('bootstatus').hidden=true;
   for(const section of document.querySelectorAll('#space>section'))section.hidden=section.id!=='gamblingview';$('gamblingview').hidden=false;
   window.currentUser={id:1,username:'Neon Cryptid',admin:false,kash:1000};window.button=(text,fn)=>{const b=document.createElement('button');b.type='button';b.textContent=text;b.addEventListener('click',()=>window.action(fn));return b;};window.action=fn=>Promise.resolve().then(fn).catch(e=>{window.lastActionError=e.message;});window.message=()=>{};window.navigatePage=async()=>{};window.memberRoleLabel=()=> 'MEMBER';window.loadNotifications=async()=>{};
   window.posts=[];window.reads=[];window.holdPoll=false;window.heldPolls=[];window.latency=200;window.holdAfterPost=false;window.loseReply=false;window.receipts=new Map();
   window.rules={paused:false,games:['crash','blackjack','roulette','dice','slots','cases'].map(game=>({game,enabled:true,min_stake:1,max_stake:1000000,payout_percent:100})),crates:['bo2-calling-cards','mw2-calling-cards','avatar-frames','cod-emblems','username-effects'].map(case_id=>({case_id,cost:100}))};
   window.fixtureCatalog={version:'c'.repeat(64),catalog:allItems.map(i=>({...i,paused:i.collection==='mw2'})),cases:['bo2-calling-cards','mw2-calling-cards','avatar-frames','cod-emblems','username-effects'].map(id=>({id,name:id,cost:100,collection:id==='username-effects'?'username-effects':id==='avatar-frames'?'frames':id==='cod-emblems'?null:id.slice(0,3),kind:id==='cod-emblems'?'emblem':id==='username-effects'?'name_effect':id==='avatar-frames'?'frame':'banner',items:allItems.filter(i=>id==='cod-emblems'?i.kind==='emblem':id==='username-effects'?i.kind==='name_effect':id==='avatar-frames'?i.kind==='frame':id==='bo2-calling-cards'?i.collection==='bo2':false).map(i=>({id:i.id,odds_percent:100})),available:id!=='mw2-calling-cards',paused:id==='mw2-calling-cards'}))};
   window.state={member_id:1,wallet:{balance:1000,daily_available:false},server_time_ms:Date.now(),crash:{id:9,phase:'running',mode:'random',paused:false,betting_ends_ms:Date.now()-5000,multiplier:1.64,history:[],bet:{round_id:9,stake:100,auto_cashout:null,status:'pending'},participants:[],participant_count:0},rules,blackjack:null,recent_games:[],cosmetics:{catalog_version:fixtureCatalog.version,owned:items.map(i=>({id:i.id,count:1})),equipped:{frame:items[0].id,banner:null,emblem:items[1].id,name_effect:items[2].id}}};
   window.api=async(path,options={})=>{
    reads.push(path);if(path==='gambling/cosmetics/catalog')return{json:async()=>structuredClone(fixtureCatalog)};
    if(!path.startsWith('gambling?'))throw Error('Unexpected read '+path);
    const snapshot=structuredClone(state);snapshot.server_time_ms=Date.now();snapshot.crash.multiplier=Math.exp((snapshot.server_time_ms-snapshot.crash.betting_ends_ms)/10000);
    if(holdPoll){await new Promise(resolve=>heldPolls.push(resolve));}else await new Promise(r=>setTimeout(r,latency));
    return{json:async()=>snapshot};
   };
   window.json=async(path,payload)=>{
    posts.push({path,payload:structuredClone(payload),sentAt:performance.now()});
    if(receipts.has(payload.request_id))return structuredClone(receipts.get(payload.request_id));
    await new Promise(r=>setTimeout(r,600));let result;
    if(path==='gambling/crash/cashout'){state.wallet.balance=1200;state.crash.bet={...state.crash.bet,status:'won',payout:200,cashout_multiplier:2,cashout_at_ms:Date.now()};result={ok:true,bet:structuredClone(state.crash.bet),balance:1200};}
    else if(path==='gambling/cosmetics/equip'){state.cosmetics.equipped=structuredClone(payload);result={ok:true,equipped:structuredClone(payload)};}
    else if(path==='gambling/cases/open'){const pool=fixtureCatalog.cases.find(c=>c.id===payload.case_id);const item=fixtureCatalog.catalog.find(i=>i.id===pool.items[0].id);let owned=state.cosmetics.owned.find(i=>i.id===item.id);if(!owned){owned={id:item.id,count:0};state.cosmetics.owned.push(owned);}owned.count++;state.wallet.balance-=100;result={ok:true,item:structuredClone(item),count:owned.count,balance:state.wallet.balance,cost:100};}
    else throw Error('Unexpected mutation '+path);
    if(payload.request_id)receipts.set(payload.request_id,result);if(holdAfterPost)holdPoll=true;
    if(loseReply){loseReply=false;throw new TypeError('Synthetic response lost');}return result;
   };
   state.crash.participants=Array.from({length:200},(_,i)=>({user_id:i+1,username:i?'Synthetic player '+(i+1):currentUser.username,display_name:i?'Synthetic player '+(i+1):currentUser.username,profile_url:'/members/'+(i+1),stake:100,status:'pending'}));state.crash.participant_count=200;
   window.ownedFixture=items;
  },{items,allItems:catalog.items});
  for(const file of ['profiles.js','gambling.js'])await page.addScriptTag({content:fs.readFileSync('server/web/'+file,'utf8')});
  await page.evaluate(async()=>{await loadGambling();});
  assert.equal(await page.locator('#case-list .casecard').count(),5);
  assert.equal(await page.evaluate(()=>Math.round(crashVisual.oneWayMs)>=90&&Math.round(crashVisual.oneWayMs)<=150),true,'RTT must correct the render clock');
  const before=await page.locator('#crash-cashout').textContent();await page.waitForTimeout(350);const after=await page.locator('#crash-cashout').textContent();assert.notEqual(before,after,'Cashout text must move between snapshots');
  // Hold an old GET indefinitely. Click must POST immediately, and a confirmed
  // result must be visible while both the old and follow-up GET remain held.
  await page.evaluate(()=>{holdPoll=true;loadGambling(true).catch(()=>{});});await page.waitForFunction(()=>heldPolls.length>0);
  const clickedAt=await page.evaluate(()=>performance.now());await page.locator('#crash-cashout').click();await page.waitForFunction(()=>posts.some(p=>p.path==='gambling/crash/cashout'));
  const sentAt=await page.evaluate(()=>posts.find(p=>p.path==='gambling/crash/cashout').sentAt);assert(sentAt-clickedAt<250,'Cashout waited for the pending poll');
  assert.equal(await page.locator('#crash-cashout').textContent(),'Cashing out…');
  await page.waitForFunction(()=>!gamblingBusy&&gamblingData.crash.bet.status==='won');
  assert.match(await page.locator('#gambling-status').textContent(),/Cashed out at 2.00/);assert.match(await page.locator('#crash-your-bet').textContent(),/200 Kash/);assert.match(await page.locator('#kashbalance').textContent(),/1,200/);
  assert.match(await page.locator('#crash-people-rows tr[data-user-id="1"]').textContent(),/Cashed out · 2.00/);
  assert.equal(await page.evaluate(()=>Object.keys(posts[0].payload).sort().join(',')),'request_id,round_id','Client timestamps must not choose payouts');
  // Release the ignored stale GET. Generation checks must not restore pending.
  await page.evaluate(()=>{holdPoll=false;for(const release of heldPolls.splice(0))release();});await page.waitForTimeout(350);assert.equal(await page.evaluate(()=>gamblingData.crash.bet.status),'won');
  // Exercise all four slots and animated text with production profile code.
  await page.evaluate(()=>{applyProfileCosmetics({username:currentUser.username,avatar:false,cosmetics:{frame:ownedFixture[0],emblem:ownedFixture[1],name_effect:ownedFixture[2]}});renderProfileCosmeticActions(true,{cosmetics:{name_effect:ownedFixture[2]}});selectGamblingTab('collection');});
  assert.equal(await page.locator('#profilename').getAttribute('data-username-effect'),'aurora');assert.equal(await page.locator('#profilename .username-emblem').count(),1);
  assert.equal(await page.locator('#cosmetic-source option[value="cod-ranks"]').count(),1);
  await page.locator('#cosmetic-kind').selectOption('name_effect');assert.equal(await page.locator('#cosmetic-collection .cosmeticcard').count(),6);
  assert.equal(await page.locator('#cosmetic-collection .nameeffectpreview strong').first().evaluate(n=>getComputedStyle(n).animationName),'canna-name-flow');
  await page.locator('#cosmetic-collection .nameeffectpreview strong').first().scrollIntoViewIfNeeded();await page.waitForTimeout(100);const gradientBefore=await page.locator('#cosmetic-collection .nameeffectpreview strong').first().evaluate(n=>getComputedStyle(n).backgroundPosition);await page.waitForTimeout(300);assert.notEqual(await page.locator('#cosmetic-collection .nameeffectpreview strong').first().evaluate(n=>getComputedStyle(n).backgroundPosition),gradientBefore,'Visible username gradient must animate');
  await page.emulateMedia({reducedMotion:'reduce'});assert.equal(await page.locator('#cosmetic-collection .nameeffectpreview strong').first().evaluate(n=>getComputedStyle(n).animationName),'none');
  await page.emulateMedia({reducedMotion:'no-preference'});await page.evaluate(()=>{cosmeticMotionPaused=true;refreshCosmeticMotion();});assert.equal(await page.locator('#cosmetic-collection .nameeffectpreview strong').first().evaluate(n=>getComputedStyle(n).animationName),'none');
  await page.evaluate(()=>{cosmeticMotionPaused=false;refreshCosmeticMotion();});
  await page.evaluate(()=>equipCosmetic('name_effect','name-effect-rainbow'));await page.waitForFunction(()=>!gamblingBusy&&gamblingData.cosmetics.equipped.name_effect==='name-effect-rainbow');
  const equip=await page.evaluate(()=>posts.find(p=>p.path==='gambling/cosmetics/equip').payload);assert.equal(equip.emblem,items[1].id);assert.equal(equip.frame,items[0].id);assert.equal(equip.banner,null);
  await page.screenshot({path:`target/username-gradients-${width}.png`,fullPage:true});
  await page.evaluate(()=>selectGamblingTab('cases'));await page.locator('[data-crate="username-effects"]').getByRole('button',{name:'Open crate · 100 Kash',exact:true}).click();await page.waitForFunction(()=>!gamblingBusy&&$('case-result').textContent.includes('Equip this username effect'));
  assert.equal(await page.evaluate(()=>gamblingData.cosmetics.owned.find(i=>i.id==='name-effect-aurora').count),2);
  assert.equal(await page.locator('.case-reel-item').count(),36);assert.equal(await page.locator('#case-result').getByRole('button',{name:'Equip this username effect',exact:true}).isVisible(),false,'Reveal waits for roll, inventory does not');
  assert.equal(await page.locator('[data-crate="username-effects"]').getByRole('button',{name:'Open crate · 100 Kash',exact:true}).isDisabled(),true);
  await page.waitForTimeout(350);await page.locator('#case-result').screenshot({path:`target/crate-reel-${width}.png`});
  // Advance the actual Web Animation almost to its end; winner must align with
  // the center marker regardless of the responsive cell width.
  const centered=await page.evaluate(()=>{const track=document.querySelector('.case-reel-track'),animation=track.getAnimations()[0];animation.pause();animation.currentTime=5199;const winner=track.children[30],box=winner.getBoundingClientRect(),view=track.parentElement.getBoundingClientRect();return {distance:Math.abs(box.x+box.width/2-view.x-view.width/2),id:winner.dataset.cosmeticId};});
  assert(centered.distance<2,'Server-selected winner did not align with center');assert.equal(centered.id,'name-effect-aurora');
  await page.locator('#case-result').getByRole('button',{name:'Skip animation',exact:true}).click();assert.equal(await page.locator('.case-reel').count(),0);
  assert.equal(await page.evaluate(()=>posts.filter(p=>p.path==='gambling/cases/open').length),1,'Skip must not purchase again');
  await page.locator('#case-result').getByRole('button',{name:'Equip this username effect',exact:true}).click();await page.waitForFunction(()=>!gamblingBusy&&gamblingData.cosmetics.equipped.name_effect==='name-effect-aurora');
  await page.screenshot({path:`target/cosmetic-crates-${width}.png`,fullPage:true});
  // Natural completion, resize/tab cleanup, reduced motion and a lost crate
  // reply all reveal the same saved result without changing the charge.
  const openEffects=async()=>{await page.locator('[data-crate="username-effects"]').getByRole('button',{name:'Open crate · 100 Kash',exact:true}).click();await page.waitForFunction(()=>!!caseReel&&!gamblingBusy);};
  await openEffects();await page.evaluate(()=>{const a=document.querySelector('.case-reel-track').getAnimations()[0];a.finish();});await page.waitForFunction(()=>caseReel===null);assert.equal(await page.locator('#case-result h3').textContent(),'Aurora');
  await openEffects();await page.evaluate(()=>window.dispatchEvent(new Event('resize')));assert.equal(await page.locator('.case-reel').count(),0);
  await openEffects();await page.evaluate(()=>selectGamblingTab('collection'));assert.equal(await page.evaluate(()=>caseReel),null);await page.evaluate(()=>selectGamblingTab('cases'));
  await page.emulateMedia({reducedMotion:'reduce'});await page.locator('[data-crate="cod-emblems"]').getByRole('button',{name:'Open crate · 100 Kash',exact:true}).click();await page.waitForFunction(()=>!gamblingBusy&&$('case-result').textContent.includes('Equip this emblem'));assert.equal(await page.locator('.case-reel').count(),0);assert.equal(await page.locator('#case-result').getByRole('button',{name:'Equip this emblem',exact:true}).isVisible(),true);await page.emulateMedia({reducedMotion:'no-preference'});
  const beforeLost=await page.evaluate(()=>state.wallet.balance);await page.evaluate(()=>loseReply=true);await page.locator('[data-crate="username-effects"]').getByRole('button',{name:'Open crate · 100 Kash',exact:true}).click();await page.waitForFunction(()=>!!gamblingPendingMutation&&!gamblingBusy);
  const pendingId=await page.evaluate(()=>gamblingPendingMutation.payload.request_id);await page.locator('#gambling-retry').click();await page.waitForFunction(()=>!gamblingBusy&&!gamblingPendingMutation&&!!caseReel);await page.locator('#case-result').getByRole('button',{name:'Skip animation',exact:true}).click();
  assert.equal(await page.evaluate(()=>state.wallet.balance),beforeLost-100,'Retry charged a second crate');assert.equal(await page.evaluate(id=>posts.filter(p=>p.payload.request_id===id).length,pendingId),2);
  for(const [caseId,kind] of [['avatar-frames','frame'],['bo2-calling-cards','banner']]){
   await page.locator(`[data-crate="${caseId}"]`).getByRole('button',{name:'Open crate · 100 Kash',exact:true}).click();await page.waitForFunction(()=>!!caseReel&&!gamblingBusy);
   const expected=await page.evaluate(id=>fixtureCatalog.cases.find(c=>c.id===id).items[0].id,caseId);assert.equal(await page.locator('.case-reel-item').nth(30).getAttribute('data-cosmetic-id'),expected);
   if(kind==='banner'){await page.waitForTimeout(300);await page.locator('#case-result').screenshot({path:`target/calling-card-reel-${width}.png`});}
   await page.evaluate(()=>document.querySelector('.case-reel-track').getAnimations()[0].finish());await page.waitForFunction(()=>!caseReel);assert.equal(await page.locator('#case-result').getByRole('button',{name:'Equip this '+(kind==='frame'?'avatar frame':'banner'),exact:true}).isVisible(),true);
  }


  await page.evaluate(()=>{applyProfileCosmetics({username:currentUser.username,avatar:false,cosmetics:{frame:ownedFixture[0],emblem:ownedFixture[1],name_effect:fixtureCatalog.catalog.find(i=>i.id==='name-effect-rainbow')}});$('profilename').textContent=currentUser.username;applyUsernameCosmetics($('profilename'),{emblem:ownedFixture[1],name_effect:fixtureCatalog.catalog.find(i=>i.id==='name-effect-rainbow')});$('profilesview').hidden=false;$('profilecard').hidden=false;$('gamblingview').hidden=true;for(const id of ['editprofile','avatarform','ratingform','profilecommentform','loggeddevices'])$(id).hidden=true;});
  await page.locator('#profilecard').screenshot({path:`target/profile-cosmetics-${width}.png`});
  await page.evaluate(()=>{$('profilesview').hidden=true;$('gamblingview').hidden=false;});
  // Lost response: retry the original cashout request, never create a new one.
  await page.evaluate(async()=>{holdPoll=false;state.crash={...state.crash,id:10,bet:{round_id:10,stake:100,status:'pending',auto_cashout:null}};await loadGambling();selectGamblingTab('crash');loseReply=true;await gamblingMutation('gambling/crash/cashout',{round_id:10}).catch(()=>{});});
  assert.equal(await page.evaluate(()=>!!gamblingPendingMutation),true);await page.evaluate(async()=>{const p=gamblingPendingMutation;await gamblingMutation(p.path,p.data,p.success);});
  const requests=await page.evaluate(()=>posts.filter(p=>p.path==='gambling/crash/cashout'&&p.payload.round_id===10));assert.equal(requests.length,2);assert.equal(requests[0].payload.request_id,requests[1].payload.request_id);assert.equal(await page.evaluate(()=>gamblingPendingMutation),null);
  await page.evaluate(()=>{selectGamblingTab('crash');drawCrashVisual(crashVisual.receivedAt+2200);});assert.equal(await page.locator('#crash-cashout').isDisabled(),true);
  await page.screenshot({path:`target/crash-cosmetics-${width}.png`,fullPage:true});assert.deepEqual(errors,[]);assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),true);
  console.log(`${width}px: live amount/RTT, immediate cashout, held/stale reads, 4 cosmetic slots, gradients, CS-style reel/skip/cleanup/retry and reduced motion passed`);await page.close();
 }}finally{await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
