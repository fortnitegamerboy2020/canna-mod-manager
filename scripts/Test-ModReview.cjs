const fs=require('fs'),vm=require('vm'),assert=require('assert/strict');
function node(tag){return {tag,children:[],textContent:'',disabled:false,append(...children){this.children.push(...children)},addEventListener(){}};}
const source=fs.readFileSync('server/web/app.js','utf8');const entry=source.slice(source.indexOf('function entry('),source.indexOf('const libraryItems'));
for(const admin of [false,true]){const ctx=vm.createContext({document:{createElement:node},libraryGameName:()=>"Fixture game",currentUser:{admin,username:'reviewer'},console});vm.runInContext(entry,ctx);const result=vm.runInContext(`entry({id:'test',name:'Mod',version:'1',author:'uploader',review_status:'pending'},'mods')`,ctx);const buttons=result.children.at(-1).children;assert.equal(buttons.find(b=>b.textContent==='Download').disabled,true);assert.equal(buttons.some(b=>b.textContent==='Approve mod'),admin);}
console.log('Pending-download UI and administrator-only approval controls passed.');
