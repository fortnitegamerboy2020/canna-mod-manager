const fs=require('fs'),assert=require('node:assert/strict');
const {chromium}=require(process.env.CANNA_PLAYWRIGHT_MODULE||'playwright');
(async()=>{
 const browser=await chromium.launch({headless:true,channel:'msedge'});
 try {for(const width of [1280,390]){
  const page=await browser.newPage({viewport:{width,height:900}}),errors=[];
  page.on('pageerror',error=>errors.push(error.message));
  const markup=fs.readFileSync('server/web/index.html','utf8');
  const section=markup.slice(markup.indexOf('<section id="playview"'),markup.indexOf('<section id="browseview"'));
  assert(section.includes('playfile'));
  await page.setContent(`<style>${fs.readFileSync('server/web/forum.css','utf8')}</style><button id="playnav">Play Lab</button><main>${section.replace(' hidden','')}</main>`);
  await page.evaluate(()=>{
   window.$=id=>document.getElementById(id);window.sent=[];window.errors=[];window.requests=[];
   window.action=async work=>{try{await work()}catch(e){errors.push(e.message)}};
   window.message=()=>{};window.showView=async()=>loadPlayLab();window.cannaConfirm=async()=>true;
   window.button=(text,work)=>{const b=document.createElement('button');b.textContent=text;b.onclick=()=>action(work);return b;};
   window.api=async path=>{requests.push(path);return {json:async()=>({rooms:[],reports:[],channels:[],releases:[],total:51,channel_total:0})}};
   window.json=async(path,data)=>{sent.push(structuredClone(data));if(['create','join','update'].includes(data.action))return {id:'room1',invite:'room1:secretcode',host:true,manifest:data.manifest,members:[{alias:'Player',active:true,ready:false,checks:[{level:'unknown',message:'Build is unknown'}]}]};if(data.action==='issue')return {ticket:'private-ticket'};if(data.action==='publish')return {channel:'channel1',release:'release1'};return {closed:true};};
  });
  await page.addScriptTag({content:fs.readFileSync('server/web/play-lab.js','utf8')});
  await page.click('#playnav');
  assert.equal(await page.locator('#playshare').isDisabled(),true);
  await page.setInputFiles('#playfile',{name:'fixture.play.json',mimeType:'application/json',buffer:Buffer.from(JSON.stringify({game:550,branch:'public',build:'123',loader:'source-vpk',manager:'0.2.31',mods:[],shared_configs:{},device_name:'Private-PC',local_path:'C:/Users/private'}))});
  await page.waitForFunction(()=>playManifest!==null);
  assert.equal((await page.evaluate(()=>playCurrent())).device_name,undefined);
  assert.equal((await page.evaluate(()=>playCurrent())).local_path,undefined);
  await page.locator('summary').filter({hasText:'Report a session'}).click();
  await page.fill('#playnote','Good session\nC:\\Users\\private\nIP 192.168.1.2\nsession=credential');
  await page.click('#playpreviewbutton');
  assert(!await page.locator('#playpreview').innerText().then(s=>s.includes('credential')||s.includes('192.168')||s.includes('Users')));
  await page.fill('#playnote','Changed after preview');assert(await page.locator('#playshare').isDisabled());
  await page.click('#playpreviewbutton');await page.click('#playshare');
  await page.waitForFunction(()=>sent.some(x=>x.action==='report'));
  assert.equal(await page.evaluate(()=>sent.find(x=>x.action==='report').note),'Changed after preview');
  await page.locator('summary').filter({hasText:'Ready to play lobby'}).click();
  await page.click('#playcreate');await page.waitForFunction(()=>sent.some(x=>x.action==='create'));
  assert((await page.locator('#playmembers').innerText()).includes('Needs checks'));
  await page.click('#playupdate');await page.click('#playclose');
  await page.fill('#playinvite','room1:secretcode');await page.click('#playjoin');
  await page.click('#playpreviewbutton');await page.click('#playissue');
  await page.locator('summary').filter({hasText:'Stable and experimental'}).click();
  await page.click('#playpublish');await page.click('#playnext');
  await page.waitForFunction(()=>requests.some(x=>x.includes('page=2')));
  assert.deepEqual(errors,[]);assert.deepEqual(await page.evaluate(()=>errors),[]);
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`horizontal overflow at ${width}`);
  await page.screenshot({path:`target/play-lab-${width}.png`});
  await page.close();
 }
 console.log('Play Lab desktop/mobile workflow and privacy checks passed');
 }finally{await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1});
