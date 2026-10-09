'use strict';
let gamblingReady=false,gamblingTab='crash',gamblingData=null,gamblingBusy=false,gamblingLoading=false;
let gamblingLoadPromise=null;
let gamblingCatalog=null,gamblingCatalogPromise=null;
let gamblingReadGeneration=0,gamblingReadController=null,gamblingRouteSignal=null,gamblingRouteAbort=null;
let gamblingMemberMismatch=false;
let gamblingPendingMutation=null;
let gamblingCaseRenderKey='',gamblingCosmeticRenderKey='';
let caseReel=null,cosmeticPage=1,cosmeticStylesKey='';
const CRATE_NAMES=Object.freeze({'bo2-calling-cards':'BO2 calling cards crate','mw2-calling-cards':'MW2 calling cards crate','avatar-frames':'Avatar frames crate','cod-emblems':'Call of Duty emblems crate','username-effects':'Animated usernames crate','bo2-animated':'BO2 motion crate','mw2-canna':'MW2 green collection','premium-cosmetics':'Rare & legendary vault','rank-emblems':'Rank & prestige crate','canna-case':'Legacy mixed cosmetics case'});
let crashPeople={round:null,pages:1,rows:[],total:0,hasMore:false},crashPeopleLoading=false;
let crashPeopleRefreshPromise=null,crashPeopleRefreshPending=null;
let crashPeopleGeneration=0,crashPeopleController=null;
let gamblingAdminReady=false,gamblingAdminData=null,gamblingAdminLoading=false;
const CRASH_VISUAL_FRESH_MS=2000,CRASH_GROWTH_MS=10000;
const gamblingTimings=new WeakMap();
let crashLatencySamples=[];
let crashVisual=null,crashAnimation=null,crashMotionQuery=null,crashHooksReady=false;
let crashAutoRefresh={round:null,lastAt:-Infinity};
let crashLastPaint=-Infinity,crashHistoryKey='';
function crashClock(){return typeof performance!=='undefined'?performance.now():Date.now();}
function crashReducedMotion(){return !!crashMotionQuery?.matches;}
function crashIsVisible(){return !document.hidden&&!$('gamblingview')?.hidden&&!$('gambling-crash')?.hidden;}
function stopCrashAnimation(requireFresh=false){if(crashAnimation!==null&&typeof cancelAnimationFrame==='function')cancelAnimationFrame(crashAnimation);crashAnimation=null;if(requireFresh&&crashVisual)crashVisual.needsFresh=true;}
function crashSetText(id,text){const n=$(id);if(n&&n.textContent!==text)n.textContent=text;}
function crashSvg(tag,attributes){const n=typeof document.createElementNS==='function'?document.createElementNS('http://www.w3.org/2000/svg',tag):gameNode(tag);for(const [key,value]of Object.entries(attributes))n.setAttribute(key,value);return n;}
function drawCrashVisual(at=crashClock()){
 crashLastPaint=at;
 if(!crashVisual||!$('crash-multiplier'))return;
 const sample=crashVisual,age=Math.max(0,at-sample.receivedAt),stale=sample.needsFresh||age>=CRASH_VISUAL_FRESH_MS;
 // The flight is a short display estimate only. Bet/cashout state and amounts
 // always use the untouched server response in renderCrash/gamblingMutation.
 const elapsed=Math.max(0,sample.serverTime+sample.oneWayMs-sample.startsAt);
 const serverGrowth=Math.exp(Math.min(elapsed,CRASH_GROWTH_MS*Math.log(1000))/CRASH_GROWTH_MS);
 const base=serverGrowth;
 const estimate=sample.phase==='running'&&!crashReducedMotion()&&!sample.needsFresh?Math.min(1000,base*Math.exp(Math.min(age,CRASH_VISUAL_FRESH_MS)/CRASH_GROWTH_MS)):sample.multiplier;
 const value=sample.phase==='crashed'?sample.final:sample.phase==='running'?estimate:1;
 const progress=Math.max(0,Math.min(1,Math.log(value)/Math.log(1000)));
 crashSetText('crash-multiplier',multiplierText(value));
 updateCrashCashoutDisplay(value,stale,elapsed+Math.min(age,CRASH_VISUAL_FRESH_MS));
 const stage=$('crash-multiplier').parentElement;const staleValue=String(stale&&sample.phase!=='crashed'&&sample.phase!=='paused');if(stage.dataset.stale!==staleValue)stage.dataset.stale=staleValue;
 const seconds=Math.max(0,Math.ceil((sample.startsAt-sample.serverTime-sample.oneWayMs-Math.min(age,CRASH_VISUAL_FRESH_MS))/1000));
 const phase=sample.phase==='betting'?(seconds?`Taking bets · ${seconds}s`:'Starting round…'):sample.phase==='running'?'In flight':sample.phase==='paused'?'Wagering paused':'Crashed';
 crashSetText('crash-phase',stale&&(sample.phase==='running'||sample.phase==='betting')?'Syncing round…':phase);
 const syncText=sample.phase==='crashed'?'Final result confirmed by server':sample.phase==='paused'?'Waiting for wagering to resume':stale?'Waiting for a fresh server update':crashReducedMotion()?'Server updates · reduced motion':'';crashSetText('crash-sync',syncText);const sync=$('crash-sync');if(sync)sync.hidden=!syncText;
 const fill=$('crash-flight-fill');if(fill?.style)fill.style.transform=`scaleX(${progress.toFixed(5)})`;
 const line=$('crash-flight-line'),area=$('crash-flight-area'),dot=$('crash-flight-dot');
 if(line&&area&&dot){const points=[];for(let i=0;i<=32;i++){const p=progress*i/32;points.push(`${(12+p*576).toFixed(2)},${(168-140*p*p).toFixed(2)}`);}const path='M'+points.join(' L');line.setAttribute('d',path);area.setAttribute('d',path+` L${(12+progress*576).toFixed(2)},180 L12,180 Z`);dot.setAttribute('cx',(12+progress*576).toFixed(2));dot.setAttribute('cy',(168-140*progress*progress).toFixed(2));}
 return !stale;
}
function scheduleCrashAnimation(){
 if(crashAnimation!==null||!crashVisual||!crashIsVisible()||crashReducedMotion()||typeof requestAnimationFrame!=='function'||!['running','betting'].includes(crashVisual.phase)||crashVisual.needsFresh)return;
 if(crashClock()-crashVisual.receivedAt>=CRASH_VISUAL_FRESH_MS){drawCrashVisual();return;}
 crashAnimation=requestAnimationFrame(()=>{crashAnimation=null;if(!crashIsVisible()){stopCrashAnimation(true);return;}const mobile=window.matchMedia?.('(max-width: 650px), (pointer: coarse)').matches;if(mobile&&crashClock()-crashLastPaint<1000/30){scheduleCrashAnimation();return;}if(drawCrashVisual())scheduleCrashAnimation();});
}
function refreshCrashVisibility(){if(!crashIsVisible()){stopCrashAnimation(true);return;}drawCrashVisual();scheduleCrashAnimation();}
function requestCrashRefresh(){if(crashVisual&&crashIsVisible()&&!gamblingBusy)loadGambling().catch(error=>{if(error.name!=='AbortError')setGamblingStatus(error.message,true);});}
function setupCrashAnimation(){
 if(crashHooksReady)return;crashHooksReady=true;
 if(typeof window.matchMedia==='function'){crashMotionQuery=window.matchMedia('(prefers-reduced-motion: reduce)');const change=()=>{stopCrashAnimation();drawCrashVisual();scheduleCrashAnimation();};if(crashMotionQuery.addEventListener)crashMotionQuery.addEventListener('change',change);else crashMotionQuery.addListener?.(change);}
 document.addEventListener?.('visibilitychange',()=>{stopCrashAnimation(true);if(!document.hidden){drawCrashVisual();requestCrashRefresh();}});
 window.addEventListener?.('pagehide',()=>stopCrashAnimation(true));window.addEventListener?.('pageshow',()=>{if(crashVisual){drawCrashVisual();requestCrashRefresh();}});
}
function updateCrashVisual(crash,serverTime,timing){
 if(timing&&timing.receivedAt!==crashVisual?.receivedAt&&Number.isFinite(timing.rtt)&&timing.rtt>=0&&timing.rtt<=10000){crashLatencySamples.push(timing.rtt);crashLatencySamples=crashLatencySamples.slice(-8);}
 const oneWayMs=crashLatencySamples.length?Math.min(500,Math.min(...crashLatencySamples)/2):0;
 const time=Number(serverTime),starts=Number(crash.betting_ends_ms),multiplier=Number(crash.multiplier),final=Number(crash.crash_multiplier);
 if(!crashVisual||crashVisual.id!==crash.id||crashVisual.phase!==crash.phase||Number.isFinite(time)&&time>crashVisual.serverTime){
  stopCrashAnimation();crashVisual={id:crash.id,phase:crash.phase,serverTime:Number.isFinite(time)?time:0,startsAt:Number.isFinite(starts)?starts:Number.isFinite(time)?time:0,multiplier:Number.isFinite(multiplier)?Math.max(1,Math.min(1000,multiplier)):1,final:Number.isFinite(final)?Math.max(1,Math.min(1000,final)):1,receivedAt:timing?.receivedAt??crashClock(),oneWayMs,needsFresh:!Number.isFinite(time)};
 }
 drawCrashVisual();scheduleCrashAnimation();
}
function updateCrashCashoutDisplay(value,stale,elapsed){
 const crash=gamblingData?.crash,bet=crash?.bet,control=$('crash-cashout');if(!control)return;
 const active=crash?.phase==='running'&&bet?.status==='pending';
 control.disabled=gamblingBusy||!!gamblingPendingMutation||!active||stale;
 if(gamblingBusy&&gamblingPendingPath==='gambling/crash/cashout'){crashSetText('crash-cashout','Cashing out…');return;}
 const estimate=Math.floor((Number(bet?.stake)||0)*Math.floor(value*100)/100);
 crashSetText('crash-cashout',active?(stale?'Syncing cashout…':`Cash out · ~${kashText(estimate)} · ${multiplierText(value)}`):'Cash out');
 control.title=active?'Live estimate, adjusted for measured network delay. The server confirms the final payout on receipt. Auto cashout is scheduled on the server.':'';
 crashSetText('crash-ping',crashLatencySamples.length?`Connection · ${Math.round(Math.min(...crashLatencySamples))} ms`:'Measuring connection…');
 if(active&&bet.auto_cashout){const remaining=Math.max(0,Math.log(bet.auto_cashout)*CRASH_GROWTH_MS-elapsed);crashSetText('crash-your-bet',`${kashText(bet.stake)} in play · Auto ${multiplierText(bet.auto_cashout)}${remaining?` · ~${(remaining/1000).toFixed(1)}s`:' · Confirming result…'}`);if(!remaining&&!stale&&!gamblingBusy&&!gamblingLoading&&crashIsVisible()){const at=crashClock();if(crashAutoRefresh.round!==crash.id||at-crashAutoRefresh.lastAt>=500){crashAutoRefresh={round:crash.id,lastAt:at};loadGambling(true).catch(error=>{if(error.name!=='AbortError')setGamblingStatus(error.message,true);});}}}
}
function gameNode(tag,text,className){const n=document.createElement(tag);if(text!==undefined&&text!==null)n.textContent=text;if(className)n.className=className;return n;}
function gameField(id,label,value,min,max,step='1'){
 const wrapper=gameNode('label',label,'gamefield');const input=gameNode('input');input.id=id;input.type='number';input.value=value;input.min=min;input.max=max;input.step=step;input.required=true;wrapper.append(input);return wrapper;
}
function gameButton(label,callback,className){const b=button(label,callback);b.type='button';if(className)b.className=className;return b;}
function kashText(n){return Math.max(0,Number(n)||0).toLocaleString()+' Kash';}
function multiplierText(n){return Number(n||1).toFixed(2)+'×';}
function gameRequestId(){return crypto.randomUUID();}
function setGamblingStatus(text,error=false){const n=$('gambling-status');if(n){n.textContent=text;n.classList.toggle('is-error',error);}}
function setupGambling(){
 if(gamblingReady)return;const root=$('gamblingview');if(!root)return;gamblingReady=true;root.classList.add('gamblingworkspace');
 const hero=gameNode('header',null,'gamehero');const intro=gameNode('div');intro.append(gameNode('p','CANNA / AFTER HOURS','eyebrow'),gameNode('h2','A little luck. A lot of Kash.'),gameNode('p','Play nine Kash games, then turn your Kash into frames, banners, emblems and animated usernames. Kash is community play currency.'));
 const wallet=gameNode('div',null,'gamewallet');wallet.append(gameNode('small','YOUR WALLET'),gameNode('strong','—'));wallet.lastChild.id='gambling-balance';const daily=gameButton('Claim daily Kash',()=>gamblingMutation('gambling/daily',{}),'primary');daily.id='gambling-daily';wallet.append(daily);hero.append(intro,wallet);
 const nav=gameNode('nav',null,'gametabs');nav.setAttribute('aria-label','Kash games');
 const panes=gameNode('div',null,'gamepanes');for(const [id,name]of [['crash','↗ Crash'],['blackjack','♠ Blackjack'],['roulette','◎ Roulette'],['dice','⚄ Dice'],['slots','▥ Slots'],['keno','✦ Keno'],['plinko','⋮ Plinko'],['wheel','◉ Wheel'],['baccarat','♦ Baccarat'],['history','◷ Recent results'],['cases','◇ Cosmetic crates'],['collection','▣ My collection']]){const tab=gameButton(name,()=>selectGamblingTab(id));tab.dataset.game=id;nav.append(tab);const pane=gameNode('section');pane.id='gambling-'+id;pane.hidden=id!==gamblingTab;panes.append(pane);}
 const status=gameNode('p');status.id='gambling-status';status.setAttribute('role','status');const retry=gameButton('Retry pending action',()=>{const pending=gamblingPendingMutation;return pending&&gamblingMutation(pending.path,pending.data,pending.success);});retry.id='gambling-retry';retry.hidden=true;root.append(hero,nav,status,retry,panes);setupCrash();setupBlackjack();setupArcade();setupCases();setupCosmeticCollection();selectGamblingTab(gamblingTab);
}
function selectGamblingTab(id){gamblingTab=id;if(id!=='cases')caseReel?.finish();if(id!=='crash')cancelCrashParticipantReads();document.querySelectorAll('.gamepanes>section').forEach(p=>p.hidden=p.id!=='gambling-'+id);document.querySelectorAll('.gametabs button').forEach(b=>{const active=b.dataset.game===id;b.classList.toggle('active',active);b.setAttribute('aria-current',active?'page':'false');});refreshCrashVisibility();if(id==='crash'&&crashVisual&&(crashVisual.needsFresh||crashClock()-crashVisual.receivedAt>=CRASH_VISUAL_FRESH_MS))requestCrashRefresh();}
function acceptGamblingCatalog(data){
 if(!data||!/^[a-f0-9]{64}$/.test(data.version||'')||!Array.isArray(data.catalog)||!data.catalog.length||data.catalog.length>2000||!Array.isArray(data.cases)||!data.cases.length||data.cases.length>16)throw new Error('Cosmetic catalog could not load. Refresh to try again.');
 gamblingCatalog={version:data.version,catalog:data.catalog,cases:data.cases};return gamblingCatalog;
}
function gamblingAbortError(){const error=new Error('This page request was cancelled.');error.name='AbortError';return error;}
function gamblingRouteActive(){return typeof location==='undefined'||location.pathname==='/gambling';}
function readGamblingJson(path,signal){
 const sentAt=crashClock();
 if(signal?.aborted)return Promise.reject(gamblingAbortError());
 return new Promise((resolve,reject)=>{
  const abort=()=>reject(gamblingAbortError());signal?.addEventListener('abort',abort,{once:true});
  Promise.resolve().then(()=>{if(signal?.aborted)throw gamblingAbortError();return api(path,signal?{signal}:{});}).then(response=>response.json()).then(data=>{if(signal?.aborted)throw gamblingAbortError();if(data&&typeof data==='object'){const receivedAt=crashClock();gamblingTimings.set(data,{receivedAt,rtt:receivedAt-sentAt});}resolve(data);},reject).finally(()=>signal?.removeEventListener('abort',abort)).catch(reject);
 });
}
function cancelGamblingPrimaryReads(){
 const controller=gamblingReadController;gamblingReadGeneration++;gamblingReadController=null;gamblingLoading=false;gamblingLoadPromise=null;gamblingCatalogPromise=null;controller?.abort();
}
function cancelCrashParticipantReads(){
 const controller=crashPeopleController;crashPeopleGeneration++;crashPeopleController=null;crashPeopleLoading=false;crashPeopleRefreshPromise=null;crashPeopleRefreshPending=null;controller?.abort();
 if($('crash-people-more'))$('crash-people-more').disabled=false;
}
function observeGamblingRoute(){
 if(typeof navigationSignal!=='function'||!gamblingRouteActive())return;
 const signal=navigationSignal();if(signal===gamblingRouteSignal){if(signal?.aborted)throw gamblingAbortError();return;}
 gamblingRouteSignal?.removeEventListener('abort',gamblingRouteAbort);gamblingRouteSignal=signal;
 gamblingRouteAbort=()=>{caseReel?.finish();cancelGamblingPrimaryReads();cancelCrashParticipantReads();stopCrashAnimation(true);};
 signal?.addEventListener('abort',gamblingRouteAbort,{once:true});if(signal?.aborted){gamblingRouteAbort();throw gamblingAbortError();}
}
function gamblingIdentityError(){const error=new Error('Your signed-in account changed. Reload this page before using Kash or your collection.');error.status=409;return error;}
function rejectGamblingMember(){
 gamblingMemberMismatch=true;gamblingData=null;gamblingPendingMutation=null;cancelGamblingPrimaryReads();cancelCrashParticipantReads();
 if($('gambling-balance'))$('gambling-balance').textContent='—';
 for(const id of ['cosmetic-collection','case-result','crash-people-rows'])$(id)?.replaceChildren();
 for(const control of $('gamblingview')?.querySelectorAll('button')||[])control.disabled=true;
 setGamblingStatus(gamblingIdentityError().message,true);throw gamblingIdentityError();
}
function gamblingMember(){
 const member=currentUser?.id;
 if(gamblingMemberMismatch||!Number.isSafeInteger(member)||member<=0||gamblingData&&gamblingData.member_id!==member||gamblingPendingMutation&&gamblingPendingMutation.member_id!==member)rejectGamblingMember();
 return member;
}
function verifyGamblingMember(data,expected){if(data?.member_id!==expected||currentUser?.id!==expected)rejectGamblingMember();}
async function loadGamblingCatalog(signal){
 if(signal?.aborted)throw gamblingAbortError();if(gamblingCatalog)return gamblingCatalog;
 if(gamblingCatalogPromise)return gamblingCatalogPromise;
 const promise=(async()=>{try{return acceptGamblingCatalog(await readGamblingJson('gambling/cosmetics/catalog',signal));}finally{if(gamblingCatalogPromise===promise)gamblingCatalogPromise=null;}})();gamblingCatalogPromise=promise;return promise;
}
function attachGamblingCatalog(data){
 const cosmetics=data?.cosmetics;
 if(!cosmetics||!Array.isArray(cosmetics.owned)||!cosmetics.equipped)throw new Error('Your collection could not load. Refresh to try again.');
 if(Array.isArray(cosmetics.catalog))acceptGamblingCatalog({version:cosmetics.catalog_version,catalog:cosmetics.catalog,cases:data.cases});
 if(!gamblingCatalog||cosmetics.catalog_version!==gamblingCatalog.version){gamblingCatalog=null;throw new Error('The cosmetic catalog changed. Refresh to load its current version.');}
 cosmetics.catalog=gamblingCatalog.catalog;const caseRule=data.rules?.games?.find(r=>r.game==='cases');data.cases=gamblingCatalog.cases.map(c=>{const settings=data.rules?.crates?.find(p=>p.case_id===c.id),cost=settings?.cost??c.cost,catalog=new Map(cosmetics.catalog.map(i=>[i.id,i])),weighted=(c.items||[]).map(drop=>({...drop,weight:drop.odds_percent*(settings?.rarity_factors?.[catalog.get(drop.id)?.rarity||'common']??100)})),total=weighted.reduce((sum,d)=>sum+d.weight,0),items=weighted.filter(d=>d.weight>0).map(d=>({id:d.id,odds_percent:100*d.weight/total}));return {...c,cost,items,item_count:items.length,available:total>0&&c.available!==false&&!data.rules?.paused&&(caseRule?.enabled??true)&&cost>=(caseRule?.min_stake??1)&&cost<=(caseRule?.max_stake??1000000)};});return data;
}
async function loadGambling(crashOnly=false){
 setupGambling();if(!gamblingReady||!gamblingRouteActive())return;observeGamblingRoute();const member=gamblingMember();
 if(gamblingLoading)return gamblingLoadPromise;
 const generation=++gamblingReadGeneration,controller=new AbortController();gamblingReadController=controller;gamblingLoading=true;
 const promise=(async()=>{try{
  const compact=crashOnly&&!!gamblingData;const catalog=compact?null:await loadGamblingCatalog(controller.signal);const response=await readGamblingJson(compact?'gambling?crash_only=true':'gambling?catalog_version='+catalog.version,controller.signal);
  if(generation!==gamblingReadGeneration||controller.signal.aborted)throw gamblingAbortError();verifyGamblingMember(response,member);
  if(compact){gamblingData={...gamblingData,crash:response.crash,wallet:response.wallet,server_time_ms:response.server_time_ms};gamblingTimings.set(gamblingData,gamblingTimings.get(response));}else{gamblingData=attachGamblingCatalog(response);}renderGambling(gamblingData,compact);queueCrashParticipantRefresh(gamblingData.crash);
 }finally{if(generation===gamblingReadGeneration){gamblingLoading=false;gamblingReadController=null;gamblingLoadPromise=null;}}})();gamblingLoadPromise=promise;return promise;
}
function renderGambling(data,crashOnly=false){
 $('gambling-balance').textContent=kashText(data.wallet.balance);$('kashbalance').textContent=kashText(data.wallet.balance);if(currentUser)currentUser.kash=data.wallet.balance;
 $('gambling-daily').disabled=gamblingBusy||!!gamblingPendingMutation||!data.wallet.daily_available;$('gambling-daily').textContent=data.wallet.daily_available?'Claim daily Kash':'Daily already claimed';
 $('gambling-retry').hidden=!gamblingPendingMutation||gamblingBusy;$('gambling-retry').disabled=gamblingBusy;
 renderCrash(data.crash,data.server_time_ms,gamblingTimings.get(data));if(crashOnly)return;renderBlackjack(data.blackjack);renderArcade(data);
 const casesKey=JSON.stringify([data.cosmetics.catalog_version,data.wallet.balance,data.rules,gamblingBusy,!!gamblingPendingMutation,!!caseReel]);
 if(casesKey!==gamblingCaseRenderKey){renderCases(data);gamblingCaseRenderKey=casesKey;}
 const cosmeticKey=JSON.stringify([data.cosmetics.catalog_version,data.cosmetics.owned,data.cosmetics.equipped,data.cosmetics.collection,gamblingBusy,!!gamblingPendingMutation]);
 if(cosmeticKey!==gamblingCosmeticRenderKey){renderCosmeticCollection(data.cosmetics);gamblingCosmeticRenderKey=cosmeticKey;}
}
let gamblingPendingPath=null;
async function gamblingMutation(path,data,success){
 if(gamblingBusy)return;const member=gamblingMember();
 const key=JSON.stringify({path,data});
 if(gamblingPendingMutation&&gamblingPendingMutation.key!==key)throw new Error('Retry your pending action before starting another. Its original request is preserved to prevent a second charge.');
 const pending=gamblingPendingMutation||{path,data,success,key,member_id:member,payload:{...data,request_id:gameRequestId()}};
 gamblingBusy=true;gamblingPendingPath=path;setGamblingStatus(path==='gambling/crash/cashout'?'Cashing out…':'Working…');if(gamblingData)renderGambling(gamblingData);let recorded=false,submitted=false;
 // A delayed display poll must never hold up a time-sensitive cashout. Its
 // eventual response is discarded; a confirmed mutation gets a fresh snapshot.
 cancelGamblingPrimaryReads();
 try{
  if(currentUser?.id!==pending.member_id)rejectGamblingMember();submitted=true;
  const result=await json(path,pending.payload,{headers:{'X-Canna-Member':String(pending.member_id)}});recorded=true;gamblingPendingMutation=null;
  if(currentUser?.id!==pending.member_id)rejectGamblingMember();
  if(path==='gambling/cases/open'&&result.item?.id&&gamblingData){const old=gamblingData.cosmetics.owned.find(item=>item.id===result.item.id);if(old)old.count=result.count;else gamblingData.cosmetics.owned.push({id:result.item.id,count:result.count});}
  if(gamblingData){if(Number.isSafeInteger(result.balance)&&result.balance>=0)gamblingData.wallet.balance=result.balance;if(result.bet&&gamblingData.crash.id===pending.payload.round_id){gamblingData.crash.bet=result.bet;const person=crashPeople.rows.find(row=>row.user_id===pending.member_id);if(person)Object.assign(person,result.bet,{cashout_elapsed_ms:Number.isFinite(result.bet.cashout_at_ms)?Math.max(0,result.bet.cashout_at_ms-gamblingData.crash.betting_ends_ms):null});}renderGambling(gamblingData);}
  if(path==='gambling/cosmetics/equip'&&result.equipped&&gamblingData){gamblingData.cosmetics.equipped=result.equipped;if(currentUser){currentUser.cosmetics=Object.fromEntries(Object.entries(result.equipped).map(([kind,id])=>[kind,gamblingData.cosmetics.catalog.find(item=>item.id===id)||null]));if(typeof renderAccountBadges==='function')renderAccountBadges();}renderGambling(gamblingData);}
  setGamblingStatus(typeof success==='function'?success(result):success||'Done.');return result;
 }catch(error){
  if(!recorded&&submitted&&(!Number.isInteger(error.status)||error.status>=500)){gamblingPendingMutation=pending;setGamblingStatus('The connection ended before the result was confirmed. Retry the pending action; Canna will reuse the same request and prevent a second charge.',true);}
  else{gamblingPendingMutation=null;setGamblingStatus(gamblingMemberMismatch?gamblingIdentityError().message:recorded?'Your action was recorded. The latest balance could not load; refresh this page.':error.message,true);}
  throw error;
 }finally{gamblingBusy=false;gamblingPendingPath=null;if(gamblingData&&!gamblingMemberMismatch)renderGambling(gamblingData);if(recorded&&gamblingRouteActive())loadGambling(path.startsWith('gambling/crash/')).catch(error=>{if(error.name!=='AbortError'&&!gamblingBusy)setGamblingStatus('Action confirmed. Refreshing the display failed; it will retry.',true);});}
}
function setupCrash(){
 const root=$('gambling-crash');const layout=gameNode('div',null,'crashlayout');const stage=gameNode('div',null,'crashstage');stage.setAttribute('aria-label','Current Crash round');
 const phase=gameNode('span','Loading round…','crashphase');phase.id='crash-phase';const multiplier=gameNode('strong','1.00×','crashmultiplier');multiplier.id='crash-multiplier';const detail=gameNode('p');detail.id='crash-detail';
 const chart=crashSvg('svg',{viewBox:'0 0 600 180',preserveAspectRatio:'none',class:'crashchart','aria-hidden':'true'});const area=crashSvg('path',{class:'crashchart-area'});area.id='crash-flight-area';const line=crashSvg('path',{class:'crashchart-line',fill:'none','vector-effect':'non-scaling-stroke'});line.id='crash-flight-line';const dot=crashSvg('circle',{class:'crashchart-dot',r:5});dot.id='crash-flight-dot';chart.append(area,line,dot);
 const flight=gameNode('div',null,'crashflight');flight.setAttribute('aria-hidden','true');const track=gameNode('div',null,'crashflight-track');const fill=gameNode('span',null,'crashflight-fill');fill.id='crash-flight-fill';track.append(fill);const scale=gameNode('div',null,'crashflight-scale');scale.append(gameNode('span','1×'),gameNode('span','1,000×'));flight.append(track,scale);const sync=gameNode('p','Waiting for server…','crashsync');sync.id='crash-sync';stage.append(chart,phase,multiplier,detail,flight,sync);setupCrashAnimation();
 const control=gameNode('section',null,'gamepanel');control.append(gameNode('h3','Your next flight'),gameNode('p','Join during the countdown. Cash out before the crash, or choose an automatic target. At the crash the round is over.'));
 const form=gameNode('form');form.id='crash-bet-form';form.append(gameField('crash-stake','Stake (Kash)',25,1,1000000),gameField('crash-auto','Auto cashout (×)',2,1.01,1000,'0.01'));const auto=gameNode('label',null,'check');const autoToggle=gameNode('input');autoToggle.type='checkbox';autoToggle.checked=true;autoToggle.id='crash-auto-enabled';auto.append(autoToggle,document.createTextNode('Use auto cashout'));form.append(auto);
 const bet=gameNode('button','Place bet');bet.type='submit';bet.className='primary';bet.id='crash-place-bet';form.append(bet);form.addEventListener('submit',e=>{e.preventDefault();action(async()=>{if(!gamblingData)return;await gamblingMutation('gambling/crash/bet',{round_id:gamblingData.crash.id,stake:Number($('crash-stake').value),auto_cashout:$('crash-auto-enabled').checked?Number($('crash-auto').value):null},'Bet placed.');});});
 const cashout=gameButton('Cash out',()=>gamblingMutation('gambling/crash/cashout',{round_id:gamblingData.crash.id},r=>r.bet?.status==='won'?`Cashed out at ${multiplierText(r.bet.cashout_multiplier)} · ${kashText(r.bet.payout)}`:r.bet?.status==='lost'?'The round crashed before the server received the cashout.':'Cashout recorded.'),'gamecashout');cashout.id='crash-cashout';const betStatus=gameNode('p');betStatus.id='crash-your-bet';betStatus.setAttribute('role','status');const ping=gameNode('small','Measuring connection…','game-footnote');ping.id='crash-ping';control.append(form,cashout,betStatus,ping);layout.append(stage,control);
 const people=gameNode('section',null,'gamepanel crashpeople');const heading=gameNode('div',null,'crashpeople-heading');const count=gameNode('p');count.id='crash-people-count';heading.append(gameNode('h3','Players this round'),count);const scroll=gameNode('div',null,'crashpeople-scroll');const table=gameNode('table');table.setAttribute('aria-label','Crash bets and cashouts');const head=gameNode('thead');const headings=gameNode('tr');for(const label of ['Player','Bet','Result'])headings.append(gameNode('th',label));head.append(headings);const body=gameNode('tbody');body.id='crash-people-rows';table.append(head,body);scroll.append(table);const empty=gameNode('p','No bets yet.');empty.id='crash-people-empty';const more=gameButton('Show more players',async()=>{if(crashPeopleLoading)return;crashPeople.pages++;await loadGambling();});more.id='crash-people-more';more.hidden=true;const status=gameNode('p');status.id='crash-people-status';status.setAttribute('role','status');people.append(heading,scroll,empty,more,status,gameNode('p','Cashouts stay here until the next round.','game-footnote'));
 const history=gameNode('div',null,'crashhistory');history.id='crash-history';root.append(layout,people,gameNode('h3','Recent rounds'),history);
}
function renderCrash(crash,now,timing){
 const phase=crash.phase;const stage=$('crash-multiplier').parentElement;stage.dataset.phase=phase;const seconds=Math.max(0,Math.ceil((crash.betting_ends_ms-now)/1000));
 $('crash-phase').textContent=phase==='betting'?`Taking bets · ${seconds}s`:phase==='running'?'In flight':phase==='paused'?'Wagering paused':'Crashed';
 $('crash-detail').textContent=`${crash.id===null?'Waiting for next round':'Round '+String(crash.id).slice(0,12)}${crash.paused?' · New wagering paused':''}`;
 const hasBet=!!crash.bet;$('crash-place-bet').disabled=gamblingBusy||!!gamblingPendingMutation||crash.paused||phase!=='betting'||hasBet;$('crash-cashout').disabled=gamblingBusy||!!gamblingPendingMutation||phase!=='running'||!hasBet||crash.bet.status!=='pending';
 $('crash-your-bet').textContent=!hasBet?'No bet in this round.':crash.bet.status==='pending'?`${kashText(crash.bet.stake)} in play${crash.bet.auto_cashout?' · Auto '+multiplierText(crash.bet.auto_cashout):''}`:crash.bet.status==='won'?`Cashed out · ${kashText(crash.bet.payout)}`:`Crashed · ${kashText(crash.bet.stake)} lost`;
 updateCrashVisual(crash,now,timing);
 const history=(crash.history||[]).slice(0,15),historyKey=JSON.stringify(history);if(historyKey!==crashHistoryKey){crashHistoryKey=historyKey;$('crash-history').replaceChildren(...history.map(r=>{const n=gameNode('span',multiplierText(r.crash_multiplier),'crashchip '+(r.crash_multiplier>=10?'high':r.crash_multiplier<2?'low':'mid'));n.title=`Round ${r.id}`;return n;}));}
 acceptCrashParticipants(crash);renderCrashParticipants(crash.phase);
}
function acceptCrashParticipants(crash){
 if(crashPeople.round!==crash.id){cancelCrashParticipantReads();crashPeople={round:crash.id,pages:1,rows:[],total:0,hasMore:false};if($('crash-people-status'))$('crash-people-status').textContent='';}
 const first=Array.isArray(crash.participants)?crash.participants:[];
 // Retain loaded later pages until their fresh snapshots arrive. Round changes
 // replace the whole list; a crash or wagering pause retains the current round.
 const later=crashPeople.rows.filter(row=>first.length&&row.user_id>first.at(-1).user_id);
 crashPeople.rows=[...first,...later];crashPeople.total=Number(crash.participant_count)||first.length;
 crashPeople.hasMore=crashPeople.rows.length<crashPeople.total;
}
function queueCrashParticipantRefresh(crash){
 if(!crashIsVisible()||crash.id!==crashPeople.round||crashPeople.pages<2||!crash.participant_has_more||crash.id===null)return;
 crashPeopleRefreshPending=crash;if(crashPeopleLoading)return;
 const latest=crashPeopleRefreshPending;crashPeopleRefreshPending=null;crashPeopleRefreshPromise=refreshCrashParticipantPages(latest);
}
async function refreshCrashParticipantPages(crash){
 if(crashPeople.pages<2||!crash.participant_has_more||crash.id===null)return;
 const round=crash.id,pages=crashPeople.pages,member=gamblingMember(),generation=++crashPeopleGeneration,controller=new AbortController(),rows=[];
 let cursor=crash.participant_next_after_user_id,more=!!crash.participant_has_more;crashPeopleController=controller;crashPeopleLoading=true;renderCrashParticipants(crash.phase);
 try{for(let page=1;page<pages&&more;page++){
  const result=await readGamblingJson(`gambling?crash_round_id=${encodeURIComponent(round)}&crash_after_user_id=${encodeURIComponent(cursor)}`,controller.signal);
  if(generation!==crashPeopleGeneration||controller.signal.aborted||crashPeople.round!==round||result.crash.id!==round)return;
  verifyGamblingMember(result,member);rows.push(...(result.crash.participants||[]));more=!!result.crash.participant_has_more;cursor=result.crash.participant_next_after_user_id;
 }
 if(generation===crashPeopleGeneration&&crashPeople.round===round){
  // Primary state and cashout mutations never wait for this secondary list.
  // Keep a newer first page and never regress an already settled bet to pending.
  const merged=new Map(crashPeople.rows.map(row=>[row.user_id,row]));
  for(const row of rows){const old=merged.get(row.user_id);if(old&&old.status!=='pending'&&row.status==='pending')continue;merged.set(row.user_id,row);}
  crashPeople.rows=[...merged.values()].sort((a,b)=>a.user_id-b.user_id);crashPeople.hasMore=crashPeople.rows.length<crashPeople.total;$('crash-people-status').textContent='';
 }
 }catch(error){if(error.name!=='AbortError'&&generation===crashPeopleGeneration&&crashPeople.round===round)$('crash-people-status').textContent=gamblingMemberMismatch?gamblingIdentityError().message:error.status===409?'The next round started. Refreshing players…':'Player updates could not load. They will retry with the next update.';}
 finally{if(generation===crashPeopleGeneration){crashPeopleLoading=false;crashPeopleController=null;renderCrashParticipants(gamblingData?.crash.phase||crash.phase);if(crashPeopleRefreshPending){const latest=crashPeopleRefreshPending;crashPeopleRefreshPending=null;queueCrashParticipantRefresh(latest);}}}
}
function renderCrashParticipants(phase){
 if(!$('crash-people-rows'))return;
 $('crash-people-count').textContent=`${crashPeople.total} ${crashPeople.total===1?'player':'players'}${crashPeople.rows.length<crashPeople.total?' · '+crashPeople.rows.length+' shown':''}`;
 $('crash-people-empty').hidden=!!crashPeople.rows.length;
 $('crash-people-more').hidden=!crashPeople.hasMore;$('crash-people-more').disabled=crashPeopleLoading;
 const body=$('crash-people-rows'),existing=new Map([...body.children].map(row=>[Number(row.dataset.userId),row]));
 const rows=crashPeople.rows.map(person=>{
  const key=JSON.stringify([crashPeople.round,person,person.status==='pending'&&phase==='betting']),old=existing.get(person.user_id);if(old?.crashRenderKey===key)return old;
  const row=gameNode('tr');row.dataset.userId=person.user_id;const player=gameNode('td'),stake=gameNode('td',kashText(person.stake),'crashpeople-stake'),result=gameNode('td',null,'crashpeople-result');
  const name=person.display_name||person.username||'Unavailable member';
  if(Number.isSafeInteger(person.user_id)&&person.user_id>0&&person.profile_url===`/members/${person.user_id}`){const link=gameNode('a',name);link.href=person.profile_url;link.dataset.page=person.profile_url;player.append(link);}else player.textContent=name;
  result.dataset.status=person.status;
  if(person.status==='won'){result.append(gameNode('strong','Cashed out · '+multiplierText(person.cashout_multiplier)));if(Number.isFinite(person.cashout_elapsed_ms)&&person.cashout_elapsed_ms>=0)result.append(gameNode('small',(person.cashout_elapsed_ms/1000).toFixed(2)+'s into the round'));if(Number.isFinite(person.cashout_at_ms)){result.title='Cashed out '+new Date(person.cashout_at_ms).toLocaleString();}}
  else result.textContent=person.status==='lost'?'Crashed':phase==='betting'?'Waiting':'In play';
  row.append(player,stake,result);row.crashRenderKey=key;return row;
 });
 const retained=new Set(rows);for(const row of [...body.children])if(!retained.has(row))row.remove();
 rows.forEach((row,index)=>{if(body.children[index]!==row)body.insertBefore(row,body.children[index]||null);});
}
function setupBlackjack(){
 const root=$('gambling-blackjack');const table=gameNode('div',null,'blackjacktable');const dealer=gameNode('section');dealer.append(gameNode('h3','Dealer'),gameNode('div',null,'playingcards'),gameNode('p'));dealer.children[1].id='blackjack-dealer';dealer.lastChild.id='blackjack-dealer-total';const player=gameNode('section');player.append(gameNode('h3','Your hand'),gameNode('div',null,'playingcards'),gameNode('p'));player.children[1].id='blackjack-player';player.lastChild.id='blackjack-player-total';const outcome=gameNode('p',null,'blackjackoutcome');outcome.id='blackjack-outcome';outcome.setAttribute('role','status');table.append(dealer,outcome,player);
 const controls=gameNode('div',null,'gamepanel');const form=gameNode('form');form.append(gameField('blackjack-stake','Stake (Kash)',25,1,1000000));const deal=gameNode('button','Deal a hand');deal.id='blackjack-deal';deal.type='submit';deal.className='primary';form.append(deal);form.addEventListener('submit',e=>{e.preventDefault();action(()=>gamblingMutation('gambling/blackjack/deal',{stake:Number($('blackjack-stake').value)},'Hand dealt.'));});const actions=gameNode('div',null,'row');for(const [id,label]of [['hit','Hit'],['stand','Stand'],['double','Double down']]){const b=gameButton(label,()=>gamblingMutation('gambling/blackjack/action',{hand_id:gamblingData.blackjack.id,action:id},'Hand updated.'));b.id='blackjack-'+id;actions.append(b);}controls.append(form,actions,gameNode('p','Get closer to 21 than the dealer without going over. Dealer stands on 17. Blackjack pays 3:2; a regular win pays 1:1. One hand at a time.'));
 const layout=gameNode('div',null,'blackjacklayout');layout.append(table,controls);root.append(layout);
}
function playingCard(card){
 const n=gameNode('div',null,'playingcard');if(card.hidden){n.classList.add('cardback');n.append(gameNode('span','C'));n.setAttribute('aria-label','Face-down dealer card');return n;}
 const suits={clubs:'♣',diamonds:'♦',hearts:'♥',spades:'♠',C:'♣',D:'♦',H:'♥',S:'♠','♣':'♣','♦':'♦','♥':'♥','♠':'♠'};const suit=suits[card.suit]||card.suit;n.classList.toggle('red',suit==='♥'||suit==='♦');n.append(gameNode('strong',String(card.rank)),gameNode('span',suit));n.setAttribute('aria-label',`${card.rank} of ${card.suit}`);return n;
}
function renderBlackjack(hand){
 $('blackjack-dealer').replaceChildren(...(hand?.dealer||[{hidden:true},{hidden:true}]).map(playingCard));$('blackjack-player').replaceChildren(...(hand?.cards||[]).map(playingCard));$('blackjack-player-total').textContent=hand?`Total ${hand.player_total} · ${kashText(hand.stake)} stake`:'Place a stake and deal your first hand.';$('blackjack-dealer-total').textContent=hand?.dealer_total!==undefined?`Total ${hand.dealer_total}`:'Dealer hole card stays hidden until the hand ends.';
 const active=hand?.status==='playing';const messages={playing:'Your move',won:'You win',lost:'Dealer wins',push:'Push · stake returned',blackjack:'Blackjack!'};$('blackjack-outcome').textContent=hand?(messages[hand.status]||hand.status)+(active?'':` · ${kashText(hand.payout)} returned`):'Ready when you are';
 $('blackjack-deal').disabled=gamblingBusy||!!gamblingPendingMutation||active;for(const id of ['hit','stand'])$('blackjack-'+id).disabled=gamblingBusy||!!gamblingPendingMutation||!active;$('blackjack-double').disabled=gamblingBusy||!!gamblingPendingMutation||!active||!hand.can_double;
}
function setupCases(){const root=$('gambling-cases');root.append(gameNode('h3','Choose your crate'),gameNode('p','Choose calling cards, motion cards, the green collection, frames, emblems, username effects or the rare & legendary vault. Check every item and its rarity odds before spending Kash. Spare copies can be recycled from My collection.'),gameNode('div',null,'casegrid'));root.lastChild.id='case-list';const result=gameNode('section',null,'case-result');result.id='case-result';result.hidden=true;result.setAttribute('role','status');root.append(result);}
function cosmeticKindLabel(kind){return {frame:'avatar frame',banner:'banner',emblem:'emblem',name_effect:'username effect'}[kind]||'cosmetic';}
function cosmeticPreview(item,large=false){
 if(item.kind==='name_effect'){const preview=gameNode('div',null,'cosmeticpreview nameeffectpreview'+(large?' large':''));const name=gameNode('strong',currentUser?.username||'Canna');if(typeof applyUsernameCosmetics==='function')applyUsernameCosmetics(name,{name_effect:item});preview.append(name);preview.dataset.rarity=item.rarity;return preview;}
 const preview=gameNode('div',null,'cosmeticpreview '+(large?'large ':'')+(item.kind==='banner'?'bannerpreview':'framepreview'));const style=String(item.style||item.id||'');if(/^[a-z0-9_-]{1,80}$/.test(style))preview.classList.add('cosmetic-'+style);
 if(item.kind==='frame')preview.append(gameNode('span',currentUser?.username?.slice(0,1).toUpperCase()||'C','cosmeticinitial'));else if(item.kind==='banner')preview.append(gameNode('span','CANNA','cosmeticbannertext'));else preview.classList.add('emblempreview');
 if(item.kind==='banner'&&['mw2','bo2'].includes(cosmeticCollection(item))){preview.classList.add('calling-card');preview.title=item.name;const width=typeof callingCardDisplayWidth==='function'?callingCardDisplayWidth(item):0;if(width)preview.style.maxWidth=width+'px';}
 const asset=item.animated&&typeof window.matchMedia==='function'&&window.matchMedia('(prefers-reduced-motion: reduce)').matches&&item.poster_asset?item.poster_asset:item.asset;
 if(typeof asset==='string'&&/^\/api\/v1\/cosmetics\/assets\/[a-zA-Z0-9_-]+$/.test(asset)){const img=gameNode('img');const hash=asset===item.poster_asset?item.poster_sha256:item.sha256;img.src=asset+(/^[a-f0-9]{64}$/.test(hash||'')?'?v='+hash:'');if(typeof bindCosmeticMotion==='function')bindCosmeticMotion(img,item);img.alt='';img.loading='lazy';img.decoding='async';img.className='cosmeticasset';preview.append(img);}
 preview.dataset.rarity=item.rarity||'common';preview.setAttribute('aria-label',item.name+' '+item.kind+' preview');return preview;
}
function showCrateDrop(item,crate,catalog){
 caseReel?.finish();
 const result=$('case-result');result.hidden=false;result.replaceChildren();
 const label=gameNode('p','Opening '+crate.name+'…','eyebrow'),details=gameNode('div');details.hidden=true;
 if(item?.paused)details.append(gameNode('p','This calling card is saved, but its collection is currently paused.'));
 else if(item)details.append(cosmeticPreview(item,true),gameNode('h3',item.name),gameNode('p',`${item.rarity} ${cosmeticKindLabel(item.kind)}${item.animated?' · Animated':''} · Added to your collection`),gameButton('Equip this '+cosmeticKindLabel(item.kind),()=>equipCosmetic(item.kind,item.id),'primary'));
 else details.append(gameNode('h3','Cosmetic added to your collection'));
 details.append(gameButton('View my collection',()=>selectGamblingTab('collection')));result.append(label,details);
 const reduced=window.matchMedia?.('(prefers-reduced-motion: reduce)'),pool=(crate.items||[]).map(drop=>({item:catalog.get(drop.id),weight:Number(drop.odds_percent)||0})).filter(drop=>drop.item&&!drop.item.paused&&drop.weight>0);
 if(!item||item.paused||!pool.length||reduced?.matches||!gamblingRouteActive()||gamblingTab!=='cases'||document.hidden){label.textContent='YOUR DROP';details.hidden=false;return;}
 // The POST has already saved the server-selected award. These weighted
 // neighboring cards only decorate the reel; they never select or change it.
 const total=pool.reduce((sum,drop)=>sum+drop.weight,0),pick=()=>{let position=Math.random()*total;for(const drop of pool){position-=drop.weight;if(position<=0)return drop.item;}return pool.at(-1).item;};
 const viewport=gameNode('div',null,'case-reel'),track=gameNode('div',null,'case-reel-track'),marker=gameNode('span',null,'case-reel-marker'),winner=30;
 viewport.setAttribute('aria-hidden','true');
 for(let index=0;index<36;index++){const cosmetic=index===winner?item:pick(),cell=gameNode('div',null,'case-reel-item');cell.dataset.rarity=cosmetic.rarity||'common';cell.dataset.cosmeticId=cosmetic.id;cell.append(cosmeticPreview(cosmetic),gameNode('strong',cosmetic.name),gameNode('small',cosmetic.rarity));track.append(cell);}
 viewport.append(track,marker);const skip=gameButton('Skip animation',()=>caseReel?.finish()),note=gameNode('small','Your cosmetic is already saved. Skip to reveal it now.','game-footnote');result.insertBefore(viewport,details);result.insertBefore(skip,details);result.insertBefore(note,details);
 let animation=null,done=false;const visibility=()=>{if(document.hidden)finish();},motion=()=>{if(reduced?.matches)finish();},resize=()=>finish();
 const finish=()=>{if(done)return;done=true;animation?.cancel();document.removeEventListener('visibilitychange',visibility);window.removeEventListener('resize',resize);window.removeEventListener('pagehide',finish);reduced?.removeEventListener?.('change',motion);if(caseReel?.finish===finish)caseReel=null;viewport.remove();skip.remove();note.remove();label.textContent='YOUR DROP';details.hidden=false;if(gamblingData&&!gamblingMemberMismatch)renderGambling(gamblingData);};
 caseReel={finish};document.addEventListener('visibilitychange',visibility);window.addEventListener('resize',resize,{once:true});window.addEventListener('pagehide',finish,{once:true});reduced?.addEventListener?.('change',motion);
 result.scrollIntoView({block:'nearest',behavior:'instant'});
 const first=track.firstElementChild,cellWidth=first.getBoundingClientRect().width,gap=parseFloat(getComputedStyle(track).gap)||0,stride=cellWidth+gap,center=viewport.clientWidth/2-cellWidth/2;
 const from=center-2*stride,to=center-winner*stride;
 if(!cellWidth||typeof track.animate!=='function'){finish();return;}
 animation=track.animate([{transform:`translateX(${from}px)`},{transform:`translateX(${to}px)`}],{duration:5200,easing:'cubic-bezier(.10,.65,.12,1)',fill:'forwards'});
 animation.finished.then(finish,()=>{});
}
function renderCases(data){
 const catalog=new Map(data.cosmetics.catalog.map(item=>[item.id,item]));
 $('case-list').replaceChildren(...data.cases.map(c=>{
  const card=gameNode('section',null,'gamepanel casecard');card.dataset.crate=c.id;
  const collection=c.collection||'',glyph=collection==='bo2'?'II':collection==='mw2'?'MW2':'◇';
  card.append(gameNode('span',glyph,'caseglyph'),gameNode('h3',c.name),gameNode('p',`${c.items.length} ${['all','mixed'].includes(c.kind)?'cosmetics':c.kind==='emblem'?'emblems':c.kind==='name_effect'?'username effects':c.kind==='frame'||collection==='frames'?'avatar frames':'calling cards'} · ${kashText(c.cost)}`));
  const tierOdds=new Map();for(const drop of c.items||[]){const tier=catalog.get(drop.id)?.rarity||'common';tierOdds.set(tier,(tierOdds.get(tier)||0)+drop.odds_percent);}const tiers=gameNode('div',null,'rarityodds');for(const [tier,odds]of tierOdds){const chip=gameNode('span',`${tier} ${odds.toFixed(2)}%`);chip.dataset.rarity=tier;tiers.append(chip);}card.append(tiers);
  if(c.paused){card.append(gameNode('p',c.pause_reason||'This collection is paused. Owned items are saved.'));const paused=gameButton('Paused',()=>{});paused.disabled=true;card.append(paused);return card;}
  const samples=gameNode('div',null,'cratepreviews');const sampleIds=[...(c.items||[])].sort((a,b)=>Number(!!catalog.get(b.id)?.animated)-Number(!!catalog.get(a.id)?.animated)).slice(0,3);
  for(const drop of sampleIds){const item=catalog.get(drop.id);if(item)samples.append(cosmeticPreview(item));}card.append(samples);
  const open=gameButton('Open crate · '+kashText(c.cost),async()=>{
   if(caseReel)return;
   await gamblingMutation('gambling/cases/open',{case_id:c.id},r=>{
    const item=typeof r.item==='string'?catalog.get(r.item):catalog.get(r.item?.id)||r.item;showCrateDrop(item,c,catalog);return 'Crate opened. Your cosmetic is saved in your collection.';
   });
  },'primary');open.disabled=gamblingBusy||!!gamblingPendingMutation||!!caseReel||c.available===false||data.wallet.balance<c.cost;if(c.available===false)open.textContent='Crate unavailable';card.append(open);
  const odds=gameNode('details');odds.append(gameNode('summary','See every item & drop odds'),gameNode('p','Shown chances are rounded to two decimals.'));const list=gameNode('div',null,'caseodds');odds.append(list);let rendered=false;
  odds.addEventListener('toggle',()=>{if(!odds.open||rendered)return;rendered=true;for(const drop of c.items){const item=catalog.get(drop.id);if(!item)continue;const row=gameNode('div');row.append(cosmeticPreview(item),gameNode('span',item.name+(item.animated?' · Animated':'')),gameNode('strong',Number(drop.odds_percent).toFixed(2)+'%'));list.append(row);}});
  card.append(odds);return card;
 }));
}
function cosmeticCollection(item){return item.collection||(item.kind==='frame'?'frames':String(item.id).startsWith('mw2-')?'mw2':String(item.id).startsWith('bo2-')?'bo2':'canna');}
function setupCosmeticCollection(){
 const root=$('gambling-collection'),tools=gameNode('div',null,'row collectiontools'),kind=gameNode('select');kind.id='cosmetic-kind';kind.setAttribute('aria-label','Filter cosmetic type');kind.append(new Option('All cosmetic types',''),new Option('Avatar frames','frame'),new Option('Profile banners','banner'),new Option('Emblems','emblem'),new Option('Username effects','name_effect'));
 const collection=gameNode('select');collection.id='cosmetic-source';collection.setAttribute('aria-label','Filter cosmetic collection');collection.append(new Option('All collections',''),new Option('BO2 calling cards','bo2'),new Option('MW2 calling cards','mw2'),new Option('Avatar frames','frames'),new Option('MW2 emblems','mw2-emblems'),new Option('CoD rank emblems','cod-ranks'),new Option('Username effects','username-effects'),new Option('Canna originals','canna'));
 const owned=gameNode('label',null,'check'),check=gameNode('input');check.type='checkbox';check.id='cosmetic-owned-only';check.checked=true;owned.append(check,document.createTextNode('Only my collection'));
 for(const input of [kind,collection,check])input.addEventListener('change',()=>{cosmeticPage=1;if(gamblingData)renderCosmeticCollection(gamblingData.cosmetics);});
 const search=gameNode('input');search.type='search';search.id='cosmetic-search';search.placeholder='Find a cosmetic…';search.maxLength=80;search.setAttribute('aria-label','Search cosmetics');search.addEventListener('input',()=>{cosmeticPage=1;if(gamblingData)renderCosmeticCollection(gamblingData.cosmetics);});
 const rarity=gameNode('select');rarity.id='cosmetic-rarity';rarity.setAttribute('aria-label','Filter rarity');rarity.append(new Option('All rarities',''));for(const r of ['common','uncommon','rare','epic','legendary'])rarity.append(new Option(r,r));
 const sort=gameNode('select');sort.id='cosmetic-sort';sort.setAttribute('aria-label','Sort cosmetics');for(const [id,name]of [['catalog','Catalog order'],['name','Name A–Z'],['rarity','Rarest first'],['copies','Most copies']])sort.append(new Option(name,id));
 const favorites=gameNode('label',null,'check'),favoriteCheck=gameNode('input');favoriteCheck.type='checkbox';favoriteCheck.id='cosmetic-favorites-only';favorites.append(favoriteCheck,document.createTextNode('Favorites only'));
 for(const input of [rarity,sort,favoriteCheck])input.addEventListener('change',()=>{cosmeticPage=1;if(gamblingData)renderCosmeticCollection(gamblingData.cosmetics);});
 tools.append(search,kind,collection,rarity,sort,owned,favorites,gameButton('Remove frame',()=>equipCosmetic('frame',null)),gameButton('Remove banner',()=>equipCosmetic('banner',null)),gameButton('Remove emblem',()=>equipCosmetic('emblem',null)),gameButton('Remove username effect',()=>equipCosmetic('name_effect',null)),gameButton('View my profile',()=>openProfile(currentUser.id)));
 const summary=gameNode('p');summary.id='cosmetic-collection-summary';summary.setAttribute('role','status');const grid=gameNode('div',null,'cosmeticgrid');grid.id='cosmetic-collection';const paging=gameNode('div',null,'row');paging.id='cosmetic-paging';paging.append(gameButton('Previous cosmetics',()=>{cosmeticPage=Math.max(1,cosmeticPage-1);renderCosmeticCollection(gamblingData.cosmetics);}),gameNode('span'),gameButton('Next cosmetics',()=>{cosmeticPage++;renderCosmeticCollection(gamblingData.cosmetics);}));
 root.append(gameNode('h3','Make your profile yours'),gameNode('p','Equip your unlocked cosmetics here. Save five complete looks, favorite your favorites and recycle spare copies for Kash. Your first copy and equipped cosmetics are kept. Uncheck Only my collection to preview locked items. Animated cards play in previews and profiles. Reduced motion shows a still frame.'),summary,tools,paging,grid);const styles=gameNode('section',null,'gamepanel');styles.id='cosmetic-styles';root.insertBefore(styles,grid);
}
function renderCosmeticCollection(cosmetics){
 const inventory=new Map(cosmetics.owned.map(i=>[i.id,i.count])),kind=$('cosmetic-kind').value,collection=$('cosmetic-source').value,only=$('cosmetic-owned-only').checked,query=$('cosmetic-search').value.trim().toLowerCase();
 const favoriteIds=new Set(cosmetics.collection?.favorites||[]),rarity=$('cosmetic-rarity').value,sort=$('cosmetic-sort').value,favorites=$('cosmetic-favorites-only').checked;
 const items=cosmetics.catalog.filter(i=>!i.paused&&(!rarity||i.rarity===rarity)&&(!favorites||favoriteIds.has(i.id))&&(!kind||kind===i.kind)&&(!collection||collection===cosmeticCollection(i))&&(!only||inventory.has(i.id))&&(!query||`${i.name} ${i.kind} ${i.rarity} ${cosmeticCollection(i)} ${i.animated?'animated':''}`.toLowerCase().includes(query)));
 const ranks={common:1,uncommon:2,rare:3,epic:4,legendary:5};if(sort==='name')items.sort((a,b)=>a.name.localeCompare(b.name));if(sort==='rarity')items.sort((a,b)=>(ranks[b.rarity]||0)-(ranks[a.rarity]||0)||a.name.localeCompare(b.name));if(sort==='copies')items.sort((a,b)=>(inventory.get(b.id)||0)-(inventory.get(a.id)||0));renderCosmeticStyles(cosmetics);
 const saved=cosmetics.catalog.filter(i=>i.paused&&inventory.has(i.id)).length;
 $('cosmetic-collection-summary').textContent=`${inventory.size} unlocked cosmetic${inventory.size===1?'':'s'} · ${items.length} shown.${saved?' '+saved+' paused item'+(saved===1?'':'s')+' saved.':''} ${gamblingPendingMutation?'Confirm the pending action before changing your cosmetics.':'Equip one frame, banner, emblem and username effect together.'}`;
 const pages=Math.max(1,Math.ceil(items.length/48));cosmeticPage=Math.min(cosmeticPage,pages);const paging=$('cosmetic-paging');paging.children[0].disabled=cosmeticPage<=1;paging.children[1].textContent=`Page ${cosmeticPage} of ${pages}`;paging.children[2].disabled=cosmeticPage>=pages;
 $('cosmetic-collection').replaceChildren(...items.slice((cosmeticPage-1)*48,cosmeticPage*48).map(item=>{
  const card=gameNode('article',null,'cosmeticcard'),count=inventory.get(item.id)||0,equipped=cosmetics.equipped[item.kind]===item.id;card.dataset.cosmeticId=item.id;
  card.append(cosmeticPreview(item,true),gameNode('h3',item.name),gameNode('p',`${item.rarity} · ${cosmeticKindLabel(item.kind)}${item.animated?' · Animated':''}${count?' · Owned '+count:' · Locked — unlock from a crate'}`));
  const equip=gameButton(equipped?'Equipped':count?'Equip '+cosmeticKindLabel(item.kind):'View cosmetic crate',()=>count?equipCosmetic(item.kind,item.id):selectGamblingTab('cases'),equipped?'equipped':'');equip.disabled=gamblingBusy||!!gamblingPendingMutation||equipped;card.append(equip);
  if(count){const favorite=gameButton(favoriteIds.has(item.id)?'★ Favorited':'☆ Favorite',()=>gamblingMutation('gambling/cosmetics/manage',{action:'favorite',item_id:item.id,favorite:!favoriteIds.has(item.id)},'Favorite updated.'));favorite.disabled=gamblingBusy||!!gamblingPendingMutation;card.append(favorite);if(count>1){const reward=cosmetics.collection?.recycle_values?.[item.rarity]||5,recycle=gameButton('Recycle spare · '+kashText(reward),async()=>{if(await cannaConfirm('Recycle one spare copy of '+item.name+' for '+kashText(reward)+'? Your first copy is kept.'))await gamblingMutation('gambling/cosmetics/manage',{action:'recycle',item_id:item.id},r=>'Spare recycled for '+kashText(r.reward)+'.');});recycle.disabled=gamblingBusy||!!gamblingPendingMutation;card.append(recycle);}}
  return card;
 }));
 if(!items.length){const empty=gameNode('section',null,'cosmeticempty');empty.append(gameNode('p',inventory.size?'No cosmetics match this filter.':'Your collection is empty. Opening a crate unlocks an item you can equip.'),gameButton('Browse cosmetic crates',()=>selectGamblingTab('cases')));$('cosmetic-collection').append(empty);}
}
async function equipCosmetic(kind,id){if(!gamblingData)return;const kinds=['frame','banner','emblem','name_effect'];if(!kinds.includes(kind))throw new Error('Choose a valid cosmetic type.');if(id&&gamblingData.cosmetics.catalog.some(item=>item.id===id&&item.paused))throw new Error('This cosmetic collection is paused due to artwork quality.');if(id&&!gamblingData.cosmetics.owned.some(item=>item.id===id&&item.count>0))throw new Error('Unlock this cosmetic from a crate before equipping it.');const equipped=Object.fromEntries(kinds.map(slot=>[slot,gamblingData.cosmetics.equipped[slot]||null]));equipped[kind]=id;await gamblingMutation('gambling/cosmetics/equip',equipped,`${cosmeticKindLabel(kind)} ${id?'equipped':'removed'}.`);}

