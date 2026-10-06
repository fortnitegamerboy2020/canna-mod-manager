const fs=require('node:fs'),vm=require('node:vm'),assert=require('node:assert/strict');
const fields={};
for(const id of ['librarygame','libraryversion','libraryloader','librarysearch','librarytype','libraryprovider','librarycount','mods','packs']) fields[id]={value:'',options:[],section:{hidden:false},replaceChildren(...rows){this.rows=rows;},closest(){return this.section;}};
fields.librarygame.value='minecraft';fields.librarytype.value='shader';fields.libraryversion.value='1.21.1';fields.libraryloader.value='neoforge';fields.libraryprovider.value='curseforge';
const context={URLSearchParams,Map,Option:function(name,value){return {name,value};},location:{search:''},$:id=>fields[id],entry:item=>item,libraryItems:{mods:[
 {name:'Matching shader',details:{game:'Minecraft',content_type:'shader',provider:'curseforge',loaders:['NeoForge'],game_versions:['1.21.1']}},
 {name:'Wrong content',details:{game:'Minecraft',content_type:'mod',provider:'curseforge',loaders:['neoforge'],game_versions:['1.21.1']}},
 {name:'Wrong version',details:{game:'Minecraft',content_type:'shader',provider:'curseforge',loaders:['neoforge'],game_versions:['1.20.1']}}
],packs:[]}};
const source=fs.readFileSync('server/web/library.js','utf8');vm.createContext(context);vm.runInContext(source.slice(0,source.indexOf('for(const id of')),context);vm.runInContext('initialLibraryFilters=false;renderLibrary();',context);
assert.equal(fields.mods.rows.length,1);assert.equal(fields.mods.rows[0].name,'Matching shader');assert.equal(fields.packs.section.hidden,true);assert.equal(fields.librarycount.textContent,'1 item');
fields.libraryloader.value='fabric';vm.runInContext('renderLibrary();',context);assert.equal(fields.mods.rows.length,0);assert.equal(fields.librarycount.textContent,'0 items');
console.log('CurseForge content, game, version, provider and case-insensitive loader filters passed.');
