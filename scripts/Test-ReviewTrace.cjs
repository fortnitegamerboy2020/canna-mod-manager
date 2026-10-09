// Production review/list handlers in Edge; synthetic HTTP only.
const fs=require('node:fs'),assert=require('node:assert/strict');
const {chromium}=require(process.env.CANNA_PLAYWRIGHT_MODULE||'playwright');
(async()=>{
 const browser=await chromium.launch({headless:true,channel:'msedge'});
 try{
 for(const width of [1280,390]){
  const page=await browser.newPage({viewport:{width,height:900}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
  let submissions=0;
  const trace={method:'Replace',limits:'Lexical hints only; no path confinement is proven.',paths:[{expression:'temporary',assignments:[{file:'source/Mod.cs',line:2,name:'temporary',expression:'path + ".rptmp"'},{file:'source/Mod.cs',line:1,name:'path (candidate caller argument)',expression:'"plugins/replacement.dll"'}]}],callers:[{file:'source/Mod.cs',line:1,method:'Awake',depth:1,arguments:['<img src=x onerror=alert(1)>'],ambiguous:false}]};
  let report={status:'complete',sha256:'a'.repeat(64),mod_name:'Repair fixture',game_name:'ROUNDS',files:[{name:'source/Mod.cs',text:'Replace("plugins/replacement.dll", bytes);\nstring temporary=path+".rptmp";\nFile.WriteAllBytes(temporary,bytes);',kind:'decompiled'}],findings:[{id:'trace',rule:'filesystem',file:'source/Mod.cs',line:3,evidence:'File.WriteAllBytes(temporary,bytes);',title:'Executable file write or replacement requires review',severity:'high',trace}],observations:[]};
  await page.route('**/*',async route=>{const path=new URL(route.request().url()).pathname;
   if(path==='/api/v1/mods/fixture/analysis'){
    if(route.request().method()==='POST'){submissions++;await new Promise(r=>setTimeout(r,100));if(submissions===1)return route.fulfill({status:429,contentType:'application/json',body:JSON.stringify({error:'Analysis queue is full; try shortly'})});report={...report,status:'queued'};return route.fulfill({contentType:'application/json',body:'{"ok":true}'});}
    return route.fulfill({contentType:'application/json',body:JSON.stringify(report)});
   }
   if(path==='/review/mods/fixture')return route.fulfill({contentType:'text/html',body:fs.readFileSync('server/web/review.html','utf8')});
   if(['/review.js','/confirm.js','/forum.css','/review-workspace.css'].includes(path))return route.fulfill({contentType:path.endsWith('.css')?'text/css':'application/javascript',body:fs.readFileSync('server/web'+path,'utf8')});
   return route.fulfill({status:404,body:''});
  });
  await page.goto('https://canna-fixture.invalid/review/mods/fixture');
  await page.getByRole('heading',{name:'Repair fixture · ROUNDS',exact:true}).waitFor();
  await page.locator('#tab-findings').click();await page.locator('#findings').getByText('Function & destination trace · Replace',{exact:true}).click();
  assert((await page.locator('#findings').textContent()).includes('plugins/replacement.dll'));
  assert((await page.locator('#findings').textContent()).includes('<img src=x onerror=alert(1)>'));
  assert.equal(await page.locator('#findings img').count(),0);assert(await page.locator('#approve').isDisabled());
  await page.evaluate(()=>{cannaConfirm=async()=>true;});await page.locator('#rescan').click();assert(await page.locator('#rescan').isDisabled());
  await page.waitForFunction(()=>!document.getElementById('rescan').disabled);assert((await page.locator('#reviewstatus').textContent()).includes('Analysis queue is full'));
  await page.locator('#rescan').click();await page.waitForFunction(()=>document.getElementById('reviewstatus').textContent.includes('queued'));
  assert.equal(submissions,2);assert(await page.locator('#approve').isDisabled());
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));assert.deepEqual(errors,[]);
  await page.screenshot({path:`target/review-trace-${width}.png`,fullPage:true});await page.close();
 }
 const page=await browser.newPage();await page.route('**/*',r=>r.fulfill({contentType:'text/html',body:'<html></html>'}));await page.goto('https://canna-fixture.invalid/admin');
 await page.setContent('<select id="reviewgame"><option value="">All games</option><option value="1557740">ROUNDS</option></select><select id="reviewsort"><option value="name">By mod name</option></select><div id="reviewqueuestatus"></div><div id="modreviewlist"></div>');
 await page.evaluate(()=>{
  window.$=id=>document.getElementById(id);window.listPages=new Map();window.calls=[];window.adminNode=(tag,text,className)=>{const n=document.createElement(tag);if(text)n.textContent=text;if(className)n.className=className;return n;};window.button=(text,fn)=>{const n=adminNode('button',text);n.onclick=fn;return n;};window.action=fn=>fn();
  window.api=async path=>{calls.push(path);return {json:async()=>({items:[{id:'mod',name:'Repair mod',version:'1',size:200,author:'Author',game_name:'ROUNDS',app_id:1557740,analysis_status:'complete',unresolved_findings:1}],total:1,games:[{app_id:1557740,name:'ROUNDS'}],queue:{scheduled:0,waiting:0,failed:0}})};};
 });
 const app=fs.readFileSync('server/web/app.js','utf8');await page.addScriptTag({content:app.slice(app.indexOf('async function pagedList('),app.indexOf('let prefetchReady='))});
 const admin=fs.readFileSync('server/web/admin.js','utf8');await page.addScriptTag({content:admin.slice(admin.indexOf('async function loadModReviews()'),admin.indexOf('\n',admin.indexOf('async function loadModReviews()')))});
 await page.locator('#reviewgame').selectOption('1557740');await page.evaluate(()=>loadModReviews());
 assert((await page.evaluate(()=>calls[0])).includes('app_id=1557740'));assert((await page.evaluate(()=>calls[0])).includes('sort=name'));
 assert((await page.locator('#modreviewlist').textContent()).includes('ROUNDS'));assert(!(await page.locator('#modreviewlist').textContent()).includes('1557740'));
 assert.equal(await page.locator('#reviewgame').inputValue(),'1557740');await page.close();
 console.log('Review trace, readable game labels, filtering/sorting queries, inert evidence, retry errors and approval gates passed at desktop/mobile widths.');
 }finally{await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
