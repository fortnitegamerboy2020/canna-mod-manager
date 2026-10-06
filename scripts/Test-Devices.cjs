 'use strict';
const fs=require('node:fs'),vm=require('node:vm'),assert=require('node:assert/strict');
const nodes=new Map(),calls=[];let redirect='';
function node(){return {hidden:false,textContent:'',children:[],append(...items){this.children.push(...items);},replaceChildren(...items){this.children=items;},addEventListener(kind,fn){this[kind]=fn;},set innerHTML(_){throw Error('Unsafe HTML rendering');}};}
const get=id=>{if(!nodes.has(id))nodes.set(id,node());return nodes.get(id);};
let devices=[{id:'current',name:'Chrome on Windows',kind:'browser',current:true,state:'Active recently',created:1,last_seen:2},{id:'remote',name:'<img onerror=attack>',kind:'desktop',current:false,state:'Idle',created:1,last_seen:1}];
const context=vm.createContext({$:get,document:{createElement:node},Date,profileId:1,currentUser:{id:1},setInterval(){},button(label,click){const n=node();n.textContent=label;n.click=click;return n;},prompt:()=> 'Bedroom PC',cannaConfirm:async()=>true,action:fn=>fn(),message(){},location:{replace(path){redirect=path;}},api:async(path,options={})=>{calls.push({path,options});if(path==='devices')return {json:async()=>devices};if(options.method==='DELETE')devices=devices.filter(d=>path!=='devices/'+d.id);return {};}});
const source=fs.readFileSync('server/web/profiles.js','utf8');vm.runInContext(source.slice(source.indexOf('async function loadDevices()')),context);
(async()=>{
 await vm.runInContext('loadDevices()',context);
 const rows=get('devicelist').children;assert.match(rows[0].children[0].children[0].textContent,/This device/);assert.equal(rows[1].children[0].children[0].textContent,'<img onerror=attack>');
 await rows[1].children[1].click();assert.ok(calls.some(c=>c.path==='devices/remote'&&JSON.parse(c.options.body).name==='Bedroom PC'));
 await get('devicelist').children[1].children[2].click();assert.ok(calls.some(c=>c.path==='devices/remote'&&c.options.method==='DELETE'));assert.equal(redirect,'');
 await get('logoutothers').click();assert.ok(calls.some(c=>c.path==='devices/logout-others'&&c.options.method==='POST'));
 await get('devicelist').children[0].children[2].click();assert.equal(redirect,'/');
 console.log('Device controls: safe names, rename, remote logout, logout others and current-device redirect passed.');
})().catch(e=>{console.error(e);process.exitCode=1;});
