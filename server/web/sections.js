'use strict';
let sectionMoves={}, groupDraft=[], groupOriginal=[];
let sectionDraft=[], sectionOriginal=[], reviewToken='', sectionDraftRevision=0;
async function loadSectionEditor() {
  await loadSections(); sectionDraftRevision=sectionRevision;
  sectionOriginal=structuredClone(forumCategories); sectionDraft=structuredClone(forumCategories);
  groupOriginal=structuredClone(forumGroups);groupDraft=structuredClone(forumGroups);
  reviewToken='';sectionMoves={}; renderSectionEditor();
}
function renderSectionEditor() {
  $('groupdraft').replaceChildren(...groupDraft.map((group,index)=>{
    const card=document.createElement('div');card.className='sectionedit';
    const label=document.createElement('label');label.textContent='Forum group title';
    const input=document.createElement('input');input.value=group.name;input.maxLength=80;input.setAttribute('aria-label',`Forum group title ${index+1}`);
    input.addEventListener('input',()=>{group.name=input.value;reviewToken='';});label.append(input);
    const options=document.createElement('div');options.className='row';
    const move=delta=>{[groupDraft[index],groupDraft[index+delta]]=[groupDraft[index+delta],groupDraft[index]];reviewToken='';renderSectionEditor();};
    const up=button('Move group up',()=>move(-1)),down=button('Move group down',()=>move(1));up.disabled=index===0;down.disabled=index===groupDraft.length-1;
    const destination=document.createElement('select');destination.setAttribute('aria-label',`Move sections from group ${index+1} into`);
    destination.append(new Option('Move sections into…',''),...groupDraft.filter(g=>g.id!==group.id).map(g=>new Option(g.name||'Unnamed group',g.id)));
    const remove=button('Delete group from draft',()=>{
      const sections=sectionDraft.filter(s=>(s.group||'unity')===group.id);
      if(sections.length && !destination.value)throw new Error('Choose another group for these sections first.');
      for(const section of sections)section.group=destination.value;
      groupDraft.splice(index,1);reviewToken='';renderSectionEditor();
    });remove.disabled=groupDraft.length===1;
    const add=button('+ Add discussion section here',()=>{sectionDraft.push({id:crypto.randomUUID(),group:group.id,name:'',description:'',active:true,vip_only:false});reviewToken='';renderSectionEditor();});add.disabled=sectionDraft.length>=32;
    options.append(up,down,add,destination,remove);card.append(label,options);return card;
  }));
  $('addgroup').disabled=groupDraft.length>=16;
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
    const groupLabel=document.createElement('label');groupLabel.textContent='Forum group';
    const groupSelect=document.createElement('select');groupSelect.setAttribute('aria-label',`Forum group for section ${index+1}`);
    groupSelect.append(...groupDraft.map(g=>new Option(g.name||'Unnamed group',g.id)));groupSelect.value=section.group||'unity';
    groupSelect.addEventListener('change',()=>{section.group=groupSelect.value;reviewToken='';});groupLabel.append(groupSelect);fields.append(groupLabel);
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
  sectionDraft.push({id:crypto.randomUUID(),group:groupDraft[0].id,name:'',description:'',active:true,vip_only:false});reviewToken='';renderSectionEditor();
});
$('addgroup').addEventListener('click',()=>{groupDraft.push({id:crypto.randomUUID(),name:''});reviewToken='';renderSectionEditor();});
$('resetsections').addEventListener('click',()=>action(loadSectionEditor));
$('reviewsections').addEventListener('click',()=>action(async()=>{
  if(JSON.stringify(sectionDraft)===JSON.stringify(sectionOriginal) && JSON.stringify(groupDraft)===JSON.stringify(groupOriginal)) throw new Error('Make a change before reviewing.');
  const result=await json('admin/sections/review',{revision:sectionDraftRevision,sections:sectionDraft,groups:groupDraft,moves:sectionMoves});
  reviewToken=result.token;
  $('sectiondiff').replaceChildren(...result.layout.sections.flatMap((s,index)=>{
    const previous=sectionOriginal[index];
    if(previous && JSON.stringify(previous)===JSON.stringify(s)) return [];
    const old=sectionOriginal.find(o=>o.id===s.id),row=document.createElement('div');row.className='reviewentry';
    const title=document.createElement('strong');title.textContent=`${index+1}. ${s.name}${old ? '' : ' · New section'}`;
    const description=document.createElement('p');description.textContent=s.description || 'No description';
    const status=document.createElement('small');status.textContent=`Group: ${groupDraft.find(g=>g.id===(s.group||'unity'))?.name || 'Unknown'} · ${s.active ? 'Open' : 'Closed'} · ${s.vip_only ? 'VIP+ can post' : 'All members can post'}`;
    row.append(title,description,status);
    if(old) { const before=document.createElement('p');before.className='sidehint';before.textContent=`Previously: ${sectionOriginal.indexOf(old)+1}. ${old.name} · ${old.description || 'No description'} · ${old.active ? 'Open' : 'Closed'} · ${old.vip_only ? 'VIP+ posting' : 'All members'}`;row.append(before); }
    return [row];
  }));
  for(const removed of sectionOriginal.filter(s=>!sectionDraft.some(v=>v.id===s.id))) {
    const row=document.createElement('p');const target=sectionDraft.find(s=>s.id===sectionMoves[removed.id]);row.textContent=`Delete ${removed.name}${target?` · Move discussions to ${target.name}`:' · Empty category'}`;$('sectiondiff').append(row);
  }
  const groupSummary=document.createElement('div');groupSummary.className='reviewentry';
  const title=document.createElement('strong');title.textContent='Forum groups (in display order)';groupSummary.append(title);
  for(const group of groupDraft){const row=document.createElement('p');const old=groupOriginal.find(g=>g.id===group.id);row.textContent=group.name+(old ? (old.name!==group.name?` · Previously ${old.name}`:''):' · New group');groupSummary.append(row);}
  for(const old of groupOriginal.filter(g=>!groupDraft.some(n=>n.id===g.id))){const row=document.createElement('p');row.textContent=`Delete group ${old.name} · Sections reassigned`;groupSummary.append(row);}
  $('sectiondiff').prepend(groupSummary);
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
