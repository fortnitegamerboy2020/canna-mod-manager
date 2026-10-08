'use strict';
const $ = id => document.getElementById(id);
sessionStorage.removeItem('canna-session');
let challenge = '', codePurpose = 'verify';
let recoveryPurpose = 'reset';
const recoveryCooldowns = new Map();
const recoveryRequests = new Set();
let recoveryGeneration = 0;
let recoveryTimer;

const message = text => { $('message').textContent = text; };
function acceptInvitationLink(){
 const query=new URLSearchParams(location.search||''),fragment=new URLSearchParams((location.hash||'').slice(1));
 const values=[...query.getAll('invite'),...fragment.getAll('invite')];if(!values.length)return;
 // New links keep their code in the fragment, outside HTTP requests/referrers.
 // Remove it from this history entry after reading; opening a link never redeems it.
 query.delete('invite');fragment.delete('invite');
 const search=query.toString(),hash=fragment.toString();
 history.replaceState(history.state,'',(location.pathname||'/')+(search?'?'+search:'')+(hash?'#'+hash:''));
 if(values.length!==1||!/^[A-Za-z0-9_-]{1,64}$/.test(values[0])){message('This invitation link is invalid. Ask for a new link or paste your code.');return;}
 $('invite').value=values[0];$('password').autocomplete='new-password';$('authheading').textContent='You’re invited';$('authdescription').textContent='Create your account below. Your invitation is already filled in.';
 message('Invitation filled in. Choose a username, email and password, then select Create account.');
}
acceptInvitationLink();
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
function recoveryCooldown() {
  clearTimeout(recoveryTimer);
  if ($('recoverypanel').hidden) return;
  const remaining = Math.max(0, Math.ceil(((recoveryCooldowns.get(recoveryPurpose) || 0) - Date.now()) / 1000));
  const sending = recoveryRequests.has(recoveryPurpose);
  $('sendrecovery').disabled = remaining > 0 || sending;
  $('sendrecovery').textContent = sending ? 'Sending…' : remaining ? `Send again in ${remaining}s` : recoveryPurpose === 'reset' ? 'Send reset code' : 'Email my username';
  if (remaining) recoveryTimer = setTimeout(recoveryCooldown, 1000);
}
function openRecovery(purpose) {
  recoveryGeneration++;
  recoveryPurpose = purpose; message('');
  $('auth').hidden = $('authhelp').hidden = true;
  $('codeform').hidden = true; $('codeform').reset();
  challenge = '';
  $('recoverypanel').hidden = false;
  $('recoverytitle').textContent = purpose === 'reset' ? 'Reset your password' : 'Find your username';
  $('recoverydescription').textContent = purpose === 'reset' ? 'Enter the email you verified for your account. We’ll email a code to choose a new password.' : 'Enter your verified account email. We’ll send your username to that address.';
  $('recoveryemail').value = $('recoveryemail').value || $('email').value;
  recoveryCooldown(); $('recoveryemail').focus();
}
function closeRecovery() {
  recoveryGeneration++;
  clearTimeout(recoveryTimer); challenge = '';
  $('recoverypanel').hidden = $('codeform').hidden = true;
  $('codeform').reset(); $('auth').hidden = $('authhelp').hidden = false;
  $('password').value = ''; $('username').focus();
}
$('forgot').addEventListener('click', () => openRecovery('reset'));
$('forgotusername').addEventListener('click', () => openRecovery('username'));
$('backtosignin').addEventListener('click', () => { closeRecovery(); message(''); });
$('recoveryform').addEventListener('submit', event => { event.preventDefault(); action(async () => {
  if (!$('recoveryform').reportValidity() || $('sendrecovery').disabled) return;
  const purpose = recoveryPurpose;
  const generation = recoveryGeneration;
  recoveryRequests.add(purpose);
  $('sendrecovery').disabled = true;
  try {
    const data = await json(purpose === 'reset' ? 'forgot-password' : 'forgot-username', {email:$('recoveryemail').value.trim()});
    recoveryCooldowns.set(purpose, Date.now() + 60000);
    if (purpose === recoveryPurpose && generation === recoveryGeneration) {
      if (purpose === 'reset') showCode(data.challenge, 'reset');
      message(data.message);
    }
  } catch (error) {
    if (generation === recoveryGeneration) throw error;
  } finally { recoveryRequests.delete(purpose); recoveryCooldown(); }
}); });
$('resend').addEventListener('click', () => action(async () => {
  const data = await json('resend-verification', {email:$('email').value, username:$('username').value});
  showCode(data.challenge, 'verify'); message(data.message);
  $('resend').disabled = true; setTimeout(() => { $('resend').disabled = false; }, 60000);
}));
$('codeform').addEventListener('submit', event => { event.preventDefault(); action(async () => {
  await json(codePurpose === 'reset' ? 'reset-password' : codePurpose === 'login' ? 'login/verify' : 'verify-email', {challenge, code:$('code').value, password:$('newpassword').value,trust_device:$('trustdevice').checked});
  $('codeform').reset(); $('codeform').hidden = true;
  if (codePurpose === 'reset') { closeRecovery(); message('Password reset. Sign in with your new password.'); }
  else location.reload();
}); });

if(location.pathname.startsWith('/packs/')) message('Sign in to download this shared modpack.');
