const fs=require('fs'),vm=require('vm'),assert=require('assert/strict');
function node(tag){return {tag,children:[],textContent:'',disabled:false,append(...children){this.children.push(...children)},addEventListener(){}};}
const source=fs.readFileSync('server/web/app.js','utf8');const entry=source.slice(source.indexOf('function entry('),source.indexOf('const libraryItems'));
for(const admin of [false,true]){const ctx=vm.createContext({document:{createElement:node},libraryGameName:()=>"Fixture game",currentUser:{admin,username:'reviewer'},console});vm.runInContext(entry,ctx);const result=vm.runInContext(`entry({id:'test',name:'Mod',version:'1',author:'uploader',review_status:'pending'},'mods')`,ctx);const buttons=result.children.at(-1).children;assert.equal(buttons.find(b=>b.textContent==='Download').disabled,true);assert.equal(buttons.some(b=>b.textContent==='Approve mod'),admin);}
console.log('Pending-download UI and administrator-only approval controls passed.');

function reviewNode(tag){
 return {tag,children:[],textContent:'',disabled:false,value:'',handlers:{},classList:{add(){}},
  append(...children){this.children.push(...children)},replaceChildren(...children){this.children=children;this.textContent=''},
  addEventListener(type,handler){this.handlers[type]=handler},scrollIntoView(){},focus(){},showModal(){},
  set innerHTML(value){throw Error('Review evidence must use textContent, not innerHTML');}};
}
const reviewSource=fs.readFileSync('server/web/review.js','utf8');
const elements=new Map();const element=id=>{if(!elements.has(id))elements.set(id,reviewNode('div'));return elements.get(id)};
element('findingfilter').value='open';
const reviewContext=vm.createContext({document:{createElement:reviewNode,getElementById:element,createTextNode:text=>({tag:'text',textContent:text}),createDocumentFragment:()=>reviewNode('fragment')},location:{pathname:'/review/fixture'},console});
vm.runInContext(reviewSource.slice(0,reviewSource.indexOf('async function load()')),reviewContext);
const fixture={status:'complete',sha256:'fixture-sha',mod_name:'Diagnostic fixture',files:[{name:'SmokeTestRunner.cs',kind:'decompiled',text:'// local fixture\nDirectory.CreateDirectory(output);\nFile.WriteAllText(report, text);\nCapturePng(image);'}],inventory:[],
 findings:[{id:'filesystem-group',rule:'filesystem',severity:'review',title:'Diagnostic output capability',file:'SmokeTestRunner.cs',line:2,evidence:'Directory.CreateDirectory(output);',context:'Opt-in diagnostics. <img src=x onerror=alert(1)> Caller controls output.',accepted:false,locations:[{file:'SmokeTestRunner.cs',line:2,evidence:'Directory.CreateDirectory(output);'},{file:'SmokeTestRunner.cs',line:3,evidence:'File.WriteAllText(report, text);'}]}],
 observations:[{id:'source-url',rule:'source-url',title:'Source URL reference',file:'SmokeTestRunner.cs',line:1,evidence:'https://example.invalid/source/<script>alert(1)</script>',accepted:false}]};
function render(reportFixture){reviewContext.fixture=reportFixture;vm.runInContext('report=fixture;renderFindings();renderObservations();',reviewContext);}
function descendants(root){return [root,...root.children.flatMap(child=>child.children?descendants(child):[child])];}
render(fixture);
assert.equal(element('approve').disabled,true,'Unresolved capabilities must block approval');
const findingNodes=descendants(element('findings')),observationNodes=descendants(element('observations'));
assert(findingNodes.some(n=>n.textContent===`Context: ${fixture.findings[0].context}`),'Context should remain exact inert text');
assert(findingNodes.some(n=>n.textContent==='1 related evidence location'),'Primary location should be deduplicated');
const related=findingNodes.find(n=>n.tag==='button'&&n.textContent==='SmokeTestRunner.cs:3');assert(related);
related.handlers.click();assert.equal(element('filename').textContent,'SmokeTestRunner.cs');assert(element('code').children.some(n=>n.id==='line-3'));
assert(observationNodes.some(n=>n.textContent===fixture.observations[0].evidence),'Observation evidence should remain exact inert text');
assert(!observationNodes.some(n=>/Accept|Reopen|Record decision/.test(n.textContent)),'Observations must not have review decision controls');
render({...fixture,findings:fixture.findings.map(f=>({...f,accepted:true}))});
assert.equal(element('approve').disabled,false,'Unaccepted informational observations must not block approval');
for(const rule of ['coverage','antivirus','packing']){render({...fixture,findings:[{id:rule,rule,severity:'high',title:rule,accepted:false}]});assert.equal(element('approve').disabled,true,`${rule} findings must block approval`);}
const groupedCoverage={...fixture,findings:[{id:'grouped-coverage',rule:'coverage',severity:'high',title:'Source preview coverage limit',file:'archive/Generated.dll',evidence:'Two source previews were omitted.',accepted:false,locations:[
 {id:'coverage-a',file:'Generated/UnavailableA.cs',line:1,evidence:'Source preview omitted. <img src=x onerror=alert(1)>'},
 {id:'coverage-b',file:'Generated/UnavailableB.cs',line:2,evidence:'Source preview omitted. <script>alert(1)</script>'}
]}]};
render(groupedCoverage);assert.equal(element('approve').disabled,true,'Grouped coverage must remain an unresolved approval blocker');
const coverageNodes=descendants(element('findings'));
assert(coverageNodes.some(n=>n.textContent==='2 related evidence locations'),'Coverage locations must not imply retained source');
for(const location of groupedCoverage.findings[0].locations){
 assert(coverageNodes.some(n=>n.tag==='pre'&&n.textContent===location.evidence),'Omitted-source evidence must render as inert text');
 assert.equal(coverageNodes.find(n=>n.tag==='button'&&n.textContent===`${location.file}:${location.line}`).disabled,true,'Missing source previews must not offer navigation');
}
for(const status of ['rejected','failed','queued','pending']){render({...fixture,status,findings:[]});assert.equal(element('approve').disabled,true,`${status} analysis must block approval`);}
render({...fixture,status:'rejected'});assert(!descendants(element('findings')).some(n=>n.textContent==='Accept finding with reason'),'Rejected reports must not offer acceptance');
render({status:'complete',files:fixture.files,findings:[]});assert.equal(element('approve').disabled,false);assert.equal(element('observations').textContent,'No informational observations in this report.','Legacy reports should work');
console.log('Review context, related navigation, inert evidence, informational observations, legacy reports and approval gates passed.');