function arcadeRule(game){return gamblingData?.rules?.games?.find(r=>r.game===game)||{enabled:true,min_stake:1,max_stake:1000000,payout_percent:game==='dice'?95:100};}
function gameIsPaused(game){return !!gamblingData?.rules?.paused||!arcadeRule(game).enabled;}
const INSTANT_GAMES=['roulette','dice','slots','keno','plinko','wheel','baccarat'];
const SLOT_SYMBOLS=['💎','7','🍀','🔔','🍒','🍋'];
function renderCosmeticStyles(cosmetics){
 const root=$('cosmetic-styles');if(!root)return;const key=JSON.stringify(cosmetics.collection?.styles||[]);if(cosmeticStylesKey===key&&root.children.length){for(const n of root.querySelectorAll('button'))n.disabled=gamblingBusy||!!gamblingPendingMutation||n.dataset.empty==='true';return;}cosmeticStylesKey=key;root.replaceChildren(gameNode('h3','Saved looks'));
 for(let slot=1;slot<=5;slot++){
  const style=cosmetics.collection?.styles?.find(s=>s.slot===slot),row=gameNode('div',null,'row'),name=gameNode('input');name.type='text';name.maxLength=40;name.value=style?.name||'';name.placeholder='Look '+slot;name.setAttribute('aria-label','Name style '+slot);
  const save=gameButton('Save slot '+slot,()=>gamblingMutation('gambling/cosmetics/manage',{action:'save_style',slot,name:name.value},'Look saved.'));
  const load=gameButton('Wear '+(style?.name||'slot '+slot),()=>gamblingMutation('gambling/cosmetics/manage',{action:'load_style',slot},'Saved look equipped.'));
  const remove=gameButton('Delete saved look',()=>gamblingMutation('gambling/cosmetics/manage',{action:'delete_style',slot},'Saved look deleted.'));
  load.dataset.empty=remove.dataset.empty=String(!style);save.disabled=gamblingBusy||!!gamblingPendingMutation;load.disabled=remove.disabled=!style||gamblingBusy||!!gamblingPendingMutation;row.append(name,save,load,remove);root.append(row);
 }
}
const arcadeSeen=new Map();
const PLINKO_TABLES={low:[2,1.5,1.2,1.1,1,1,.8,1,1,1.1,1.2,1.5,2],high:[500,50,10,3,1,.5,.2,.5,1,3,10,50,500]};
const WHEEL_SEGMENTS=[0,1,0,2,0,1,0,5,0,1,0,2,0,1,0,10,0,1,0,1];
function plinkoFrames(path){
 const gravity=1400,bounceVelocity=-70,fallTime=height=>(-bounceVelocity+Math.sqrt(bounceVelocity*bounceVelocity+2*gravity*height))/gravity;
 const frames=[{transform:'translate(180px, 5px)',offset:0}],flight=fallTime(17),first=Math.sqrt(15/gravity),last=fallTime(30.5),total=first+11*flight+last;
 let x=180,y=12.5,elapsed=first;for(let i=1;i<=6;i++){const t=first*i/6;frames.push({transform:`translate(180px, ${5+.5*gravity*t*t}px)`,offset:t/total});}
 for(let row=0;row<12;row++){
  const dt=row===11?last:flight,dy=row===11?230-y:17,direction=path[row]?1:-1;
  for(let i=1;i<=10;i++){const t=dt*i/10;frames.push({transform:`translate(${x+direction*12.5*t/dt}px, ${y+bounceVelocity*t+.5*gravity*t*t}px)`,offset:Math.min(1,(elapsed+t)/total)});}
  x+=direction*12.5;y+=dy;elapsed+=dt;
 }
 frames.at(-1).offset=1;return {frames,duration:total*1000,x,y};
}
function prizeWheel(root,result){
 const stage=gameNode('div',null,'wheel-stage'),pointer=gameNode('span','▼','wheel-pointer'),svg=crashSvg('svg',{viewBox:'0 0 320 320',role:'img','aria-label':result?`Prize wheel: segment ${result.slot+1}, ${result.base_multiplier} times`:'Prize wheel ready: twenty labeled segments',class:'wheel-disc'});
 for(let i=0;i<20;i++){const a=(i*18-90)*Math.PI/180,b=((i+1)*18-90)*Math.PI/180,m=(a+b)/2;svg.append(crashSvg('path',{d:`M160 160 L${160+145*Math.cos(a)} ${160+145*Math.sin(a)} A145 145 0 0 1 ${160+145*Math.cos(b)} ${160+145*Math.sin(b)} Z`,fill:['#233c34','#415d42','#31554a','#726a35'][i%4],stroke:'#0f2018','stroke-width':2}));const label=crashSvg('text',{x:160+115*Math.cos(m),y:165+115*Math.sin(m),fill:'#e9f3c8','font-size':13,'text-anchor':'middle'});label.textContent=WHEEL_SEGMENTS[i]+'×';svg.append(label);}
 svg.append(crashSvg('circle',{cx:160,cy:160,r:24,fill:'#d2e599',stroke:'#18271b','stroke-width':4}));stage.append(svg,pointer);root.append(stage,gameNode('strong',result?result.base_multiplier+'× return before payout factor':'Ready to spin'));
 if(result){const angle=-(result.slot*18+9);svg.style.transform=`rotate(${angle}deg)`;root.append(gameNode('small',`Segment ${result.slot+1} of 20`));if(!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches)svg.animate?.([{transform:`rotate(${angle-1080}deg)`},{transform:`rotate(${angle}deg)`}],{duration:1900,easing:'cubic-bezier(.12,.65,.2,1)'});}
}
function renderArcadeVisual(game,row){
 const root=$(game+'-visual'),r=row?.result;
 const risk=$('plinko-choice')?.value||'low',key=JSON.stringify(row||{idle:true,risk:game==='plinko'?risk:''});if(arcadeSeen.get(game)===key)return;arcadeSeen.set(game,key);root.replaceChildren();
 if(!r&&!['plinko','wheel'].includes(game)){
  if(game==='slots')root.textContent=SLOT_SYMBOLS.slice(0,3).join('  ');
  else if(game==='roulette'){const grid=gameNode('div',null,'roulette-preview');for(let n=0;n<=36;n++)grid.append(gameNode('span',String(n)));root.append(grid,gameNode('small','Choose a bet to spin'));}
  else if(game==='dice'){const face=gameNode('div','⚄','dice-preview');root.append(face,gameNode('small','Choose your chance to roll'));}
  else if(game==='keno'){for(let n=0;n<10;n++)root.append(gameNode('span','?','kenoball'));root.append(gameNode('small','Your ten drawn numbers will appear here'));}
  else if(game==='baccarat'){for(const side of ['Player','Banker']){const hand=gameNode('div',null,'baccarathand');hand.append(gameNode('small',side));for(let n=0;n<2;n++)hand.append(gameNode('span','♠','baccaratcard card-back'));root.append(hand);}root.append(gameNode('small','Choose Player, Banker or Tie to deal'));}
  return;
 }
 if(['roulette','dice','slots'].includes(game)){root.textContent=game==='roulette'?String(r.number):game==='dice'?Number(r.roll).toFixed(2):(r.reels||[]).map(i=>SLOT_SYMBOLS[i]).join('  ');return;}
 if(game==='keno'){for(const n of r.draw||[]){const ball=gameNode('span',String(n),'kenoball');ball.dataset.hit=String((r.picks||[]).includes(n));root.append(ball);}root.append(gameNode('strong',`${r.hits} matches`));}
 if(game==='baccarat'){for(const side of ['player','banker']){const hand=gameNode('div',null,'baccarathand');hand.append(gameNode('small',side.toUpperCase()));for(const n of r[side]||[])hand.append(gameNode('span',String(n),'baccaratcard'));hand.append(gameNode('strong','Total '+r[side+'_total']));root.append(hand);}root.append(gameNode('strong',r.push?'Tie · stake returned':r.winner+' wins'));}
 if(game==='wheel')prizeWheel(root,r);
 if(game==='plinko'){
  const table=r?.multipliers||PLINKO_TABLES[risk],svg=crashSvg('svg',{viewBox:'0 0 360 260',role:'img','aria-label':r?`Plinko ${r.risk} risk, slot ${r.slot+1}, ${r.base_multiplier} times`:`Plinko ${risk} risk, ready to drop`});
  for(let row=0;row<12;row++)for(let col=0;col<=row;col++)svg.append(crashSvg('circle',{cx:180+(col-row/2)*25,cy:20+row*17,r:2.5,fill:'#83baae'}));
  const trajectory=r?plinkoFrames(r.path):null,ball=crashSvg('circle',{cx:0,cy:0,r:5,fill:'#ecffba',stroke:'#18271b','stroke-width':1.5,transform:`translate(${trajectory?.x||180} ${trajectory?.y||5})`,class:'plinko-ball'});svg.append(ball);
  if(trajectory&&!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches)ball.animate?.(trajectory.frames,{duration:trajectory.duration,easing:'linear'});
  for(let i=0;i<13;i++){svg.append(crashSvg('rect',{x:18+i*25,y:235,width:24,height:21,rx:3,fill:i===r?.slot?'#527144':table[i]<1?'#453b31':'#263f35'}));const text=crashSvg('text',{x:30+i*25,y:249,'text-anchor':'middle',fill:i===r?.slot?'#e7ffc8':'#d3e0d8','font-size':10});text.textContent=table[i]+'×';svg.append(text);}root.append(svg,gameNode('strong',r?r.base_multiplier+'× return before payout factor':'Choose risk and drop a ball'));
 }
}
function setupArcade(){
 for(const [game,title,description]of [['roulette','Roulette','European wheel: 0–36. Color, parity and range bets return 2×; a straight number returns 36× before the payout factor. Zero loses outside bets.'],['dice','Dice','Choose a roll-under chance from 2% to 95%. Rolls run from 0.00 to 99.99; a roll exactly at your threshold loses.'],['slots','Slots','Three independent reels, six equally likely symbols. Three matches return 50× / 25× / 15× / 10× / 8× / 5×. Two matches return 1×.'],['keno','Keno','Pick four of 40 numbers. Ten balls are drawn without replacement. Match 0 / 1 / 2 / 3 / 4 to return 0× / 0× / 1× / 5× / 50× before the payout factor.'],['plinko','Plinko','Twelve independent left/right bounces. Select low or high risk; edge slots are much rarer than center slots. The server saves the path and payout before the animation.'],['wheel','Wheel','Twenty equally likely segments. Six return 1×, two return 2×, one returns 5×, one returns 10× and ten return 0× before the payout factor.'],['baccarat','Baccarat','Fresh eight-deck shoe. Player returns 2×, Banker 1.95×, Tie 9× before the payout factor. Ties return Player/Banker stakes; standard third-card rules, naturals stand. Cards show point values, with 10/J/Q/K worth zero.']]){
  const root=$('gambling-'+game);root.append(gameNode('h3',title),gameNode('p',description));
  const visual=gameNode('div',game==='slots'?SLOT_SYMBOLS.slice(0,3).join('  '):'—','arcadevisual '+game);visual.id=game+'-visual';visual.setAttribute('aria-live','polite');root.append(visual);
  const panel=gameNode('section',null,'gamepanel'),form=gameNode('form',null,'arcadeform');form.append(gameField(game+'-stake','Stake (Kash)',25,1,1000000));
  if(game==='roulette'){
   const label=gameNode('label','Your bet'),choice=gameNode('select');choice.id='roulette-choice';for(const [value,name]of [['red','Red'],['black','Black'],['even','Even'],['odd','Odd'],['low','1–18'],['high','19–36'],['number','Single number']])choice.append(new Option(name,value));label.append(choice);const numberField=gameField('roulette-number','Number (0–36)',0,0,36);numberField.hidden=true;form.append(label,numberField);choice.addEventListener('change',()=>{numberField.hidden=choice.value!=='number';});
  }
  if(game==='dice'){const underField=gameField('dice-under','Roll under (%)',50,2,95);form.append(underField);underField.querySelector('input').addEventListener('input',renderArcadeRules);}
  if(game==='keno'){
   const grid=gameNode('div',null,'kenogrid');grid.id='keno-picks';grid.setAttribute('role','group');grid.setAttribute('aria-label','Choose four Keno numbers');
   for(let n=1;n<=40;n++){const pick=gameButton(String(n),()=>{const selected=grid.querySelectorAll('[aria-pressed=true]');if(pick.getAttribute('aria-pressed')==='true')pick.setAttribute('aria-pressed','false');else if(selected.length<4)pick.setAttribute('aria-pressed','true');renderArcadeRules();});pick.dataset.number=n;pick.setAttribute('aria-pressed',n<=4?'true':'false');grid.append(pick);}form.append(grid);
  }
  if(game==='plinko'||game==='baccarat'){const label=gameNode('label',game==='plinko'?'Risk':'Your bet'),choice=gameNode('select');choice.id=game+'-choice';for(const value of game==='plinko'?['low','high']:['player','banker','tie'])choice.append(new Option(value[0].toUpperCase()+value.slice(1),value));label.append(choice);form.append(label);choice.addEventListener('change',()=>{renderArcadeRules();if(game==='plinko'&&!gamblingData?.recent_games?.some(row=>row.game==='plinko'))renderArcadeVisual('plinko',null);});}
  const rules=gameNode('p');rules.id=game+'-rules';const submit=gameNode('button',game==='roulette'?'Spin wheel':({dice:'Roll dice',slots:'Spin reels',keno:'Draw ten balls',plinko:'Drop ball',wheel:'Spin prize wheel',baccarat:'Deal Baccarat'}[game]||'Play'));submit.type='submit';submit.className='primary';submit.id=game+'-play';form.append(rules,submit);form.addEventListener('submit',event=>{event.preventDefault();action(async()=>{
   const data={game,stake:Number($(game+'-stake').value)};
   if(game==='roulette'){data.choice=$('roulette-choice').value;if(data.choice==='number')data.number=Number($('roulette-number').value);}
   if(game==='dice')data.under=Number($('dice-under').value);
   if(game==='keno'){data.picks=[...$('keno-picks').querySelectorAll('[aria-pressed=true]')].map(n=>Number(n.dataset.number));if(data.picks.length!==4)throw new Error('Pick exactly four numbers.');}
   if(game==='plinko'||game==='baccarat')data.choice=$(game+'-choice').value;
   await gamblingMutation('gambling/arcade/play',data,result=>`${title}: ${kashText(result.payout)} returned on ${kashText(result.stake)}.`);
  });});panel.append(form);const result=gameNode('p');result.id=game+'-result';result.setAttribute('role','status');root.append(panel,result);
 }
 const root=$('gambling-history');root.append(gameNode('h3','Your latest arcade results'),gameNode('p','Your last 20 instant arcade games are saved here, including after a refresh.'),gameNode('div',null,'arcadehistory'));root.lastChild.id='arcade-history';
}
function arcadeResultText(row){const r=row.result||{};const outcome=row.game==='keno'?`${r.hits} hits · Draw ${(r.draw||[]).join(', ')}`:row.game==='plinko'?`Slot ${r.slot+1} · ${r.risk} risk · ${r.base_multiplier}×`:row.game==='wheel'?`Segment ${r.slot+1} · ${r.base_multiplier}×`:row.game==='baccarat'?`Player ${r.player_total} / Banker ${r.banker_total} · ${r.winner}${r.push?' · push':''}`:row.game==='roulette'?`${r.number} ${r.color} · ${r.choice}${r.pick!==null&&r.pick!==undefined?' '+r.pick:''}`:row.game==='dice'?`${Number(r.roll).toFixed(2)} · roll under ${r.under}`:(r.reels||[]).map(i=>SLOT_SYMBOLS[i]||'?').join(' ');return `${row.game}: ${outcome} · ${kashText(row.stake)} staked · ${kashText(row.payout)} returned`;}
function renderArcadeRules(){
 for(const game of INSTANT_GAMES){
  const rule=arcadeRule(game),chance=Number($('dice-under')?.value)||50;
  const extra={keno:'Pick four numbers; payouts 0 / 0 / 1 / 5 / 50× for 0–4 hits.',plinko:'Low: 2 / 1.5 / 1.2 / 1.1 / 1 / 1 / 0.8× mirrored. At factor 100%, only the center slot is below 1× (22.56%); expected return 97.55% before rounding. High: 500 / 50 / 10 / 3 / 1 / 0.5 / 0.2× mirrored. Bounces keep the same binomial odds.',wheel:'Each of the 20 segments has a 5% chance.',baccarat:'Player 2× · Banker 1.95× · Tie 9×; ties push outside bets.'};
  const odds=extra[game]?`${extra[game]} Payout factor ${rule.payout_percent}%.`:game==='dice'?`Win chance ${chance}%. Total return ${(rule.payout_percent/chance).toFixed(2)}× on a win. Expected return ${rule.payout_percent}% before whole-Kash rounding.`:game==='roulette'?`Payout factor ${rule.payout_percent}%. Expected return ${(36/37*rule.payout_percent).toFixed(2)}% before rounding.`:`Payout factor ${rule.payout_percent}%. Expected return ${(203/216*rule.payout_percent).toFixed(2)}% before rounding.`;
  $(game+'-rules').textContent=`${gameIsPaused(game)?'New wagers paused. ':''}Stake ${rule.min_stake.toLocaleString()}–${rule.max_stake.toLocaleString()} Kash. ${odds} Returns include your stake.`;
  $(game+'-stake').min=rule.min_stake;$(game+'-stake').max=rule.max_stake;$(game+'-play').disabled=gamblingBusy||!!gamblingPendingMutation||gameIsPaused(game)||(game==='keno'&&$('keno-picks').querySelectorAll('[aria-pressed=true]').length!==4);
 }
 for(const game of ['crash','blackjack']){const r=arcadeRule(game);$(game+'-stake').min=r.min_stake;$(game+'-stake').max=r.max_stake;}
 if(gameIsPaused('blackjack'))$('blackjack-deal').disabled=true;
 if(gameIsPaused('crash'))$('crash-place-bet').disabled=true;
}
function renderArcade(data){
 renderArcadeRules();const history=data.recent_games||[];
 $('arcade-history').replaceChildren(...history.map(row=>{const item=gameNode('article',null,'arcaderesult');item.dataset.win=String(row.payout>row.stake);item.append(gameNode('p',arcadeResultText(row)),gameNode('small',new Date(row.created*1000).toLocaleString()));return item;}));
 if(!history.length)$('arcade-history').append(gameNode('p','No arcade games yet.'));
 for(const game of INSTANT_GAMES){const row=history.find(r=>r.game===game);$(game+'-result').textContent=row?arcadeResultText(row):'';renderArcadeVisual(game,row);$(game+'-visual').dataset.color=row?.result?.color||'';}
}
function setupHouseAdmin(root){
 const form=gameNode('form',null,'adminsettingcard');form.id='admin-house-form';form.append(gameNode('h3','Games, limits & payouts'),gameNode('p','Changes apply to new wagers only. Existing Crash cashouts and Blackjack hands can finish. Payout factors for the arcade games are displayed to players.'));
 const pause=gameNode('label',null,'adminswitch'),toggle=gameNode('input');toggle.type='checkbox';toggle.id='admin-house-paused';pause.append(toggle,gameNode('span','Pause all new wagers'));form.append(pause,gameField('admin-house-daily','New games per person / UTC day',200,1,500));
 const grid=gameNode('div',null,'houserules');grid.id='admin-house-games';form.append(grid,gameNode('h3','Crate prices'),gameNode('p','Set prices for each themed crate. Costs must fit the Cases stake limits; existing saved price overrides are preserved.'));const crates=gameNode('div',null,'houserules');crates.id='admin-house-crates';form.append(crates);
 const save=gameNode('button','Save game controls');save.type='submit';save.className='primary';const status=gameNode('p');status.id='admin-house-status';status.setAttribute('role','status');form.append(save,status);
 form.addEventListener('submit',event=>{event.preventDefault();action(async()=>{const games=[...grid.children].map(row=>({game:row.dataset.game,enabled:row.querySelector('[data-field="enabled"]').checked,min_stake:Number(row.querySelector('[data-field="min_stake"]').value),max_stake:Number(row.querySelector('[data-field="max_stake"]').value),payout_percent:Number(row.querySelector('[data-field="payout_percent"]').value)}));const prices=[...crates.children].map(row=>({case_id:row.dataset.caseId,cost:Number(row.querySelector('[data-field=cost]').value),rarity_factors:Object.fromEntries([...row.querySelectorAll('[data-rarity]')].map(n=>[n.dataset.rarity,Number(n.value)]))}));save.disabled=true;status.textContent='Saving…';try{const result=await json('admin/gambling/rules',{paused:toggle.checked,daily_limit:Number($('admin-house-daily').value),games,crates:prices});renderHouseEditor(result.rules);status.textContent='Game controls saved. Changes are logged.';await refreshGamblingAdminPreview();}finally{save.disabled=false;}});});
 root.append(form,gameNode('h3','Activity'),gameNode('div',null,'housemetrics'));root.lastChild.id='admin-house-metrics';
}
function renderHouseEditor(rules){
 if(!rules)return;$('admin-house-paused').checked=rules.paused;$('admin-house-daily').value=rules.daily_limit;
 $('admin-house-games').replaceChildren(...rules.games.map(rule=>{const row=gameNode('section',null,'houserule');row.dataset.game=rule.game;row.append(gameNode('h4',rule.game==='cases'?'Cosmetic crates':rule.game[0].toUpperCase()+rule.game.slice(1)));const label=gameNode('label',null,'check'),enabled=gameNode('input');enabled.type='checkbox';enabled.dataset.field='enabled';enabled.checked=rule.enabled;label.append(enabled,document.createTextNode('Accept new wagers'));row.append(label);for(const [field,name,min,max]of [['min_stake','Minimum stake',1,1000000],['max_stake','Maximum stake',1,1000000],['payout_percent','Payout factor (%)',25,150]]){const fieldNode=gameField('admin-rule-'+rule.game+'-'+field,name,rule[field],min,max);fieldNode.querySelector('input').dataset.field=field;if(field==='payout_percent'&&!INSTANT_GAMES.includes(rule.game))fieldNode.hidden=true;row.append(fieldNode);}return row;}));
 $('admin-house-crates').replaceChildren(...rules.crates.map(crate=>{const row=gameNode('section',null,'houserule');row.append(gameNode('h4',CRATE_NAMES[crate.case_id]||'Cosmetic crate'));const cost=gameField('admin-price-'+crate.case_id,'Price (Kash)',crate.cost,1,1000000);cost.querySelector('input').dataset.field='cost';row.append(cost);row.dataset.caseId=crate.case_id;
  row.append(gameNode('small','Rarity weight factors: 100 keeps the original weight; 0 excludes a tier. Players see the resulting exact odds.'));
  for(const rarity of ['common','uncommon','rare','epic','legendary']){const field=gameField('admin-rarity-'+crate.case_id+'-'+rarity,rarity,crate.rarity_factors?.[rarity]??100,0,1000);field.querySelector('input').dataset.rarity=rarity;row.append(field);}return row;}));
}
function renderHouseMetrics(metrics){if(!metrics)return;$('admin-house-metrics').replaceChildren(...[['Arcade plays / 24h',metrics.arcade_games_24h],['Arcade staked / 24h',kashText(metrics.arcade_staked_24h)],['Arcade returned / 24h',kashText(metrics.arcade_paid_24h)],['Active Blackjack hands',metrics.active_blackjack_hands],['Pending Crash bets',metrics.pending_crash_bets]].map(([label,value])=>{const item=gameNode('div');item.append(gameNode('small',label),gameNode('strong',String(value)));return item;}));}
function setupGamblingAdmin(){
 if(gamblingAdminReady||currentUser.role!=='owner'||!$('admin-gambling'))return;gamblingAdminReady=true;const root=$('admin-gambling');
 setupHouseAdmin(root);
 root.append(gameNode('h3','Crash control desk'),gameNode('p','See the scheduled crash, switch the generation mode, pause new bets or queue a run of future multipliers. Changes affect future rounds.'),gameButton('Refresh live outcome',refreshGamblingAdminPreview));
 const live=gameNode('div',null,'admincrashpreview');live.id='admin-crash-preview';root.append(live);
 const form=gameNode('form',null,'adminsettingcard');const mode=gameNode('select');mode.id='admin-crash-mode';mode.setAttribute('aria-label','Crash generation mode');mode.append(new Option('Random generation','random'),new Option('Owner-controlled queue','controlled'));mode.addEventListener('change',()=>{if(mode.value==='random')$('admin-crash-queue-editor').replaceChildren();});const modeLabel=gameNode('label','Crash generation');modeLabel.append(mode);form.append(modeLabel);
 const pause=gameNode('label',null,'adminswitch');const paused=gameNode('input');paused.type='checkbox';paused.id='admin-crash-paused';pause.append(paused,gameNode('span','Pause new wagering'));form.append(pause,gameNode('p','Current stakes and cashouts continue to settle. Pausing stops accepting new wagers.'));
 const heading=gameNode('div',null,'row');heading.append(gameNode('h3','Next rounds'),gameButton('+ Add sequence',()=>addCrashQueueRow()));form.append(heading);const queue=gameNode('div');queue.id='admin-crash-queue-editor';form.append(queue);
 const presets=gameNode('div',null,'row crashpresets');for(const [label,min,max,count]of [['Low run',1,1.5,5],['High run',10,50,5],['Five at 50×',50,50,5],['Five at 1,000×',1000,1000,5]])presets.append(gameButton(label,()=>addCrashQueueRow({min_multiplier:min,max_multiplier:max,rounds:count})));form.append(presets,gameNode('p','Queue up to 20 rounds at 1.00–1,000.00×. Equal minimum and maximum pins an exact value. A range picks a random value inside that range.'));
 const save=gameNode('button','Save Crash settings & queue');save.type='submit';save.className='primary';const status=gameNode('p');status.id='admin-crash-status';status.setAttribute('role','status');form.append(save,status);form.addEventListener('submit',e=>{e.preventDefault();action(async()=>{const rows=[...queue.children].map(row=>({min_multiplier:Number(row.querySelector('[data-field="min"]').value),max_multiplier:Number(row.querySelector('[data-field="max"]').value),rounds:Number(row.querySelector('[data-field="rounds"]').value)}));if(rows.reduce((n,r)=>n+r.rounds,0)>20)throw new Error('Queue at most 20 rounds.');if(rows.some(r=>r.min_multiplier>r.max_multiplier))throw new Error('Every minimum must be at or below its maximum.');save.disabled=true;status.textContent='Saving…';try{await json('admin/gambling',{mode:mode.value,paused:paused.checked,queue:rows});await loadGamblingAdmin();status.textContent='Future Crash rounds updated. Changes are logged.';}finally{save.disabled=false;}});});root.append(form,gameNode('h3','Scheduled outcomes'),gameNode('div',null,'crashhistory'));root.lastChild.id='admin-crash-scheduled';root.append(gameNode('h3','Recent rounds'),gameNode('div',null,'crashhistory'));root.lastChild.id='admin-crash-history';
}
function addCrashQueueRow(value={min_multiplier:2,max_multiplier:10,rounds:1}){
 const root=$('admin-crash-queue-editor');if(root.children.length>=20){message('Keep the queue to 20 sequences or fewer.');return;}$('admin-crash-mode').value='controlled';window.refreshFilterMenus?.();const row=gameNode('div',null,'crashqueuerow');for(const [field,label,v,min,max,step]of [['min','Minimum ×',value.min_multiplier,1,1000,'0.01'],['max','Maximum ×',value.max_multiplier,1,1000,'0.01'],['rounds','Rounds',value.rounds,1,20,'1']]){const labelNode=gameNode('label',label);const input=gameNode('input');input.type='number';input.min=min;input.max=max;input.step=step;input.value=v;input.required=true;input.dataset.field=field;labelNode.append(input);row.append(labelNode);}row.append(gameButton('Remove',()=>row.remove()));root.append(row);
}
async function loadGamblingAdmin(){
 setupGamblingAdmin();const data=await(await api('admin/gambling')).json();renderGamblingAdminPreview(data);
 renderHouseEditor(data.rules);
 $('admin-crash-mode').value=data.mode;$('admin-crash-paused').checked=data.paused;$('admin-crash-queue-editor').replaceChildren();for(const item of data.queue||[])addCrashQueueRow({min_multiplier:item.crash_multiplier,max_multiplier:item.crash_multiplier,rounds:1});window.refreshFilterMenus?.();
}
function renderGamblingAdminPreview(data){
 renderHouseMetrics(data.metrics);
 gamblingAdminData=data;const paused=data.paused||data.rules?.paused||data.rules?.games?.find(r=>r.game==='crash')?.enabled===false;const crash=data.crash;const preview=$('admin-crash-preview');preview.replaceChildren(gameNode('small',`${crash.id===null?'NO ACTIVE ROUND':'ROUND '+crash.id} · ${crash.phase.toUpperCase()}`),gameNode('strong',data.planned_crash_multiplier===null?'—':multiplierText(data.planned_crash_multiplier)),gameNode('p',`Scheduled crash ${data.planned_crash_at_ms?new Date(data.planned_crash_at_ms).toLocaleTimeString():'not scheduled'}${crash.id===null?'':` · Current round: ${crash.mode==='controlled'?'owner-controlled':'random'}`} · Future rounds: ${data.mode==='controlled'?'owner-controlled':'random'}${paused?' · New wagering paused':''}`));
 $('admin-crash-scheduled').replaceChildren(...(data.queue||[]).map((r,i)=>gameNode('span',`${i+1}: ${multiplierText(r.crash_multiplier)}`,'crashchip high')));if(!data.queue?.length)$('admin-crash-scheduled').append(gameNode('p','No queued outcomes. Future rounds use the current generation mode.'));
 $('admin-crash-history').replaceChildren(...(crash.history||[]).map(r=>gameNode('span',multiplierText(r.crash_multiplier)+' · '+r.mode,'crashchip')));
}
async function refreshGamblingAdminPreview(){if(gamblingAdminLoading)return;gamblingAdminLoading=true;try{renderGamblingAdminPreview(await(await api('admin/gambling')).json());}finally{gamblingAdminLoading=false;}}
setInterval(()=>{if(!gamblingReady||document.hidden||$('gamblingview')?.hidden||gamblingBusy||gamblingTab==='crash')return;loadGambling().catch(error=>{if(error.name!=='AbortError')setGamblingStatus(error.message,true);});},1500);
setInterval(()=>{if(!gamblingReady||!crashIsVisible()||gamblingBusy)return;loadGambling(true).catch(error=>{if(error.name!=='AbortError')setGamblingStatus(error.message,true);});},500);
setInterval(()=>{if(!gamblingAdminReady||document.hidden||$('moderation')?.hidden||$('admin-gambling')?.hidden)return;refreshGamblingAdminPreview().catch(error=>{const n=$('admin-crash-status');if(n)n.textContent=error.message;});},2000);
