const fs=require('node:fs'),vm=require('node:vm'),assert=require('node:assert/strict');
const fields={};
for(const id of ['librarygame','libraryversion','libraryloader','librarysearch','librarytype','libraryprovider','librarycount','mods','packs']) fields[id]={value:'',options:[],section:{hidden:false},replaceChildren(...rows){this.rows=rows;},closest(){return this.section;}};
fields.librarygame.value='minecraft';fields.librarytype.value='shader';fields.libraryversion.value='1.21.1';fields.libraryloader.value='neoforge';fields.libraryprovider.value='curseforge';
const context={URLSearchParams,Map,Option:function(name,value){return {name,value};},location:{search:''},$:id=>fields[id],entry:item=>item,libraryItems:{mods:[
 {name:'Matching shader',details:{game:'Minecraft',content_type:'shader',provider:'curseforge',loaders:['NeoForge'],game_versions:['1.21.1']}},
 {name:'Wrong content',details:{game:'Minecraft',content_type:'mod',provider:'curseforge',loaders:['neoforge'],game_versions:['1.21.1']}},
 {name:'Wrong version',details:{game:'Minecraft',content_type:'shader',provider:'curseforge',loaders:['neoforge'],game_versions:['1.20.1']}}
],packs:[]}};
const source=fs.readFileSync('server/web/library.js','utf8');vm.createContext(context);vm.runInContext(source.slice(source.indexOf('const gameNames='),source.indexOf('function updateUploadGame'))+source.slice(source.indexOf('let initialLibraryFilters='),source.indexOf('for(const id of')),context);vm.runInContext('initialLibraryFilters=false;renderLibrary();',context);
assert.equal(fields.mods.rows.length,1);assert.equal(fields.mods.rows[0].name,'Matching shader');assert.equal(fields.packs.section.hidden,true);assert.equal(fields.librarycount.textContent,'1 item');
fields.libraryloader.value='fabric';vm.runInContext('renderLibrary();',context);assert.equal(fields.mods.rows.length,0);assert.equal(fields.librarycount.textContent,'0 items');
for(const id of ['librarygame','librarytype','libraryversion','libraryloader','libraryprovider']) fields[id].value='';
context.libraryItems.mods.push({app_id:42,name:'Unsupported game mod',description:'',author:''},{app_id:632360,name:'Supported preview mod',description:'',author:''});
context.libraryItems.packs.push({name:'Minecraft pack',game:{app_id:4294967295,name:'Minecraft'}});
vm.runInContext("librarySupportedGames.set('632360','Risk of Rain 2');renderLibrary();",context);
assert.equal(fields.mods.rows.length,4);assert(!fields.librarygame.rows.some(o=>o.value==='42'));assert(fields.librarygame.rows.some(o=>o.value==='632360'));
assert.equal(fields.packs.rows.length,1);fields.librarygame.value='minecraft';vm.runInContext('renderLibrary();',context);assert.equal(fields.packs.rows.length,1);
console.log('CurseForge content, game, version, provider and case-insensitive loader filters passed.');
