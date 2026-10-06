'use strict';
const codeInput=document.getElementById('connectioncode');
const statusText=document.getElementById('connectionstatus');
const submitButton=document.getElementById('verifyconnection');
const request=new URLSearchParams(location.search).get('request');
let busy=false,finished=false,lastCode='';
// Keep the request out of subsequent page URLs and the browser's address bar.
history.replaceState(null,'','/connect');
if(!request || !/^[a-f0-9]{64}$/i.test(request)) {
  finished=true;codeInput.disabled=true;submitButton.disabled=true;
  statusText.textContent='Open Settings in Canna and click Sign in & connect account. Canna will open this page with a new connection request.';
} else codeInput.focus();
async function verifyConnection() {
  const code=codeInput.value.replace(/[\s-]/g,'').toUpperCase();
  if(busy || finished || !/^[A-F0-9]{6}$/.test(code) || code===lastCode)return;
  busy=true;lastCode=code;submitButton.disabled=true;statusText.textContent='Checking connection code…';
  try {
    const response=await fetch('/api/v1/desktop/approve',{method:'POST',credentials:'same-origin',headers:{'Content-Type':'application/json'},body:JSON.stringify({request,code})});
    if(!response.ok) {
      const text=await response.text();let error;
      try {error=JSON.parse(text).error;}catch {error=null;}
      throw new Error(error || 'Verification failed. Start a new connection in Canna if this request has expired.');
    }
    finished=true;codeInput.disabled=true;statusText.textContent='Connection verified. Return to Canna; your account will connect automatically.';
  } catch(error) {statusText.textContent=error.message;if(/locked|expired|already used/i.test(error.message)){finished=true;codeInput.disabled=true;}}
  finally {busy=false;submitButton.disabled=finished;if(!finished && codeInput.value.replace(/[\s-]/g,'').toUpperCase()!==lastCode)void verifyConnection();}
}
codeInput.addEventListener('input',()=>{void verifyConnection();});
document.getElementById('connectionform').addEventListener('submit',event=>{event.preventDefault();lastCode='';void verifyConnection();});
