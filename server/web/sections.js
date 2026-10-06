'use strict';
let sectionMoves={};
let sectionDraft=[], sectionOriginal=[], reviewToken='', sectionDraftRevision=0;
async function loadSectionEditor() {
  await loadSections(); sectionDraftRevision=sectionRevision;
  sectionOriginal=structuredClone(forumCategories); sectionDraft=structuredClone(forumCategories);
  reviewToken='';sectionMoves={}; renderSectionEditor();
}
function renderSectionEditor() {
  $('sectiondraft').replaceChildren(...sectionDraft.map((section,index)=>{
    const card=document.createElement('div'); card.className='sectionedit';
    const fields=document.createElement('div'); fields.className='sectionfields';
    const field=(caption,key,max)=>{
      const label=document.createElement('label'); label.textContent=caption;
      const input=document.createElement('input'); input.value=section[key]; input.maxLength=max;
      input.setAttribute('aria-label',`${caption} for section ${index+1}`);
      input.addEventListener('input',()=>{section[key]=input.value;reviewToken='';});
      label.append(input); fields.append(label);
    };
    field('Section name','name',80); field('Description','description',300);
    const options=document.createElement('div'); options.className='row';
    for(const [key,caption] of [['active','Allow new discussions'],['vip_only','VIP+ posting only']]) {
      const label=document.createElement('label'); label.className='check';
      const input=document.createElement('input'); input.type='checkbox'; input.checked=section[key];
      input.setAttribute('aria-label',`${caption} for section ${index+1}`);
      input.addEventListener('change',()=>{section[key]=input.checked;reviewToken='';});
      label.append(input,document.createTextNode(caption)); options.append(label);
    }
    const move=(delta)=>{const other=index+delta;[sectionDraft[index],sectionDraft[other]]=[sectionDraft[other],sectionDraft[index]];reviewToken='';renderSectionEditor();};
    const up=button('Move up',()=>move(-1)),down=button('Move down',()=>move(1));up.disabled=index===0;down.disabled=index===sectionDraft.length-1;options.append(up,down);
    options.append(button('Delete category from draft',()=>{sectionDraft.splice(index,1);reviewToken='';renderSectionEditor();}));
    card.append(fields,options);return card;
  }));
  for(const removed of sectionOriginal.filter(s=>!sectionDraft.some(v=>v.id===s.id))) {
    const row=document.createElement('div');row.className='reviewentry';const label=document.createElement('label');label.textContent=`Delete ${removed.name} · Move its discussions into:`;
    const select=document.createElement('select');select.setAttribute('aria-label',`Destination for ${removed.name}`);
    select.append(new Option('Choose destination (required if it contains discussions)',''));
    for(const target of sectionDraft)select.append(new Option(target.name||'Unnamed category',target.id));
    if(!sectionDraft.some(s=>s.id===sectionMoves[removed.id]))delete sectionMoves[removed.id];select.value=sectionMoves[removed.id]||'';
    select.addEventListener('change',()=>{if(select.value)sectionMoves[removed.id]=select.value;else delete sectionMoves[removed.id];reviewToken='';});
    row.append(label,select,button('Undo deletion',()=>{sectionDraft.push(structuredClone(removed));delete sectionMoves[removed.id];reviewToken='';renderSectionEditor();}));$('sectiondraft').append(row);
  }
  $('addsection').disabled=sectionDraft.length>=32;
}
$('addsection').addEventListener('click',()=>{
  sectionDraft.push({id:crypto.randomUUID(),name:'',description:'',active:true,vip_only:false});reviewToken='';renderSectionEditor();
});
$('resetsections').addEventListener('click',()=>action(loadSectionEditor));
$('reviewsections').addEventListener('click',()=>action(async()=>{
  if(JSON.stringify(sectionDraft)===JSON.stringify(sectionOriginal)) throw new Error('Make a change before reviewing.');
  const result=await json('admin/sections/review',{revision:sectionDraftRevision,sections:sectionDraft,moves:sectionMoves});
  reviewToken=result.token;
  $('sectiondiff').replaceChildren(...result.layout.sections.flatMap((s,index)=>{
    const previous=sectionOriginal[index];
    if(previous && JSON.stringify(previous)===JSON.stringify(s)) return [];
    const old=sectionOriginal.find(o=>o.id===s.id),row=document.createElement('div');row.className='reviewentry';
    const title=document.createElement('strong');title.textContent=`${index+1}. ${s.name}${old ? '' : ' · New section'}`;
    const description=document.createElement('p');description.textContent=s.description || 'No description';
    const status=document.createElement('small');status.textContent=`${s.active ? 'Open' : 'Closed'} · ${s.vip_only ? 'VIP+ can post' : 'All members can post'}`;
    row.append(title,description,status);
    if(old) { const before=document.createElement('p');before.className='sidehint';before.textContent=`Previously: ${sectionOriginal.indexOf(old)+1}. ${old.name} · ${old.description || 'No description'} · ${old.active ? 'Open' : 'Closed'} · ${old.vip_only ? 'VIP+ posting' : 'All members'}`;row.append(before); }
    return [row];
  }));
  for(const removed of sectionOriginal.filter(s=>!sectionDraft.some(v=>v.id===s.id))) {
    const row=document.createElement('p');const target=sectionDraft.find(s=>s.id===sectionMoves[removed.id]);row.textContent=`Delete ${removed.name}${target?` · Move discussions to ${target.name}`:' · Empty category'}`;$('sectiondiff').append(row);
  }
  $('sectionreview').showModal();
}));
$('cancelsections').addEventListener('click',()=>{reviewToken='';$('sectionreview').close();});
$('sectionreview').addEventListener('cancel',()=>{reviewToken='';});
$('applysections').addEventListener('click',()=>action(async()=>{
  if(!reviewToken) throw new Error('Review changes again before applying.');
  const btn=$('applysections');btn.disabled=true;
  try {await json('admin/sections/apply',{token:reviewToken});reviewToken='';$('sectionreview').close();await loadSectionEditor();await loadTopics();message('Forum sections updated.');}
  finally {btn.disabled=false;}
}));