async function browserTest(){
 const {chromium}=require(process.env.CANNA_PLAYWRIGHT_MODULE||'playwright');
 const browser=await chromium.launch({headless:true,channel:'msedge'});
 try{
  const page=await browser.newPage({viewport:{width:1280,height:900}});
  await page.route('**/*',route=>{const path=new URL(route.request().url()).pathname;
   if(path==='/api/v1/mods/fixture/analysis')return route.fulfill({contentType:'application/json',body:JSON.stringify(fixture)});
   if(path==='/review/fixture')return route.fulfill({contentType:'text/html',body:fs.readFileSync('server/web/review.html','utf8')});
   if(path==='/review.js'||path==='/confirm.js'||path==='/forum.css')return route.fulfill({contentType:path.endsWith('.css')?'text/css':'application/javascript',body:fs.readFileSync('server/web'+path,'utf8')});
   return route.fulfill({status:404,body:''});
  });
  await page.goto('http://canna-fixture.invalid/review/fixture');
  await page.getByText('Informational · Source URL reference',{exact:true}).waitFor();
  assert.equal(await page.locator('#approve').isDisabled(),true);
  assert.equal(await page.locator('#observations button').count(),1,'Only source navigation belongs on observation cards');
  assert.equal(await page.locator('#findings img,#observations script').count(),0,'Evidence must not create markup');
  await page.getByText('1 related evidence location',{exact:true}).click();
  await page.getByRole('button',{name:'SmokeTestRunner.cs:3',exact:true}).click();
  assert.equal(await page.locator('#line-3').getAttribute('class'),'codeline flaggedline');
  await page.screenshot({path:'target/mod-review-preview-wide.png',fullPage:true});
  await page.setViewportSize({width:390,height:800});
  await page.screenshot({path:'target/mod-review-preview-narrow.png',fullPage:true});
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth),'Review page must fit mobile width');
  await page.evaluate(data=>{report={...data,findings:data.findings.map(f=>({...f,accepted:true}))};renderFindings();renderObservations();},fixture);
  assert.equal(await page.locator('#approve').isDisabled(),false,'Only findings affect approval');
  await page.evaluate(data=>{report=data;renderFindings();renderObservations();},groupedCoverage);
  assert.equal(await page.locator('#approve').isDisabled(),true,'Grouped coverage must remain an unresolved approval blocker');
  await page.getByText('2 related evidence locations',{exact:true}).click();
  for(const location of groupedCoverage.findings[0].locations){
   assert.equal(await page.getByRole('button',{name:`${location.file}:${location.line}`,exact:true}).isDisabled(),true);
   assert.equal(await page.getByText(location.evidence,{exact:true}).isVisible(),true,'Omitted-source evidence must remain visible');
  }
  assert.equal(await page.locator('#findings img,#findings script').count(),0,'Grouped coverage evidence must not create markup');
  await page.evaluate(()=>{report={status:'rejected',files:report.files,findings:[],observations:[]};renderFindings();renderObservations();});
  assert.equal(await page.locator('#approve').isDisabled(),true);
  await page.evaluate(()=>{report={status:'complete',files:report.files,findings:[]};renderFindings();renderObservations();});
  assert.equal(await page.locator('#approve').isDisabled(),false);
  console.log('Desktop/mobile browser review rendering, related navigation and approval gates passed.');
 }finally{await browser.close();}
}
if(process.argv.includes('--browser'))browserTest().catch(error=>{console.error(error);process.exitCode=1;});
