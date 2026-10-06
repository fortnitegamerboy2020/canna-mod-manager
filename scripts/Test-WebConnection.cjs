'use strict';
const fs=require('node:fs'),vm=require('node:vm'),assert=require('node:assert/strict');
const source=fs.readFileSync('server/web/library.js','utf8');
const routing=fs.readFileSync('server/web/community.js','utf8');
async function check(path,search,approved) {
 const nodes=new Map();const calls=[];let message='';
 const get=id=>{if(!nodes.has(id))nodes.set(id,{hidden:true,disabled:false,textContent:'',addEventListener(_,fn){this.click=fn;}});return nodes.get(id);};
 const context=vm.createContext({$:get,location:{pathname:path,search},URLSearchParams,updateNavigation(){},loadLibrary(){calls.push('library');},loadTopics(){},loadThread(){},loadAdmin(){},openThread:'',message(t){message=t;},action:fn=>fn(),api:async(url,opts)=>{assert.equal(url,'desktop/approve');assert.equal(JSON.parse(opts.body).request,'a'.repeat(64));calls.push('approve');},json:()=>{throw new Error('Unexpected legacy connect');}});
 const start=routing.indexOf('function showView(name)');const end=routing.indexOf("$('librarynav')",start);vm.runInContext(routing.slice(start,end),context);
 vm.runInContext(source.slice(source.indexOf('const desktopRequest=')),context);
 assert.equal(get('libraryview').hidden,false);assert.equal(get('forumview').hidden,true);assert.ok(calls.includes('library'));
 if(approved){assert.match(get('connectdesktop').textContent,/AAAAAA/);await get('connectdesktop').click();assert.ok(calls.includes('approve'));assert.equal(get('connectdesktop').disabled,true);assert.match(message,/Connection approved/);}
}
(async()=>{await check('/connect','?request='+ 'a'.repeat(64),true);await check('/','?game=minecraft',false);console.log('Connection approval and game-library routes passed.');})().catch(e=>{console.error(e);process.exitCode=1;});
