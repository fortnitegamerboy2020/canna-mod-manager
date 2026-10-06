'use strict';
let confirmationQueue=Promise.resolve();
function cannaConfirm(message,options={}) {
  const result=confirmationQueue.then(()=>new Promise(resolve=>{
    const previous=document.activeElement,dialog=document.createElement('dialog');dialog.className='cannaconfirm';
    const title=document.createElement('h2');title.id='confirmation-title';title.textContent=options.title||'Confirm change';dialog.setAttribute('aria-labelledby',title.id);
    const body=document.createElement('p');body.textContent=message;
    const controls=document.createElement('div');controls.className='row';
    const cancel=document.createElement('button');cancel.textContent='Cancel';cancel.autofocus=true;
    const approve=document.createElement('button');approve.textContent=options.label||'Confirm';approve.className='primary';
    let completed=false;function finish(value){if(completed)return;completed=true;dialog.close();dialog.remove();previous?.focus();resolve(value);}
    cancel.addEventListener('click',()=>finish(false));approve.addEventListener('click',()=>finish(true));
    dialog.addEventListener('cancel',event=>{event.preventDefault();finish(false);});dialog.addEventListener('close',()=>finish(false));
    controls.append(cancel,approve);dialog.append(title,body,controls);document.body.append(dialog);dialog.showModal();cancel.focus();
  }));
  confirmationQueue=result.catch(()=>false);return result;
}
