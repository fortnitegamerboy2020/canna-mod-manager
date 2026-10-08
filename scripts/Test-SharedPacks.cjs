const fs=require('node:fs'),assert=require('node:assert/strict');
const {chromium}=require(process.env.CANNA_PLAYWRIGHT_MODULE||'playwright');
(async()=>{
 const browser=await chromium.launch({headless:true,channel:'msedge'});
 for(const width of [1280,390]){
  const page=await browser.newPage({viewport:{width,height:800}});await page.setContent(`<style>${fs.readFileSync('server/web/forum.css','utf8')}</style><main><section id="packview"></section></main>`);
  await page.evaluate(()=>{
   window.$=id=>document.getElementById(id);window.downloads=[];window.paths=[];window.messages=[];window.navigatePage=p=>paths.push(p);window.message=m=>messages.push(m);window.action=fn=>fn();window.downloadToApp=async(item,kind)=>downloads.push({item,kind});
   window.info={id:'11111111-1111-4111-8111-111111111111',url:'https://cannamods.vip/packs/11111111-1111-4111-8111-111111111111',author:'Pack author',revision:2,can_update:true,ready:true,manifest:{name:'Family game night',description:'Matching pinned selections',game:{name:'ROUNDS'},mods:[{name:'Original mod',version:'1.2',enabled:true,description:'Original description',dependencies:['Required API'],provenance:{authors:'Original author',source_url:'https://thunderstore.io/c/rounds/p/Author/Mod/'}},{name:'Required API',version:'1',enabled:true,description:'API',dependencies:[]}]}};
   window.api=async()=>({json:async()=>info});
  });
  // Read the UUID through the actual route parser, without requiring the real site.
  await page.addScriptTag({content:fs.readFileSync('server/web/shared-packs.js','utf8').replace("location.pathname.split('/')[2]","'11111111-1111-4111-8111-111111111111'")});
  await page.evaluate(()=>loadSharedPack());assert(await page.getByRole('heading',{name:'Family game night'}).isVisible());assert(await page.getByText('ROUNDS · by Pack author · Revision 2',{exact:true}).isVisible());assert(await page.getByRole('heading',{name:'Included mods (2)'}).isVisible());
  assert.equal(await page.getByRole('link',{name:'Original project'}).getAttribute('href'),'https://thunderstore.io/c/rounds/p/Author/Mod/');
  await page.getByRole('button',{name:'Install in Canna'}).click();assert.equal(await page.evaluate(()=>downloads[0].kind),'packs');assert.equal(await page.evaluate(()=>downloads[0].item.name),'Family game night');
  await page.evaluate(()=>{info.ready=false;return loadSharedPack();});assert(await page.getByRole('button',{name:'Install in Canna'}).isDisabled());
  await page.evaluate(()=>{info.ready=true;info.can_update=false;info.manifest.name='<img src=x onerror=alert(1)>';return loadSharedPack();});assert.equal(await page.locator('img').count(),0);assert.equal(await page.getByText(/You created this pack/).count(),0);assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),true);
  await page.getByRole('button',{name:'← My library'}).click();assert.equal(await page.evaluate(()=>paths[0]),'/library');await page.close();console.log(`Shared pack ${width}px: details, attribution, handoff, approval gate, creator controls, escaping and layout passed`);
 }
 await browser.close();
})().catch(e=>{console.error(e);process.exit(1)});
