const fs=require('fs'), assert=require('assert');
const {chromium}=require(process.env.CANNA_PLAYWRIGHT_MODULE || 'playwright');
(async()=>{
 const browser=await chromium.launch({headless:true,channel:'msedge'});
 for(const width of [1280,390]){
  const page=await browser.newPage({viewport:{width,height:900}});
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.setContent(`<style>${fs.readFileSync('server/web/forum.css','utf8')}</style><main><div id="externalimport"></div><button id="submissionsnav">My submissions</button></main>`);
  await page.evaluate(()=>{
   window.$=id=>document.getElementById(id);window.requests=[];window.downloads=[];window.fail=false;window.pending=false;
   window.api=async path=>{requests.push(path);if(path==='providers/games')return {json:async()=>({games:[{name:'ROUNDS',community:'rounds'},{name:'Valheim',community:'valheim'}]})};if(fail)throw new Error('Provider request failed (403)');return {json:async()=>({items:[{name:'Fixture Mod',authors:'Original Author',author_url:'https://thunderstore.io/c/rounds/p/Author/',source_url:'https://thunderstore.io/c/rounds/p/Author/Mod/',description:'Original description',downloads:1234,rating:5,rating_label:'upvotes'}],categories:[{name:'Tools',slug:'tools'}],has_more:!path.includes('page=2')})};};
   window.json=async(path,body)=>path.endsWith('preview')?{name:'Fixture Mod',versions:[{id:'1',name:'1',game_versions:[]}]}:{id:'fixture',approved:!pending};
   window.downloadToApp=async item=>downloads.push(item);
  });
  await page.addScriptTag({content:fs.readFileSync('server/web/provider-browser.js','utf8')});
  await page.getByRole('button',{name:'Search',exact:true}).click();
  await page.getByRole('heading',{name:'Fixture Mod'}).waitFor();
  assert.equal(await page.getByRole('link',{name:'Original Author'}).getAttribute('href'),'https://thunderstore.io/c/rounds/p/Author/');
  await page.getByLabel('Sort').selectOption('downloads');
  await page.getByRole('button',{name:'Search',exact:true}).click();
  await page.waitForFunction(()=>requests.at(-1).includes('order=downloads'));
  await page.getByRole('button',{name:'Next',exact:true}).click();
  await page.getByText('Page 2',{exact:true}).waitFor();
  assert(await page.getByRole('button',{name:'Next',exact:true}).isDisabled());
  await page.getByRole('button',{name:'Add to Canna & download'}).click();
  await page.waitForFunction(()=>downloads.length===1);
  await page.evaluate(()=>pending=true);
  await page.getByRole('button',{name:'Add to Canna & download'}).click();
  await page.getByText('Added. Scanning or moderator review is required before download.',{exact:false}).waitFor();
  assert.equal(await page.evaluate(()=>downloads.length),1);
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  await page.evaluate(()=>fail=true);
  await page.getByRole('button',{name:'Search',exact:true}).click();
  await page.getByText('Provider request failed (403)',{exact:true}).waitFor();
  assert.deepEqual(errors,[]);
  console.log(`Provider browser ${width}px: filters, pagination, attribution, approval gating, provider errors and layout passed`);
  await page.close();
 }
 await browser.close();
})().catch(e=>{console.error(e);process.exit(1)});
