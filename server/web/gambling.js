'use strict';
let gamblingReady=false,gamblingTab='crash',gamblingData=null,gamblingBusy=false,gamblingLoading=false;
let gamblingLoadPromise=null;
let gamblingPendingMutation=null;
let gamblingCaseRenderKey='',gamblingCosmeticRenderKey='';
let crashPeople={round:null,pages:1,rows:[],total:0,hasMore:false},crashPeopleLoading=false;
let crashPeopleRefreshPromise=null,crashPeopleRefreshPending=null;
let gamblingAdminReady=false,gamblingAdminData=null,gamblingAdminLoading=false;
const CRASH_VISUAL_FRESH_MS=2000,CRASH_GROWTH_MS=10000;
let crashVisual=null,crashAnimation=null,crashMotionQuery=null,crashHooksReady=false;
function crashClock(){return typeof performance!=='undefined'?performance.now():Date.now();}
function crashReducedMotion(){return !!crashMotionQuery?.matches;}
function crashIsVisible(){return !document.hidden&&!$('gamblingview')?.hidden&&!$('gambling-crash')?.hidden;}
function stopCrashAnimation(requireFresh=false){if(crashAnimation!==null&&typeof cancelAnimationFrame==='function')cancelAnimationFrame(crashAnimation);crashAnimation=null;if(requireFresh&&crashVisual)crashVisual.needsFresh=true;}
function crashSetText(id,text){const n=$(id);if(n&&n.textContent!==text)n.textContent=text;}
function crashSvg(tag,attributes){const n=typeof document.createElementNS==='function'?document.createElementNS('http://www.w3.org/2000/svg',tag):gameNode(tag);for(const [key,value]of Object.entries(attributes))n.setAttribute(key,value);return n;}
function drawCrashVisual(at=crashClock()){
 if(!crashVisual||!$('crash-multiplier'))return;
 const sample=crashVisual,age=Math.max(0,at-sample.receivedAt),stale=sample.needsFresh||age>=CRASH_VISUAL_FRESH_MS;
 // The flight is a short display estimate only. Bet/cashout state and amounts
 // always use the untouched server response in renderCrash/gamblingMutation.
 const elapsed=Math.max(0,sample.serverTime-sample.startsAt);
 const serverGrowth=Math.exp(Math.min(elapsed,CRASH_GROWTH_MS*Math.log(100))/CRASH_GROWTH_MS);
 const base=Math.min(sample.multiplier,serverGrowth);
 const estimate=sample.phase==='running'&&!crashReducedMotion()&&!sample.needsFresh?Math.min(100,base*Math.exp(Math.min(age,CRASH_VISUAL_FRESH_MS)/CRASH_GROWTH_MS)):sample.multiplier;
 const value=sample.phase==='crashed'?sample.final:sample.phase==='running'?estimate:1;
 const progress=Math.max(0,Math.min(1,Math.log(value)/Math.log(100)));
 crashSetText('crash-multiplier',multiplierText(value));
 const stage=$('crash-multiplier').parentElement;stage.dataset.stale=String(stale&&sample.phase!=='crashed'&&sample.phase!=='paused');
 const seconds=Math.max(0,Math.ceil((sample.startsAt-sample.serverTime-Math.min(age,CRASH_VISUAL_FRESH_MS))/1000));
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
 crashAnimation=requestAnimationFrame(()=>{crashAnimation=null;if(!crashIsVisible()){stopCrashAnimation(true);return;}if(drawCrashVisual())scheduleCrashAnimation();});
}
function refreshCrashVisibility(){if(!crashIsVisible()){stopCrashAnimation(true);return;}drawCrashVisual();scheduleCrashAnimation();}
function requestCrashRefresh(){if(crashVisual&&crashIsVisible()&&!gamblingBusy)loadGambling().catch(error=>setGamblingStatus(error.message,true));}
function setupCrashAnimation(){
 if(crashHooksReady)return;crashHooksReady=true;
 if(typeof window.matchMedia==='function'){crashMotionQuery=window.matchMedia('(prefers-reduced-motion: reduce)');const change=()=>{stopCrashAnimation();drawCrashVisual();scheduleCrashAnimation();};if(crashMotionQuery.addEventListener)crashMotionQuery.addEventListener('change',change);else crashMotionQuery.addListener?.(change);}
 document.addEventListener?.('visibilitychange',()=>{stopCrashAnimation(true);if(!document.hidden){drawCrashVisual();requestCrashRefresh();}});
 window.addEventListener?.('pagehide',()=>stopCrashAnimation(true));window.addEventListener?.('pageshow',()=>{if(crashVisual){drawCrashVisual();requestCrashRefresh();}});
}
function updateCrashVisual(crash,serverTime){
 const time=Number(serverTime),starts=Number(crash.betting_ends_ms),multiplier=Number(crash.multiplier),final=Number(crash.crash_multiplier);
 if(!crashVisual||crashVisual.id!==crash.id||crashVisual.phase!==crash.phase||Number.isFinite(time)&&time>crashVisual.serverTime){
  stopCrashAnimation();crashVisual={id:crash.id,phase:crash.phase,serverTime:Number.isFinite(time)?time:0,startsAt:Number.isFinite(starts)?starts:Number.isFinite(time)?time:0,multiplier:Number.isFinite(multiplier)?Math.max(1,Math.min(100,multiplier)):1,final:Number.isFinite(final)?Math.max(1,Math.min(100,final)):1,receivedAt:crashClock(),needsFresh:!Number.isFinite(time)};
 }
 drawCrashVisual();scheduleCrashAnimation();
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
 const hero=gameNode('header',null,'gamehero');const intro=gameNode('div');intro.append(gameNode('p','CANNA / AFTER HOURS','eyebrow'),gameNode('h2','A little luck. A lot of Kash.'),gameNode('p','Play Crash or Blackjack, then turn your Kash into profile frames and banners. Kash is community play currency.'));
 const wallet=gameNode('div',null,'gamewallet');wallet.append(gameNode('small','YOUR WALLET'),gameNode('strong','—'));wallet.lastChild.id='gambling-balance';const daily=gameButton('Claim daily Kash',()=>gamblingMutation('gambling/daily',{}),'primary');daily.id='gambling-daily';wallet.append(daily);hero.append(intro,wallet);
 const notice=gameNode('p',null,'game-notice');notice.id='gambling-disclosure';const nav=gameNode('nav',null,'gametabs');nav.setAttribute('aria-label','Kash games');
 const panes=gameNode('div',null,'gamepanes');for(const [id,name]of [['crash','↗ Crash'],['blackjack','♠ Blackjack'],['cases','◇ Cosmetic crates'],['collection','▣ My collection']]){const tab=gameButton(name,()=>selectGamblingTab(id));tab.dataset.game=id;nav.append(tab);const pane=gameNode('section');pane.id='gambling-'+id;pane.hidden=id!==gamblingTab;panes.append(pane);}
 const status=gameNode('p');status.id='gambling-status';status.setAttribute('role','status');const retry=gameButton('Retry pending action',()=>{const pending=gamblingPendingMutation;return pending&&gamblingMutation(pending.path,pending.data,pending.success);});retry.id='gambling-retry';retry.hidden=true;root.append(hero,notice,nav,status,retry,panes);setupCrash();setupBlackjack();setupCases();setupCosmeticCollection();selectGamblingTab(gamblingTab);
}
function selectGamblingTab(id){gamblingTab=id;document.querySelectorAll('.gamepanes>section').forEach(p=>p.hidden=p.id!=='gambling-'+id);document.querySelectorAll('.gametabs button').forEach(b=>{const active=b.dataset.game===id;b.classList.toggle('active',active);b.setAttribute('aria-current',active?'page':'false');});refreshCrashVisibility();if(id==='crash'&&crashVisual&&(crashVisual.needsFresh||crashClock()-crashVisual.receivedAt>=CRASH_VISUAL_FRESH_MS))requestCrashRefresh();}
async function loadGambling(){setupGambling();if(!gamblingReady)return;if(gamblingLoading)return gamblingLoadPromise;gamblingLoading=true;gamblingLoadPromise=(async()=>{try{const data=await(await api('gambling')).json();gamblingData=data;renderGambling(data);queueCrashParticipantRefresh(data.crash);}finally{gamblingLoading=false;}})();return gamblingLoadPromise;}
function renderGambling(data){
 $('gambling-balance').textContent=kashText(data.wallet.balance);$('kashbalance').textContent=kashText(data.wallet.balance);if(currentUser)currentUser.kash=data.wallet.balance;
 $('gambling-daily').disabled=gamblingBusy||!!gamblingPendingMutation||!data.wallet.daily_available;$('gambling-daily').textContent=data.wallet.daily_available?'Claim daily Kash':'Daily already claimed';
 $('gambling-retry').hidden=!gamblingPendingMutation||gamblingBusy;$('gambling-retry').disabled=gamblingBusy;
 const disclosure=data.notice||'Kash cannot be bought or withdrawn. The owner can inspect upcoming Crash results and configure future rounds.';$('gambling-disclosure').textContent=disclosure;
 renderCrash(data.crash,data.server_time_ms);renderBlackjack(data.blackjack);
 const casesKey=JSON.stringify([data.cases,data.cosmetics.catalog,data.wallet.balance,gamblingBusy,!!gamblingPendingMutation]);
 if(casesKey!==gamblingCaseRenderKey){renderCases(data);gamblingCaseRenderKey=casesKey;}
 const cosmeticKey=JSON.stringify([data.cosmetics,gamblingBusy,!!gamblingPendingMutation]);
 if(cosmeticKey!==gamblingCosmeticRenderKey){renderCosmeticCollection(data.cosmetics);gamblingCosmeticRenderKey=cosmeticKey;}
}
async function gamblingMutation(path,data,success){
 if(gamblingBusy)return;
 const key=JSON.stringify({path,data});
 if(gamblingPendingMutation&&gamblingPendingMutation.key!==key)throw new Error('Retry your pending action before starting another. Its original request is preserved to prevent a second charge.');
 const pending=gamblingPendingMutation||{path,data,success,key,payload:{...data,request_id:gameRequestId()}};
 gamblingBusy=true;setGamblingStatus('Working…');if(gamblingData)renderGambling(gamblingData);let recorded=false;
 try{if(gamblingLoading)await gamblingLoadPromise;const result=await json(path,pending.payload);recorded=true;gamblingPendingMutation=null;await loadGambling();setGamblingStatus(typeof success==='function'?success(result):success||'Done.');return result;}
 catch(error){
  if(!recorded&&(!Number.isInteger(error.status)||error.status>=500)){gamblingPendingMutation=pending;setGamblingStatus('The connection ended before the result was confirmed. Retry the pending action; Canna will reuse the same request and prevent a second charge.',true);}
  else{gamblingPendingMutation=null;setGamblingStatus(recorded?'Your action was recorded. The latest balance could not load; refresh this page.':error.message,true);}
  throw error;
 }finally{gamblingBusy=false;if(gamblingData)renderGambling(gamblingData);}
}
function setupCrash(){
 const root=$('gambling-crash');const layout=gameNode('div',null,'crashlayout');const stage=gameNode('div',null,'crashstage');stage.setAttribute('aria-label','Current Crash round');
 const phase=gameNode('span','Loading round…','crashphase');phase.id='crash-phase';const multiplier=gameNode('strong','1.00×','crashmultiplier');multiplier.id='crash-multiplier';const detail=gameNode('p');detail.id='crash-detail';
 const chart=crashSvg('svg',{viewBox:'0 0 600 180',preserveAspectRatio:'none',class:'crashchart','aria-hidden':'true'});const area=crashSvg('path',{class:'crashchart-area'});area.id='crash-flight-area';const line=crashSvg('path',{class:'crashchart-line',fill:'none','vector-effect':'non-scaling-stroke'});line.id='crash-flight-line';const dot=crashSvg('circle',{class:'crashchart-dot',r:5});dot.id='crash-flight-dot';chart.append(area,line,dot);
 const flight=gameNode('div',null,'crashflight');flight.setAttribute('aria-hidden','true');const track=gameNode('div',null,'crashflight-track');const fill=gameNode('span',null,'crashflight-fill');fill.id='crash-flight-fill';track.append(fill);const scale=gameNode('div',null,'crashflight-scale');scale.append(gameNode('span','1×'),gameNode('span','100×'));flight.append(track,scale);const sync=gameNode('p','Waiting for server…','crashsync');sync.id='crash-sync';stage.append(chart,phase,multiplier,detail,flight,sync);setupCrashAnimation();
 const control=gameNode('section',null,'gamepanel');control.append(gameNode('h3','Your next flight'),gameNode('p','Join during the countdown. Cash out before the crash, or choose an automatic target. At the crash the round is over.'));
 const form=gameNode('form');form.id='crash-bet-form';form.append(gameField('crash-stake','Stake (Kash)',25,1,1000000),gameField('crash-auto','Auto cashout (×)',2,1.01,100,'0.01'));const auto=gameNode('label',null,'check');const autoToggle=gameNode('input');autoToggle.type='checkbox';autoToggle.checked=true;autoToggle.id='crash-auto-enabled';auto.append(autoToggle,document.createTextNode('Use auto cashout'));form.append(auto);
 const bet=gameNode('button','Place bet');bet.type='submit';bet.className='primary';bet.id='crash-place-bet';form.append(bet);form.addEventListener('submit',e=>{e.preventDefault();action(async()=>{if(!gamblingData)return;await gamblingMutation('gambling/crash/bet',{round_id:gamblingData.crash.id,stake:Number($('crash-stake').value),auto_cashout:$('crash-auto-enabled').checked?Number($('crash-auto').value):null},'Bet placed.');});});
 const cashout=gameButton('Cash out',()=>gamblingMutation('gambling/crash/cashout',{round_id:gamblingData.crash.id},r=>'Cashout recorded. '+kashText(r.bet?.payout||r.payout||0)),'gamecashout');cashout.id='crash-cashout';const betStatus=gameNode('p');betStatus.id='crash-your-bet';betStatus.setAttribute('role','status');control.append(form,cashout,betStatus);layout.append(stage,control);
 const people=gameNode('section',null,'gamepanel crashpeople');const heading=gameNode('div',null,'crashpeople-heading');const count=gameNode('p');count.id='crash-people-count';heading.append(gameNode('h3','Players this round'),count);const scroll=gameNode('div',null,'crashpeople-scroll');const table=gameNode('table');table.setAttribute('aria-label','Crash bets and cashouts');const head=gameNode('thead');const headings=gameNode('tr');for(const label of ['Player','Bet','Result'])headings.append(gameNode('th',label));head.append(headings);const body=gameNode('tbody');body.id='crash-people-rows';table.append(head,body);scroll.append(table);const empty=gameNode('p','No bets yet.');empty.id='crash-people-empty';const more=gameButton('Show more players',async()=>{if(crashPeopleLoading)return;crashPeople.pages++;await loadGambling();});more.id='crash-people-more';more.hidden=true;const status=gameNode('p');status.id='crash-people-status';status.setAttribute('role','status');people.append(heading,scroll,empty,more,status,gameNode('p','Cashouts stay here until the next round.','game-footnote'));
 const history=gameNode('div',null,'crashhistory');history.id='crash-history';root.append(layout,people,gameNode('h3','Recent rounds'),history,gameNode('p','Round state and payouts are decided by the server. Random mode still allows the owner to inspect the upcoming result. Owner-controlled rounds are labelled here.','game-footnote'));
}
function renderCrash(crash,now){
 const phase=crash.phase;const stage=$('crash-multiplier').parentElement;stage.dataset.phase=phase;const seconds=Math.max(0,Math.ceil((crash.betting_ends_ms-now)/1000));
 $('crash-phase').textContent=phase==='betting'?`Taking bets · ${seconds}s`:phase==='running'?'In flight':phase==='paused'?'Wagering paused':'Crashed';
 updateCrashVisual(crash,now);
 $('crash-detail').textContent=`${crash.id===null?'Waiting for next round':'Round '+String(crash.id).slice(0,12)} · ${crash.mode==='controlled'?'Owner-controlled':'Random'}${crash.owner_visible?' · Owner can inspect result':''}${crash.paused?' · New wagering paused':''}`;
 const hasBet=!!crash.bet;$('crash-place-bet').disabled=gamblingBusy||!!gamblingPendingMutation||crash.paused||phase!=='betting'||hasBet;$('crash-cashout').disabled=gamblingBusy||!!gamblingPendingMutation||phase!=='running'||!hasBet||crash.bet.status!=='pending';
 $('crash-cashout').textContent=hasBet&&crash.bet.status==='pending'&&phase==='running'?`Cash out · ${kashText(Math.floor(crash.bet.stake*crash.multiplier))}`:'Cash out';
 $('crash-your-bet').textContent=!hasBet?'No bet in this round.':crash.bet.status==='pending'?`${kashText(crash.bet.stake)} in play${crash.bet.auto_cashout?' · Auto '+multiplierText(crash.bet.auto_cashout):''}`:crash.bet.status==='won'?`Cashed out · ${kashText(crash.bet.payout)}`:`Crashed · ${kashText(crash.bet.stake)} lost`;
 $('crash-history').replaceChildren(...(crash.history||[]).slice(0,15).map(r=>{const n=gameNode('span',multiplierText(r.crash_multiplier),'crashchip '+(r.crash_multiplier>=10?'high':r.crash_multiplier<2?'low':'mid'));n.title=`Round ${r.id} · ${r.mode==='controlled'?'Owner-controlled':'Random'}`;return n;}));
 acceptCrashParticipants(crash);renderCrashParticipants(crash.phase);
}
function acceptCrashParticipants(crash){
 if(crashPeople.round!==crash.id){crashPeople={round:crash.id,pages:1,rows:[],total:0,hasMore:false};if($('crash-people-status'))$('crash-people-status').textContent='';}
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
 const round=crash.id,pages=crashPeople.pages,rows=[];let cursor=crash.participant_next_after_user_id,more=!!crash.participant_has_more;
 crashPeopleLoading=true;renderCrashParticipants(crash.phase);
 try{for(let page=1;page<pages&&more;page++){
  const result=await(await api(`gambling?crash_round_id=${encodeURIComponent(round)}&crash_after_user_id=${encodeURIComponent(cursor)}`)).json();
  if(crashPeople.round!==round||result.crash.id!==round)return;
  rows.push(...(result.crash.participants||[]));more=!!result.crash.participant_has_more;cursor=result.crash.participant_next_after_user_id;
 }
 if(crashPeople.round===round){
  // Primary state and cashout mutations never wait for this secondary list.
  // Keep a newer first page and never regress an already settled bet to pending.
  const merged=new Map(crashPeople.rows.map(row=>[row.user_id,row]));
  for(const row of rows){const old=merged.get(row.user_id);if(old&&old.status!=='pending'&&row.status==='pending')continue;merged.set(row.user_id,row);}
  crashPeople.rows=[...merged.values()].sort((a,b)=>a.user_id-b.user_id);crashPeople.hasMore=crashPeople.rows.length<crashPeople.total;$('crash-people-status').textContent='';
 }
 }catch(error){if(crashPeople.round===round)$('crash-people-status').textContent=error.status===409?'The next round started. Refreshing players…':'Player updates could not load. They will retry with the next update.';}
 finally{crashPeopleLoading=false;renderCrashParticipants(gamblingData?.crash.phase||crash.phase);if(crashPeopleRefreshPending){const latest=crashPeopleRefreshPending;crashPeopleRefreshPending=null;queueCrashParticipantRefresh(latest);}}
}
function renderCrashParticipants(phase){
 if(!$('crash-people-rows'))return;
 $('crash-people-count').textContent=`${crashPeople.total} ${crashPeople.total===1?'player':'players'}${crashPeople.rows.length<crashPeople.total?' · '+crashPeople.rows.length+' shown':''}`;
 $('crash-people-empty').hidden=!!crashPeople.rows.length;
 $('crash-people-more').hidden=!crashPeople.hasMore;$('crash-people-more').disabled=crashPeopleLoading;
 $('crash-people-rows').replaceChildren(...crashPeople.rows.map(person=>{
  const row=gameNode('tr');row.dataset.userId=person.user_id;const player=gameNode('td'),stake=gameNode('td',kashText(person.stake),'crashpeople-stake'),result=gameNode('td',null,'crashpeople-result');
  const name=person.display_name||person.username||'Unavailable member';
  if(Number.isSafeInteger(person.user_id)&&person.user_id>0&&person.profile_url===`/members/${person.user_id}`){const link=gameNode('a',name);link.href=person.profile_url;link.dataset.page=person.profile_url;player.append(link);}else player.textContent=name;
  result.dataset.status=person.status;
  if(person.status==='won'){result.append(gameNode('strong','Cashed out · '+multiplierText(person.cashout_multiplier)));if(Number.isFinite(person.cashout_elapsed_ms)&&person.cashout_elapsed_ms>=0)result.append(gameNode('small',(person.cashout_elapsed_ms/1000).toFixed(2)+'s into the round'));if(Number.isFinite(person.cashout_at_ms)){result.title='Cashed out '+new Date(person.cashout_at_ms).toLocaleString();}}
  else result.textContent=person.status==='lost'?'Crashed':phase==='betting'?'Waiting':'In play';
  row.append(player,stake,result);return row;
 }));
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
function setupCases(){const root=$('gambling-cases');root.append(gameNode('h3','Choose your crate'),gameNode('p','BO2 calling cards, classic MW2 calling cards and avatar frames have separate crates. Check every item and its odds before spending Kash. Duplicate drops add to your collection.'),gameNode('div',null,'casegrid'));root.lastChild.id='case-list';const result=gameNode('section',null,'case-result');result.id='case-result';result.hidden=true;result.setAttribute('role','status');root.append(result);}
function cosmeticPreview(item,large=false){
 const preview=gameNode('div',null,'cosmeticpreview '+(large?'large ':'')+(item.kind==='banner'?'bannerpreview':'framepreview'));const style=String(item.style||item.id||'');if(/^[a-z0-9_-]{1,80}$/.test(style))preview.classList.add('cosmetic-'+style);
 if(item.kind==='frame')preview.append(gameNode('span',currentUser?.username?.slice(0,1).toUpperCase()||'C','cosmeticinitial'));else preview.append(gameNode('span','CANNA','cosmeticbannertext'));
 if(item.kind==='banner'&&['mw2','bo2'].includes(cosmeticCollection(item))){preview.classList.add('calling-card');preview.title=item.name;}
 const asset=item.animated&&typeof window.matchMedia==='function'&&window.matchMedia('(prefers-reduced-motion: reduce)').matches&&item.poster_asset?item.poster_asset:item.asset;
 if(typeof asset==='string'&&/^\/api\/v1\/cosmetics\/assets\/[a-zA-Z0-9_-]+$/.test(asset)){const img=gameNode('img');const hash=asset===item.poster_asset?item.poster_sha256:item.sha256;img.src=asset+(/^[a-f0-9]{64}$/.test(hash||'')?'?v='+hash:'');img.alt='';img.loading='lazy';img.decoding='async';img.className='cosmeticasset';preview.append(img);}
 preview.dataset.rarity=item.rarity||'common';preview.setAttribute('aria-label',item.name+' '+item.kind+' preview');return preview;
}
function renderCases(data){
 const catalog=new Map(data.cosmetics.catalog.map(item=>[item.id,item]));
 $('case-list').replaceChildren(...data.cases.map(c=>{
  const card=gameNode('section',null,'gamepanel casecard');card.dataset.crate=c.id;
  const collection=c.collection||'',glyph=collection==='bo2'?'II':collection==='mw2'?'MW2':'◇';
  card.append(gameNode('span',glyph,'caseglyph'),gameNode('h3',c.name),gameNode('p',`${c.items.length} ${collection==='frames'?'avatar frames':'calling cards'} · ${kashText(c.cost)}`));
  const samples=gameNode('div',null,'cratepreviews');const sampleIds=[...(c.items||[])].sort((a,b)=>Number(!!catalog.get(b.id)?.animated)-Number(!!catalog.get(a.id)?.animated)).slice(0,3);
  for(const drop of sampleIds){const item=catalog.get(drop.id);if(item)samples.append(cosmeticPreview(item));}card.append(samples);
  const open=gameButton('Open crate · '+kashText(c.cost),async()=>{
   await gamblingMutation('gambling/cases/open',{case_id:c.id},r=>{
    const item=typeof r.item==='string'?catalog.get(r.item):r.item,result=$('case-result');result.hidden=false;result.replaceChildren(gameNode('p','YOUR DROP','eyebrow'));
    if(item)result.append(cosmeticPreview(item,true),gameNode('h3',item.name),gameNode('p',`${item.rarity} ${item.kind}${item.animated?' · Animated':''} · Added to your collection`),gameButton('Equip this '+item.kind,()=>equipCosmetic(item.kind,item.id),'primary'));
    else result.append(gameNode('h3','Cosmetic added to your collection'));
    result.append(gameButton('View my collection',()=>selectGamblingTab('collection')));return 'Crate opened. Your drop is ready to equip.';
   });
  },'primary');open.disabled=gamblingBusy||!!gamblingPendingMutation||c.available===false||data.wallet.balance<c.cost;if(c.available===false)open.textContent='Crate unavailable';card.append(open);
  const odds=gameNode('details');odds.append(gameNode('summary','See every item & drop odds'),gameNode('p','Shown chances are rounded to two decimals.'));const list=gameNode('div',null,'caseodds');odds.append(list);let rendered=false;
  odds.addEventListener('toggle',()=>{if(!odds.open||rendered)return;rendered=true;for(const drop of c.items){const item=catalog.get(drop.id);if(!item)continue;const row=gameNode('div');row.append(cosmeticPreview(item),gameNode('span',item.name+(item.animated?' · Animated':'')),gameNode('strong',Number(drop.odds_percent).toFixed(2)+'%'));list.append(row);}});
  card.append(odds);return card;
 }));
}
function cosmeticCollection(item){return item.collection||(item.kind==='frame'?'frames':String(item.id).startsWith('mw2-')?'mw2':String(item.id).startsWith('bo2-')?'bo2':'canna');}
function setupCosmeticCollection(){
 const root=$('gambling-collection'),tools=gameNode('div',null,'row collectiontools'),kind=gameNode('select');kind.id='cosmetic-kind';kind.setAttribute('aria-label','Filter cosmetic type');kind.append(new Option('Frames & banners',''),new Option('Avatar frames','frame'),new Option('Profile banners','banner'));
 const collection=gameNode('select');collection.id='cosmetic-source';collection.setAttribute('aria-label','Filter cosmetic collection');collection.append(new Option('All collections',''),new Option('BO2 calling cards','bo2'),new Option('MW2 calling cards','mw2'),new Option('Avatar frames','frames'),new Option('Canna originals','canna'));
 const owned=gameNode('label',null,'check'),check=gameNode('input');check.type='checkbox';check.id='cosmetic-owned-only';check.checked=true;owned.append(check,document.createTextNode('Only my collection'));
 for(const input of [kind,collection,check])input.addEventListener('change',()=>gamblingData&&renderCosmeticCollection(gamblingData.cosmetics));
 const search=gameNode('input');search.type='search';search.id='cosmetic-search';search.placeholder='Find a frame or banner…';search.maxLength=80;search.setAttribute('aria-label','Search cosmetics');search.addEventListener('input',()=>gamblingData&&renderCosmeticCollection(gamblingData.cosmetics));
 tools.append(search,kind,collection,owned,gameButton('Remove frame',()=>equipCosmetic('frame',null)),gameButton('Remove banner',()=>equipCosmetic('banner',null)),gameButton('View my profile',()=>openProfile(currentUser.id)));
 const summary=gameNode('p');summary.id='cosmetic-collection-summary';summary.setAttribute('role','status');const grid=gameNode('div',null,'cosmeticgrid');grid.id='cosmetic-collection';
 root.append(gameNode('h3','Make your profile yours'),gameNode('p','Equip your unlocked cosmetics here. Uncheck Only my collection to preview locked items. Calling cards keep their complete artwork; animated cards play in previews and profiles. Reduced motion shows a still frame.'),summary,tools,grid);
}
function renderCosmeticCollection(cosmetics){
 const inventory=new Map(cosmetics.owned.map(i=>[i.id,i.count])),kind=$('cosmetic-kind').value,collection=$('cosmetic-source').value,only=$('cosmetic-owned-only').checked,query=$('cosmetic-search').value.trim().toLowerCase();
 const items=cosmetics.catalog.filter(i=>(!kind||kind===i.kind)&&(!collection||collection===cosmeticCollection(i))&&(!only||inventory.has(i.id))&&(!query||`${i.name} ${i.kind} ${i.rarity} ${cosmeticCollection(i)} ${i.animated?'animated':''}`.toLowerCase().includes(query)));
 $('cosmetic-collection-summary').textContent=`${inventory.size} unlocked cosmetic${inventory.size===1?'':'s'} · ${items.length} shown. ${gamblingPendingMutation?'Confirm the pending action before changing your cosmetics.':'Equip one avatar frame and one banner.'}`;
 $('cosmetic-collection').replaceChildren(...items.map(item=>{
  const card=gameNode('article',null,'cosmeticcard'),count=inventory.get(item.id)||0,equipped=cosmetics.equipped[item.kind]===item.id;card.dataset.cosmeticId=item.id;
  card.append(cosmeticPreview(item,true),gameNode('h3',item.name),gameNode('p',`${item.rarity} · ${item.kind}${item.animated?' · Animated':''}${count?' · Owned '+count:' · Locked — unlock from a crate'}`));
  const equip=gameButton(equipped?'Equipped':count?'Equip '+item.kind:'View cosmetic crate',()=>count?equipCosmetic(item.kind,item.id):selectGamblingTab('cases'),equipped?'equipped':'');equip.disabled=gamblingBusy||!!gamblingPendingMutation||equipped;card.append(equip);return card;
 }));
 if(!items.length){const empty=gameNode('section',null,'cosmeticempty');empty.append(gameNode('p',inventory.size?'No cosmetics match this filter.':'Your collection is empty. Opening a crate unlocks an item you can equip.'),gameButton('Browse cosmetic crates',()=>selectGamblingTab('cases')));$('cosmetic-collection').append(empty);}
}
async function equipCosmetic(kind,id){if(!gamblingData)return;if(!['frame','banner'].includes(kind))throw new Error('Choose an avatar frame or a banner.');if(id&&!gamblingData.cosmetics.owned.some(item=>item.id===id&&item.count>0))throw new Error('Unlock this cosmetic from a crate before equipping it.');const equipped={frame:gamblingData.cosmetics.equipped.frame||null,banner:gamblingData.cosmetics.equipped.banner||null,[kind]:id};await gamblingMutation('gambling/cosmetics/equip',equipped,id?`${kind==='frame'?'Avatar frame':'Banner'} equipped. View My profile to see it.`:`${kind==='frame'?'Avatar frame':'Banner'} removed.`);}
function setupGamblingAdmin(){
 if(gamblingAdminReady||currentUser.role!=='owner'||!$('admin-gambling'))return;gamblingAdminReady=true;const root=$('admin-gambling');
 root.append(gameNode('h3','Crash control desk'),gameNode('p','See the scheduled crash, switch the generation mode, pause new bets or queue a run of future multipliers. The public game labels owner-controlled rounds and explains owner visibility. Changes affect future rounds.'),gameButton('Refresh live outcome',refreshGamblingAdminPreview));
 const live=gameNode('div',null,'admincrashpreview');live.id='admin-crash-preview';root.append(live);
 const form=gameNode('form',null,'adminsettingcard');const mode=gameNode('select');mode.id='admin-crash-mode';mode.setAttribute('aria-label','Crash generation mode');mode.append(new Option('Random generation','random'),new Option('Owner-controlled queue','controlled'));mode.addEventListener('change',()=>{if(mode.value==='random')$('admin-crash-queue-editor').replaceChildren();});const modeLabel=gameNode('label','Crash generation');modeLabel.append(mode);form.append(modeLabel);
 const pause=gameNode('label',null,'adminswitch');const paused=gameNode('input');paused.type='checkbox';paused.id='admin-crash-paused';pause.append(paused,gameNode('span','Pause new wagering'));form.append(pause,gameNode('p','Current stakes and cashouts continue to settle. Pausing stops accepting new wagers.'));
 const heading=gameNode('div',null,'row');heading.append(gameNode('h3','Next rounds'),gameButton('+ Add sequence',()=>addCrashQueueRow()));form.append(heading);const queue=gameNode('div');queue.id='admin-crash-queue-editor';form.append(queue);
 const presets=gameNode('div',null,'row crashpresets');for(const [label,min,max,count]of [['Low run',1,1.5,5],['High run',10,50,5],['Five at 50×',50,50,5]])presets.append(gameButton(label,()=>addCrashQueueRow({min_multiplier:min,max_multiplier:max,rounds:count})));form.append(presets,gameNode('p','Queue up to 20 rounds at 1.00–100.00×. Equal minimum and maximum pins an exact value. A range picks a random value inside that range.'));
 const save=gameNode('button','Save Crash settings & queue');save.type='submit';save.className='primary';const status=gameNode('p');status.id='admin-crash-status';status.setAttribute('role','status');form.append(save,status);form.addEventListener('submit',e=>{e.preventDefault();action(async()=>{const rows=[...queue.children].map(row=>({min_multiplier:Number(row.querySelector('[data-field="min"]').value),max_multiplier:Number(row.querySelector('[data-field="max"]').value),rounds:Number(row.querySelector('[data-field="rounds"]').value)}));if(rows.reduce((n,r)=>n+r.rounds,0)>20)throw new Error('Queue at most 20 rounds.');if(rows.some(r=>r.min_multiplier>r.max_multiplier))throw new Error('Every minimum must be at or below its maximum.');save.disabled=true;status.textContent='Saving…';try{await json('admin/gambling',{mode:mode.value,paused:paused.checked,queue:rows});await loadGamblingAdmin();status.textContent='Future Crash rounds updated. Changes are logged.';}finally{save.disabled=false;}});});root.append(form,gameNode('h3','Scheduled outcomes'),gameNode('div',null,'crashhistory'));root.lastChild.id='admin-crash-scheduled';root.append(gameNode('h3','Recent rounds'),gameNode('div',null,'crashhistory'));root.lastChild.id='admin-crash-history';
}
function addCrashQueueRow(value={min_multiplier:2,max_multiplier:10,rounds:1}){
 const root=$('admin-crash-queue-editor');if(root.children.length>=20){message('Keep the queue to 20 sequences or fewer.');return;}$('admin-crash-mode').value='controlled';window.refreshFilterMenus?.();const row=gameNode('div',null,'crashqueuerow');for(const [field,label,v,min,max,step]of [['min','Minimum ×',value.min_multiplier,1,100,'0.01'],['max','Maximum ×',value.max_multiplier,1,100,'0.01'],['rounds','Rounds',value.rounds,1,20,'1']]){const labelNode=gameNode('label',label);const input=gameNode('input');input.type='number';input.min=min;input.max=max;input.step=step;input.value=v;input.required=true;input.dataset.field=field;labelNode.append(input);row.append(labelNode);}row.append(gameButton('Remove',()=>row.remove()));root.append(row);
}
async function loadGamblingAdmin(){
 setupGamblingAdmin();const data=await(await api('admin/gambling')).json();renderGamblingAdminPreview(data);
 $('admin-crash-mode').value=data.mode;$('admin-crash-paused').checked=data.paused;$('admin-crash-queue-editor').replaceChildren();for(const item of data.queue||[])addCrashQueueRow({min_multiplier:item.crash_multiplier,max_multiplier:item.crash_multiplier,rounds:1});window.refreshFilterMenus?.();
}
function renderGamblingAdminPreview(data){
 gamblingAdminData=data;const crash=data.crash;const preview=$('admin-crash-preview');preview.replaceChildren(gameNode('small',`${crash.id===null?'NO ACTIVE ROUND':'ROUND '+crash.id} · ${crash.phase.toUpperCase()}`),gameNode('strong',data.planned_crash_multiplier===null?'—':multiplierText(data.planned_crash_multiplier)),gameNode('p',`Scheduled crash ${data.planned_crash_at_ms?new Date(data.planned_crash_at_ms).toLocaleTimeString():'not scheduled'}${crash.id===null?'':` · Current round: ${crash.mode==='controlled'?'owner-controlled':'random'}`} · Future rounds: ${data.mode==='controlled'?'owner-controlled':'random'}${data.paused?' · New wagering paused':''}`));
 $('admin-crash-scheduled').replaceChildren(...(data.queue||[]).map((r,i)=>gameNode('span',`${i+1}: ${multiplierText(r.crash_multiplier)}`,'crashchip high')));if(!data.queue?.length)$('admin-crash-scheduled').append(gameNode('p','No queued outcomes. Future rounds use the current generation mode.'));
 $('admin-crash-history').replaceChildren(...(crash.history||[]).map(r=>gameNode('span',multiplierText(r.crash_multiplier)+' · '+r.mode,'crashchip')));
}
async function refreshGamblingAdminPreview(){if(gamblingAdminLoading)return;gamblingAdminLoading=true;try{renderGamblingAdminPreview(await(await api('admin/gambling')).json());}finally{gamblingAdminLoading=false;}}
setInterval(()=>{if(!gamblingReady||document.hidden||$('gamblingview')?.hidden||gamblingBusy)return;loadGambling().catch(error=>setGamblingStatus(error.message,true));},1500);
setInterval(()=>{if(!gamblingAdminReady||document.hidden||$('moderation')?.hidden||$('admin-gambling')?.hidden)return;refreshGamblingAdminPreview().catch(error=>{const n=$('admin-crash-status');if(n)n.textContent=error.message;});},2000);
