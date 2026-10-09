// Real browser fixtures only; never submit reports to the production server.
const fs=require('node:fs'),assert=require('node:assert/strict');
const {chromium}=require(process.env.CANNA_PLAYWRIGHT_MODULE||'playwright');
(async()=>{
 const browser=await chromium.launch({channel:'msedge',headless:true});
 try{for(const width of [1280,390]){
  const page=await browser.newPage({viewport:{width,height:900}});const errors=[];
  page.on('pageerror',e=>errors.push(e.message));
  await page.setContent('<div id="message"></div><section id="moderation"><h2>Admin</h2><div id="adminsummary"></div><div id="memberlist"></div><div id="ownerlog"></div></section>');
  await page.addStyleTag({content:fs.readFileSync('server/web/forum.css','utf8')+'\n'+fs.readFileSync('server/web/admin-games.css','utf8')});
  await page.evaluate(()=>{
   window.$=id=>document.getElementById(id);window.currentUser={id:99,role:'admin',admin:true};window.calls=[];window.fixtureReports=[];
   window.action=async fn=>{try{await fn();}catch(e){document.getElementById('message').textContent=e.message;}};
   window.button=(label,fn)=>{const n=document.createElement('button');n.textContent=label;n.addEventListener('click',()=>action(fn));return n;};
   window.setupLoungeAdmin=()=>{};window.message=text=>document.getElementById('message').textContent=text;
   window.api=async path=>{calls.push(path);const data=path==='admin/launcher-diagnostics'?{reports:fixtureReports}:path==='admin/rebound-diagnostics'?{reports:fixtureBliss}:path==='me'?{id:99,kash:0}:{};return new Response(JSON.stringify(data),{headers:{'Content-Type':'application/json'}});};
   window.fixtureBliss=[];
  });
  const library=fs.readFileSync('server/web/library.js','utf8');await page.addScriptTag({content:library.slice(library.indexOf('const gameNames='),library.indexOf('function updateUploadGame'))});
  await page.addScriptTag({content:fs.readFileSync('server/web/admin.js','utf8')});
  await page.evaluate(async()=>{setupAdmin();await selectAdminTab('diagnostics');});
  assert.match(await page.locator('#launcherdiagnosticlist').textContent(),/off until enabled in desktop Settings/);
  assert.match(await page.locator('#rebounddiagnosticlist').textContent(),/desktop Settings/);
  await page.evaluate(()=>{
   fixtureReports=[{created:1700000000,report:{schema:1,game_id:1686940,desktop_version:'0.2.46',platform:'windows',operation:'prepare',phase:'loader',code:'access_denied',http_status:403,mod_count:14,loader_present:false,bliss_enabled:false,session:'a'.repeat(32)}}];
   const hashes={game:'g',mods:'m',assets:'a',config:'c',content:'x',files:[]};fixtureBliss=[{created:1700000000,report:{session:'b'.repeat(32),actor:1,guard_version:'0.1.3',category:'settings',local:hashes,peers:[]}}];
  });
  await page.getByRole('button',{name:'Refresh reports',exact:true}).click();await page.waitForFunction(()=>document.getElementById('launcherdiagnosticlist').textContent.includes('HTTP 403'));
  assert.match(await page.locator('#launcherdiagnosticlist').textContent(),/Bopl Battle/);assert.match(await page.locator('#launcherdiagnosticlist').textContent(),/14 selected mods/);
  assert.match(await page.locator('#rebounddiagnosticlist').textContent(),/Actor 1/);
  await page.evaluate(async()=>{fixtureReports[0].report.code='<img src=x onerror="window.injected=true">';await loadLauncherDiagnostics();});
  assert.equal(await page.locator('#launcherdiagnosticlist img').count(),0);assert.equal(await page.evaluate(()=>!!window.injected),false);
  await page.evaluate(async()=>{fixtureReports[0].report.code='access_denied';await loadLauncherDiagnostics();});
  assert.equal(await page.evaluate(()=>calls.every(path=>path==='me'||path.startsWith('admin/'))),true);
  assert.deepEqual(errors,[]);assert.equal(await page.locator('#message').textContent(),'');
  await page.screenshot({path:`target/launcher-diagnostics-${width}.png`,fullPage:true});await page.close();
  console.log(`${width}px: diagnostics navigation, real response decoding, game labels, refresh and inert text passed`);
 }}finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
