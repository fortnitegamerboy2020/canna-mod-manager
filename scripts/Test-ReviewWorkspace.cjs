const fs = require('fs'), vm = require('vm'), assert = require('assert');
const nodes = new Map(), all = [];
class N {
 constructor(tag) { this.tagName = tag.toUpperCase(); this.children = []; this.parentElement = null; this.dataset = {}; this.events = {}; this.attributes = {}; this.value = ''; this.hidden = false; this.disabled = false; this.open = false; this.scrollTop = this.scrollLeft = 0; this._text = ''; this._class = ''; all.push(this); this.classList = {add: (...classes) => { this._class = [...new Set([...this._class.split(' ').filter(Boolean), ...classes])].join(' '); }}; }
 set id(id) { this._id = id; nodes.set(id, this); } get id() { return this._id; }
 set textContent(value) { this.children = []; this._text = String(value); } get textContent() { return this._text + this.children.map(child => child.textContent).join(''); }
 set className(value) { this._class = value; } get className() { return this._class; }
 set innerHTML(_) { throw Error('Unsafe HTML insertion'); }
 append(...items) { for (let item of items) { if (typeof item === 'string') item = Object.assign(new N('text'), {_text: item}); if (item.tagName === 'FRAGMENT') { this.append(...item.children.slice()); continue; } if (item.parentElement) item.parentElement.children = item.parentElement.children.filter(child => child !== item); item.parentElement = this; this.children.push(item); } }
 replaceChildren(...items) { this.children = []; this._text = ''; this.append(...items); }
 setAttribute(key, value) { this.attributes[key] = String(value); }
 addEventListener(key, fn) { (this.events[key] ||= []).push(fn); }
 descendants() { return this.children.flatMap(child => [child, ...child.descendants()]); }
 matches(selector) { if (selector[0] === '.') return this.className.split(' ').includes(selector.slice(1)); if (selector[0] === '#') return this.id === selector.slice(1); return this.tagName === selector.toUpperCase(); }
 querySelectorAll(selector) { return this.descendants().filter(child => child.matches(selector)); } querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
 focus() { document.activeElement = this; } scrollIntoView() { this.scrolled = true; } showModal() { this.open = true; } close() { this.open = false; }
}
const document = {getElementById: id => nodes.get(id), createElement: tag => new N(tag), createTextNode: value => Object.assign(new N('text'), {_text: String(value)}), createDocumentFragment: () => new N('fragment'), activeElement: null};
const html = fs.readFileSync('server/web/review.html', 'utf8');
for (const match of html.matchAll(/<(\w+)[^>]*\bid="([^"]+)"[^>]*>/g)) { const n = new N(match[1]); n.id = match[2]; }
nodes.get('lineform').append(new N('button'));
for (const id of ['filetype', 'findingfilefilter']) nodes.get(id).value = 'all'; nodes.get('findingfilter').value = 'open';
const initial = {
 mod_name: '<img src=x onerror=alert(1)> Review fixture', status: 'complete', sha256: 'a'.repeat(64),
 inventory: [{name: 'assets/icon.png', size: 123}, {name: 'bin/native.dll', size: 4096}, {name: 'bin/managed.dll', size: 2048}, {name: 'source/Main.cs', size: 90000}, {name: 'README.md', size: 55}],
 files: [{name: 'source/Main.cs', kind: 'decompiled', origin: 'bin/managed.dll', language: 'csharp', decompiler: 'ilspycmd', text: Array.from({length: 1201}, (_, index) => index === 2 ? 'public class Main {' : index === 20 || index === 1100 ? 'private void Connect() { /* needle */ }' : index === 500 ? '<script>alert("never execute")</script>' : '// line ' + (index + 1)).join('\n')}, {name: 'README.md', kind: 'uploaded', text: 'Uploaded source description\nhttps://example.test'}],
 findings: [{id: 'network', title: 'Network call', severity: 'medium', rule: 'http', file: 'source/Main.cs', line: 21, evidence: 'needle', locations: [{file: 'source/Main.cs', line: 1101, evidence: 'needle'}], accepted: false}, {id: 'binary', title: 'Unsupported binary', severity: 'high', rule: 'native', file: 'bin/native.dll', evidence: 'Inventory evidence', accepted: true, reason: 'Known library'}],
 observations: [{title: 'Source URL', file: 'README.md', line: 2, evidence: 'https://example.test'}],
 engines: {decompiler: {status: 'limited', version: '1'}},
 decompilations: [{input: 'bin/native.dll', tool: 'objdump', status: 'unavailable', diagnostic: 'Unavailable fixture tool', generated_files: []}, {input: 'bin/managed.dll', tool: 'ilspycmd', status: 'complete', generated_files: ['source/Main.cs'], generated_count: 1, preview_count: 1}],
 review_overview: {coverage: {state: 'incomplete'}, note: 'Do not treat source reconstruction as a safety guarantee.', counts: {}, capabilities: [{rule: 'http', label: 'Networking', count: 2, unresolved_count: 1}], files: [{name: 'source/Main.cs', kind: 'decompiled', symbols: [{name: 'Main', kind: 'class', line: 3}], entry_points: [{name: 'Connect', kind: 'method', line: 21}]}, {name: 'bin/native.dll', preview_available: false, preview_reason: 'Native decompiler unavailable'}], suggestions: [{id: 'network', category: 'Networking', priority: 'high', title: 'Trace network inputs', why: '<svg onload=alert(1)> is untrusted text', checks: ['Check destinations'], locations: [{file: 'source/Main.cs', line: 1101}]}], dependencies: [{name: 'UntrustedLibrary', file: 'README.md'}]}
};
let data = structuredClone(initial), calls = [], timers = [], confirmed = [], checks = 0;
const context = {document, location: {pathname: '/review/fixture'}, window: {addEventListener() {}}, console, setTimeout: fn => { timers.push(fn); return timers.length; }, clearTimeout() {}, cannaConfirm: async message => { confirmed.push(message); return true; }, fetch: async (url, options) => { calls.push({url, ...options, body: options.body ? JSON.parse(options.body) : undefined}); return {ok: true, status: 200, json: async () => url.endsWith('/analysis') && options.method === 'GET' ? structuredClone(data) : {ok: true}}; }};
vm.createContext(context); vm.runInContext(fs.readFileSync('server/web/review.js', 'utf8'), context);
const run = code => vm.runInContext(code, context), check = fn => { fn(); checks++; }, event = async (id, type, value = {}) => { for (const fn of nodes.get(id).events[type] || []) await fn({preventDefault() {}, ...value}); };
(async () => {
 await run('load()');
 check(() => assert.equal(nodes.get('modname').textContent, initial.mod_name));
 check(() => assert.equal(run('entries.length'), 5));
 check(() => assert.equal(nodes.get('approve').disabled, true));
 check(() => assert(nodes.get('coverage').textContent.includes('incomplete')));
 check(() => assert(nodes.get('suggestedchecks').textContent.includes('<svg onload=alert(1)>')));
 check(() => assert(nodes.get('capabilities').textContent.includes('Declared metadata only')));
 check(() => assert.equal(run('sourceLink({file:"missing.cs",line:1}).disabled'), true));
 check(() => assert.equal(run('sourceLink({file:"bin/native.dll"}).disabled'), false));
 run('codeFile("bin/native.dll")');
 check(() => assert(nodes.get('code').textContent.includes('Native decompiler unavailable')));
 check(() => assert(nodes.get('code').textContent.includes('Unavailable fixture tool')));
 check(() => assert.equal(nodes.get('infilesearch').disabled, true));
 run('codeFile("source/Main.cs",1101,true)');
 check(() => assert.equal(run('selectedLine'), 1101));
 check(() => assert(nodes.get('code').children.length <= 400));
 check(() => assert(nodes.get('line-1101').scrolled));
 check(() => assert(nodes.get('filekind').textContent.includes('Reconstructed source')));
 check(() => assert(nodes.get('filemeta').textContent.includes('bin/managed.dll')));
 check(() => assert(nodes.get('fileevidence').textContent.includes('Network call')));
 check(() => assert(nodes.get('symboloutline').textContent.includes('Connect')));
 nodes.get('code').scrollTop = 245; nodes.get('code').scrollLeft = 33;
 const oldCode = nodes.get('code').children[0], oldOutline = nodes.get('symboloutline').children[0], oldEvidence = nodes.get('fileevidence').children[0];
 await run('load()');
 check(() => assert.equal(run('selectedLine'), 1101));
 check(() => assert.strictEqual(nodes.get('code').children[0], oldCode));
 check(() => assert.strictEqual(nodes.get('symboloutline').children[0], oldOutline));
 check(() => assert.strictEqual(nodes.get('fileevidence').children[0], oldEvidence));
 check(() => assert.equal(nodes.get('code').scrollTop, 245));
 nodes.get('infilesearch').value = 'needle'; await event('infilesearch', 'input');
 check(() => assert.equal(run('matches.length'), 2));
 check(() => assert.equal(run('selectedLine'), 21));
 await event('searchnext', 'click'); check(() => assert.equal(run('selectedLine'), 1101));
 await event('searchnext', 'click'); check(() => assert.equal(run('selectedLine'), 21));
 await event('searchprevious', 'click'); check(() => assert.equal(run('selectedLine'), 1101));
 await run('load()'); check(() => assert.equal(run('matchIndex'), 1));
 check(() => assert.equal(nodes.get('infilesearch').value, 'needle'));
 nodes.get('gotoline').value = '501'; await event('lineform', 'submit');
 check(() => assert.equal(run('selectedLine'), 501));
 check(() => assert(nodes.get('line-501').textContent.includes('<script>')));
 check(() => assert.equal(nodes.get('line-501').querySelectorAll('script').length, 0));
 await event('infilesearch', 'keydown', {key: 'Escape'}); check(() => assert.equal(nodes.get('infilesearch').value, ''));
 nodes.get('filesearch').value = 'native'; await event('filesearch', 'input');
 check(() => assert(nodes.get('filecount').textContent.startsWith('1 of 5')));
 check(() => assert(nodes.get('filetree').textContent.includes('native.dll')));
 nodes.get('filesearch').value = ''; nodes.get('codesearch').value = 'needle'; run('filterFiles()');
 check(() => assert(nodes.get('filecount').textContent.startsWith('1 of 5')));
 check(() => assert(!nodes.get('filetree').textContent.includes('native.dll')));
 await event('clearfiles', 'click'); nodes.get('filetype').value = 'unavailable'; await event('filetype', 'change');
 check(() => assert(nodes.get('filecount').textContent.startsWith('3 of 5')));
 await event('clearfiles', 'click'); await event('collapsefiles', 'click'); const dirs = nodes.get('filetree').querySelectorAll('details');
 check(() => assert(dirs.every(dir => !dir.open)));
 await run('load()'); check(() => assert(nodes.get('filetree').querySelectorAll('details').every(dir => !dir.open)));
 await event('filefindingsbutton', 'click');
 check(() => assert.equal(run('activeTab'), 'findings'));
 check(() => assert.equal(nodes.get('findingfilefilter').value, 'selected'));
 check(() => assert(!nodes.get('findings').textContent.includes('Unsupported binary')));
 nodes.get('findingfilefilter').value = 'all'; nodes.get('findingfilter').value = 'accepted'; run('renderFindings()');
 check(() => assert(nodes.get('findings').textContent.includes('Unsupported binary')));
 run('switchTab("overview")'); await event('tab-overview', 'keydown', {key: 'ArrowRight'});
 check(() => assert.equal(run('activeTab'), 'code'));
 check(() => assert.equal(document.activeElement, nodes.get('tab-code')));
 await event('tab-code', 'keydown', {key: 'End'}); check(() => assert.equal(run('activeTab'), 'findings'));
 nodes.get('findingfilter').value = 'open'; run('renderFindings()'); const acceptButton = nodes.get('findings').querySelector('.decisionbutton'); await acceptButton.events.click[0]();
 check(() => assert.equal(run('decisionTarget.sha256'), initial.sha256));
 data.sha256 = 'b'.repeat(64); await run('load()');
 check(() => assert.equal(nodes.get('decisionrecord').disabled, true));
 const decisionsBefore = calls.filter(call => /analysis\/network$/.test(call.url)).length; await event('decisionform', 'submit');
 check(() => assert.equal(calls.filter(call => /analysis\/network$/.test(call.url)).length, decisionsBefore));
 await event('decisioncancel', 'click'); const reopened = nodes.get('findings').querySelector('.decisionbutton'); await reopened.events.click[0](); nodes.get('decisionreason').value = 'Verified bounded destination'; await event('decisionform', 'submit');
 const decision = calls.findLast(call => /analysis\/network$/.test(call.url));
 check(() => assert.equal(decision.body.sha256, 'b'.repeat(64)));
 check(() => assert.equal(decision.body.accepted, true));
 check(() => assert.equal(decision.credentials, 'same-origin'));
 data.findings.forEach(item => item.accepted = true); await run('load()'); check(() => assert.equal(nodes.get('approve').disabled, false));
 data.status = 'rejected'; await run('load()'); check(() => assert.equal(nodes.get('approve').disabled, true));
 check(() => assert.equal(nodes.get('fileevidence').querySelectorAll('.decisionbutton').length, 0));
 data = {status: 'complete', sha256: 'c'.repeat(64), mod_name: 'Legacy', files: [{name: 'Legacy.cs', kind: 'decompiled', text: 'public class Legacy {\n public void Awake() {\n }\n}'}], findings: [], observations: []}; await run('load()');
 check(() => assert(nodes.get('suggestedchecks').textContent.includes('Inspect startup behavior')));
 check(() => assert(nodes.get('symboloutline').textContent.includes('Awake')));
 check(() => assert(nodes.get('coverage').textContent.includes('Coverage not established')));
 data = {status: 'queued', files: [], findings: [], observations: []}; await run('load()');
 check(() => assert(nodes.get('code').textContent.includes('No retained source')));
 check(() => assert.equal(nodes.get('approve').disabled, true));
 check(() => assert.equal(nodes.get('filefindingsbutton').disabled, true));
 data = {status: 'complete', sha256: 'd'.repeat(64), files: [], inventory: Array.from({length: 650}, (_, index) => ({name: 'resources/file-' + String(index).padStart(3, '0') + '.bin', size: index})), findings: [], observations: []}; await run('load()');
 check(() => assert.equal(nodes.get('filetree').querySelectorAll('.filelink').length, 500));
 check(() => assert.equal(nodes.get('morefiles').hidden, false));
 await event('morefiles', 'click'); check(() => assert.equal(nodes.get('filetree').querySelectorAll('.filelink').length, 650));
 check(() => assert.equal(nodes.get('morefiles').hidden, true));
 await run('load()'); check(() => assert.equal(nodes.get('filetree').querySelectorAll('.filelink').length, 650));
 data = structuredClone(initial); data.files[0].text = 'x'.repeat(10000) + '\n'.repeat(1000) + 'needle'; await run('load()'); run('codeFile("source/Main.cs",1,true)');
 check(() => assert(nodes.get('codewindow').textContent.includes('long lines clipped')));
 check(() => assert(nodes.get('code').textContent.length < 20000));
 nodes.get('infilesearch').value = 'needle'; await event('infilesearch', 'input'); check(() => assert.equal(run('selectedLine'), 1001));
 run('codeFile("source/Main.cs",1500,true)'); check(() => assert(nodes.get('codewindow').textContent.includes('Requested line 1500 is outside the retained preview')));
 await run('load()'); check(() => assert(nodes.get('codewindow').textContent.includes('Requested line 1500')));
 run('codeFile("source/Main.cs",1,true)'); check(() => assert(!nodes.get('codewindow').textContent.includes('Requested line')));
 data = structuredClone(initial); data.findings[0].context = {explanation:'Caller-controlled output directory'}; await run('load()'); run('codeFile("source/Main.cs")');
 check(() => assert(nodes.get('fileevidence').textContent.includes('Context: Caller-controlled output directory')));
 check(() => assert(!nodes.get('fileevidence').textContent.includes('[object Object]')));
 data = {status:'complete',sha256:'e'.repeat(64),findings:[null,true,{accepted:true}],files:[null],inventory:[null],observations:[false],review_overview:{files:[null],suggestions:[null],capabilities:[true],dependencies:[null],decompilations:[null]}}; await run('load()');
 check(() => assert(nodes.get('reviewstatus').textContent.includes('Malformed report data')));
 check(() => assert.equal(nodes.get('approve').disabled,true));
 check(() => assert(nodes.get('findings').querySelectorAll('.decisionbutton').every(button=>button.disabled)));
 data = {status:'complete',sha256:'f'.repeat(64),findings:'invalid',files:[],inventory:[],observations:[]}; await run('load()');
 check(() => assert.equal(nodes.get('approve').disabled,true));
 check(() => assert(nodes.get('reviewstatus').textContent.includes('findings')));
 data = {status:'complete',files:[],findings:[],observations:[]}; await run('load()'); check(() => assert.equal(nodes.get('approve').disabled,true));
 const submittedBefore=calls.filter(call=>call.method==='POST'&&call.url.endsWith('/analysis')).length;
 data={...structuredClone(initial),status:'pending'};await run('load()');await run('load()');await run('load()');
 check(()=>assert.equal(calls.filter(call=>call.method==='POST'&&call.url.endsWith('/analysis')).length,submittedBefore,'Pending polling must not consume retry limits'));
 check(()=>assert.equal(nodes.get('approve').disabled,true));
 console.log(JSON.stringify({pass: true, checks, scope: 'Review workspace DOM integration: unsafe content as text, exact archive paths and binary limitations, filters, bounded long source, line/search/outline navigation, polling state and DOM preservation, file-focused evidence, accessible tabs, exact-hash acceptance and rejected/legacy/empty report behavior'}));
})().catch(error => { console.error(error); process.exitCode = 1; });
