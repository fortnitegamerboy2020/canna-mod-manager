 'use strict';
const fs=require('node:fs'),vm=require('node:vm'),assert=require('node:assert/strict');
const source=fs.readFileSync('server/web/connect.js','utf8');
async function fixture(search,answer=204) {
 const nodes=new Map(),calls=[];
 const get=id=>{if(!nodes.has(id))nodes.set(id,{value:'',disabled:false,textContent:'',focus(){},addEventListener(name,fn){this[name]=fn;}});return nodes.get(id);};
 const context=vm.createContext({document:{getElementById:get},location:{search},history:{replaceState(_,__,path){assert.equal(path,'/connect');}},URLSearchParams,fetch:async(path,options)=>{calls.push(JSON.parse(options.body));assert.equal(path,'/api/v1/desktop/approve');assert.equal(options.credentials,'same-origin');return {ok:answer===204,text:async()=>JSON.stringify({error:'Incorrect connection code'})};}});
 vm.runInContext(source,context);
 return {get,calls,async enter(value){get('connectioncode').value=value;get('connectioncode').input();await new Promise(resolve=>setImmediate(resolve));}};
}
(async()=>{
 const request='a'.repeat(64),page=await fixture('?request='+request);
 await page.enter('ABC');assert.equal(page.calls.length,0);
 await page.enter(' ab-c123 ');assert.equal(page.calls.length,1);assert.equal(page.calls[0].code,'ABC123');assert.equal(page.calls[0].request,request);
 assert.equal(page.get('connectioncode').disabled,true);assert.match(page.get('connectionstatus').textContent,/verified/);
 await page.enter('FFF111');assert.equal(page.calls.length,1);
 const wrong=await fixture('?request='+request,400);
 await wrong.enter('ABC123');await wrong.enter('ABC123');assert.equal(wrong.calls.length,1);assert.match(wrong.get('connectionstatus').textContent,/Incorrect/);
 await wrong.enter('DEF456');assert.equal(wrong.calls.length,2);
 const empty=await fixture('');assert.equal(empty.get('connectioncode').disabled,true);assert.equal(empty.calls.length,0);
 assert.doesNotMatch(fs.readFileSync('server/web/library.js','utf8'),/desktop\/approve|desktop\/connect/);
 console.log('Dedicated code verification typing, paste, retries, completion and missing request passed.');
})().catch(error=>{console.error(error);process.exitCode=1;});
