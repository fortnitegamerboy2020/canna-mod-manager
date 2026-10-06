'use strict';
const $ = id => document.getElementById(id);
sessionStorage.removeItem('canna-session');
let challenge = '', codePurpose = 'verify';

const message = text => { $('message').textContent = text; };
async function api(path, options = {}) {
  const headers = new Headers(options.headers || {});
  const response = await fetch(`/api/v1/${path}`, {...options, headers});
  if (!response.ok) {
    const error = await response.json().catch(() => ({}));
    throw new Error(error.error || `Request failed (${response.status})`);
  }
  return response;
}
async function json(path, data) {
  return (await api(path, {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(data)})).json();
}
async function action(callback) {
  message('');
  try { await callback(); } catch (error) { message(error.message); }
}
async function authenticate(register) {
  if (register && !$('email').value) throw new Error('Enter your email to verify your account.');
  const data = await json(register ? 'register' : 'login', {username:$('username').value, email:$('email').value, password:$('password').value, invite:$('invite').value});
  $('password').value = ''; $('invite').value = '';
  if (data.verification_required) {
    showCode(data.challenge, 'verify');
    message('Check your email for a verification code. It expires in ten minutes.');
    return;
  }
  if (data.two_factor_required) {
    showCode(data.challenge, 'login');
    message('Check your email for your sign-in code. It expires in ten minutes.');
    return;
  }
  location.reload();
}
function showCode(id, purpose) {
  challenge = id; codePurpose = purpose;
  $('codeform').hidden = false; $('code').value = '';
  $('newpassword').hidden = $('newpasswordlabel').hidden = purpose !== 'reset';
  $('newpassword').required = purpose === 'reset';
  $('trustlabel').hidden = purpose !== 'login'; $('trustdevice').checked = false;
  $('code').focus();
}
$('auth').addEventListener('submit', event => { event.preventDefault(); action(() => authenticate(false)); });
$('register').addEventListener('click', () => action(async () => { if ($('auth').reportValidity()) await authenticate(true); }));
$('forgot').addEventListener('click', () => action(async () => {
  const data = await json('forgot-password', {email:$('email').value});
  showCode(data.challenge, 'reset'); message(data.message);
  $('forgot').disabled = true; setTimeout(() => { $('forgot').disabled = false; }, 60000);
}));
$('resend').addEventListener('click', () => action(async () => {
  const data = await json('resend-verification', {email:$('email').value, username:$('username').value});
  showCode(data.challenge, 'verify'); message(data.message);
  $('resend').disabled = true; setTimeout(() => { $('resend').disabled = false; }, 60000);
}));
$('codeform').addEventListener('submit', event => { event.preventDefault(); action(async () => {
  await json(codePurpose === 'reset' ? 'reset-password' : codePurpose === 'login' ? 'login/verify' : 'verify-email', {challenge, code:$('code').value, password:$('newpassword').value,trust_device:$('trustdevice').checked});
  $('codeform').reset(); $('codeform').hidden = true;
  if (codePurpose === 'reset') message('Password reset. Sign in with your new password.');
  else location.reload();
}); });

if(location.pathname.startsWith('/packs/')) message('Sign in to download this shared modpack.');
