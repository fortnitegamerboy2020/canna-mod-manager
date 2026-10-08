'use strict';
(() => {
 const controls = new WeakMap();
 const menu = document.createElement('dialog');menu.className='filter-menu';menu.setAttribute('aria-label','Search filter options');
 const search = document.createElement('input');search.type='search';search.placeholder='Search options…';search.maxLength=80;search.setAttribute('aria-label','Search filter options');
 const list=document.createElement('div');list.className='filter-menu-options';list.setAttribute('role','listbox');
 menu.append(search,list);document.body.append(menu);
 let active=null,anchor=null;
 const options=el=>[...el.options].filter(o=>!o.disabled).sort((a,b)=>a.text.localeCompare(b.text,undefined,{sensitivity:'base',numeric:true}));
 function refresh(el){const button=controls.get(el);if(!button)return;const label=el.getAttribute('aria-label')||el.labels?.[0]?.textContent?.trim().split('\n')[0]||'Filter';button.textContent=(el.selectedOptions[0]?.text||'Choose an option')+' ▾';button.setAttribute('aria-label','Choose '+label);button.disabled=el.disabled;button.hidden=el.hidden;}
 function close(){if(menu.open)menu.close();if(anchor)anchor.setAttribute('aria-expanded','false');active=anchor=null;}
 function render(){if(!active)return;const q=search.value.trim().toLocaleLowerCase();const matches=options(active).filter(o=>o.text.toLocaleLowerCase().includes(q));list.replaceChildren();for(const o of matches){const button=document.createElement('button');button.type='button';button.setAttribute('role','option');button.setAttribute('aria-selected',String(o.value===active.value));button.textContent=o.text;button.addEventListener('click',()=>{const el=active,trigger=anchor;el.value=o.value;close();refresh(el);el.dispatchEvent(new Event('change',{bubbles:true}));trigger.focus();});list.append(button);}if(!matches.length){const note=document.createElement('p');note.textContent='No matching options';list.append(note);}}
 function position(){if(!anchor)return;const r=anchor.getBoundingClientRect(),width=Math.min(340,innerWidth-24);menu.style.width=width+'px';menu.style.left=Math.max(12,Math.min(r.left,innerWidth-width-12))+'px';const h=Math.min(menu.scrollHeight,innerHeight-24);menu.style.top=Math.max(12,Math.min(r.bottom+6,innerHeight-h-12))+'px';}
 function enhance(el){if(controls.has(el)||el.multiple)return;const button=document.createElement('button');button.type='button';button.className='filter-trigger';button.setAttribute('aria-haspopup','listbox');button.setAttribute('aria-expanded','false');el.classList.add('filter-select-native');el.tabIndex=-1;el.after(button);controls.set(el,button);refresh(el);button.addEventListener('click',()=>{if(active===el){close();return;}close();active=el;anchor=button;search.value='';render();menu.show();position();button.setAttribute('aria-expanded','true');search.focus();});el.addEventListener('change',()=>refresh(el));new MutationObserver(()=>{refresh(el);if(active===el){render();position();}}).observe(el,{childList:true,subtree:true,attributes:true,attributeFilter:['disabled','hidden','selected']});}
 search.addEventListener('input',()=>{render();position();});
 menu.addEventListener('keydown',event=>{if(event.key==='Escape'){event.preventDefault();const trigger=anchor;close();trigger?.focus();}else if(event.key==='ArrowDown'||event.key==='ArrowUp'){event.preventDefault();const buttons=[...list.querySelectorAll('button')];if(!buttons.length)return;let i=buttons.indexOf(document.activeElement);i=event.key==='ArrowDown'?Math.min(buttons.length-1,i+1):i<=0?buttons.length-1:i-1;buttons[i].focus();}else if(event.key==='Enter'&&document.activeElement===search){event.preventDefault();list.querySelector('button')?.click();}});
 document.addEventListener('pointerdown',event=>{if(menu.open&&!menu.contains(event.target)&&!anchor?.contains(event.target))close();},true);
 window.addEventListener('resize',position);document.addEventListener('scroll',position,true);
 const scan=()=>document.querySelectorAll('select').forEach(enhance);
 document.addEventListener('DOMContentLoaded',scan,{once:true});
 new MutationObserver(scan).observe(document.body,{childList:true,subtree:true});
 window.refreshFilterMenus=()=>{document.querySelectorAll('select').forEach(el=>{enhance(el);refresh(el);});};
})();
