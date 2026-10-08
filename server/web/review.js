'use strict';
const $ = id => document.getElementById(id), modId = location.pathname.split('/').at(-1);
const TREE_BATCH = 500, CODE_LINES = 400, CODE_CHARACTERS = 400000, LINE_CHARACTERS = 4000, MATCH_LIMIT = 5000;
let report = {files: [], findings: [], observations: []}, entries = [], entryMap = new Map(), selectedFile = '', selectedLine = 1;
let poll, decisionTarget, activeTab = 'overview', treeLimit = TREE_BATCH, treeSignature = '', sourceSignature = '', renderedCodeSignature = '', renderedFileEvidenceSignature = '', renderedOutlineSignature = '', renderedOverviewSignature = '', renderedFindingsSignature = '', loadGeneration = 0;
let sourceLines = [], sourceText = '', sourceRevision = 0, codeStart = 0, codeEnd = 0, matches = [], matchIndex = -1, matchesLimited = false, fileSearchTimer, navigationNotice = '';
let invalidReportNotice = '';
const directoryState = new Map();
const overviewDisclosureState = new Map();
function node(tag, text, className) { const n = document.createElement(tag); if (text !== undefined) n.textContent = text; if (className) n.className = className; return n; }
function list(value) { return Array.isArray(value) ? value : []; }
function isRecord(value) { return value !== null && typeof value === 'object' && !Array.isArray(value); }
function records(value) { return list(value).filter(isRecord); }
function text(value) { return typeof value === 'string' ? value : value === undefined || value === null ? '' : String(value); }
function contextText(value) { if (isRecord(value)) return typeof value.explanation === 'string' ? value.explanation.slice(0, 4000) : JSON.stringify(value).slice(0, 4000); return text(value).slice(0, 4000); }
function badge(label, kind = '') { return node('span', label, 'reviewbadge ' + kind); }
function empty(message) { return node('p', message, 'emptyview'); }
function sizeLabel(size) { return Number.isFinite(size) && size >= 0 ? size < 1024 ? `${size} B` : size < 1048576 ? `${(size / 1024).toFixed(1)} KiB` : `${(size / 1048576).toFixed(1)} MiB` : ''; }
async function api(path, body) {
 const response = await fetch('/api/v1/' + path, {method: body === undefined ? 'GET' : 'POST', credentials: 'same-origin', headers: body === undefined ? {} : {'Content-Type': 'application/json'}, body: body === undefined ? undefined : JSON.stringify(body)});
 if (!response.ok) { let error; try { error = await response.json(); } catch {} throw Error(error?.error || `Request failed (${response.status})`); }
 return response.json();
}
function switchTab(tab, focus = false) {
 activeTab = tab;
 for (const name of ['overview', 'code', 'findings']) { const selected = name === tab; $('tab-' + name).setAttribute('aria-selected', String(selected)); $('tab-' + name).tabIndex = selected ? 0 : -1; $(name + 'panel').hidden = !selected; }
 if (focus) $('tab-' + tab).focus();
}
function locationsOf(item) { return [item, ...records(item.locations)].filter(x => x && typeof x.file === 'string'); }
function fileFindings(name) { return list(report.findings).filter(item => locationsOf(item).some(x => x.file === name)); }
function fileObservations(name) { return list(report.observations).filter(item => locationsOf(item).some(x => x.file === name)); }
function prepareEntries() {
 const collected = new Map(), metadata = new Map(records(report.review_overview?.files).filter(file => typeof file.name === 'string').map(file => [file.name, file]));
 for (const item of [...list(report.inventory), ...list(report.files)]) {
  if (!item || typeof item.name !== 'string') continue;
  const previous = collected.get(item.name) || {};
  collected.set(item.name, {...previous, ...item, ...metadata.get(item.name), text: typeof item.text === 'string' ? item.text : previous.text});
 }
 for (const meta of metadata.values()) if (!collected.has(meta.name)) collected.set(meta.name, {...meta});
 entries = [...collected.values()].map(file => {
  const available = typeof file.text === 'string', findings = fileFindings(file.name), observations = fileObservations(file.name);
  return {...file, available, kind: available ? file.kind === 'decompiled' ? 'decompiled' : 'uploaded' : 'unavailable', findings, observations};
 }).sort((a, b) => a.name.localeCompare(b.name));
 entryMap = new Map(entries.map(entry => [entry.name, entry]));
}
function sourceLink(location) {
 const name = text(location?.file), line = Number(location?.line), exists = entryMap.has(name);
 const link = node('button', name ? `${name}${Number.isInteger(line) && line > 0 ? ':' + line : ''}` : 'Archive / binary evidence');
 link.type = 'button'; link.disabled = !exists; link.title = exists ? entryMap.get(name).available ? 'Open evidence in the source browser' : 'Inspect archive entry; no source preview is available' : 'No matching source or archive entry in this report';
 if (exists) link.addEventListener('click', () => { switchTab('code'); codeFile(name, line, true); });
 return link;
}
function renderFiles(force = false) {
 const pathQuery = $('filesearch').value.trim().toLowerCase(), codeQuery = $('codesearch').value.trim().toLowerCase(), kind = $('filetype').value || 'all';
 const filtered = entries.filter(file => (!pathQuery || file.name.toLowerCase().includes(pathQuery)) && (!codeQuery || file.available && file.text.toLowerCase().includes(codeQuery)) && (kind === 'all' || kind === 'source' && file.available || kind === 'decompiled' && file.kind === 'decompiled' || kind === 'uploaded' && file.kind === 'uploaded' || kind === 'unavailable' && !file.available || kind === 'findings' && file.findings.length));
 const signature = JSON.stringify([pathQuery, codeQuery, kind, treeLimit, selectedFile, filtered.map(file => [file.name, file.kind, file.findings.length, file.findings.filter(f => !f.accepted).length])]);
 $('filecount').textContent = `${filtered.length} of ${entries.length} files · ${filtered.filter(file => file.available).length} source previews${codeQuery ? ' · code matches only retained text' : ''}`;
 $('archivebadge').textContent = String(entries.length); $('morefiles').hidden = filtered.length <= treeLimit;
 $('morefiles').textContent = `Show ${Math.min(TREE_BATCH, filtered.length - treeLimit)} more files`;
 if (!force && signature === treeSignature) return;
 treeSignature = signature;
 const root = node('div'), groups = new Map(), folderCounts = new Map();
 for (const file of filtered) { const parts = file.name.split('/'); let path = ''; for (const part of parts.slice(0, -1)) { path += part + '/'; folderCounts.set(path, (folderCounts.get(path) || 0) + 1); } }
 for (const file of filtered.slice(0, treeLimit)) {
  const parts = file.name.split('/'); let parent = root, path = '';
  for (const part of parts.slice(0, -1)) {
   path += part + '/';
   if (!groups.has(path)) {
    const detail = node('details'), summary = node('summary', part || '/'), folderPath = path;
    detail.dataset.path = folderPath;
    detail.open = directoryState.has(folderPath) ? directoryState.get(folderPath) : parts.length <= 3 || !!pathQuery || !!codeQuery || selectedFile.startsWith(folderPath);
    summary.append(node('span', String(folderCounts.get(folderPath)), 'foldercount')); detail.append(summary);
    detail.addEventListener('toggle', () => directoryState.set(folderPath, detail.open)); parent.append(detail); groups.set(folderPath, detail);
   }
   parent = groups.get(path);
  }
  const button = node('button', undefined, 'filelink' + (file.name === selectedFile ? ' selected' : '') + (file.available ? '' : ' inventoryonly'));
  button.type = 'button'; button.title = file.name + ' · ' + (file.kind === 'decompiled' ? 'Reconstructed source' : file.available ? 'Uploaded source' : 'Inventory only; no text preview'); button.dataset.file = file.name;
  button.setAttribute('aria-current', file.name === selectedFile ? 'true' : 'false');
  button.append(node('span', file.kind === 'decompiled' ? '{ }' : file.available ? '</>' : '▧', 'filetypeicon'), node('span', parts.at(-1), 'filenameleaf'));
  if (file.findings.length) button.append(node('span', String(file.findings.length), 'filefindingcount'));
  button.addEventListener('click', () => codeFile(file.name, undefined, true)); parent.append(button);
 }
 if (!filtered.length) root.append(empty('No files match these filters. Code search covers retained source previews only.'));
 $('filetree').replaceChildren(root);
}
function appendMatches(parent, value, query) {
 if (!query) { parent.append(document.createTextNode(value)); return; }
 const lower = value.toLowerCase(), needle = query.toLowerCase(); let start = 0, count = 0, index;
 while ((index = lower.indexOf(needle, start)) !== -1 && count++ < 100) { parent.append(document.createTextNode(value.slice(start, index)), node('mark', value.slice(index, index + query.length))); start = index + query.length; }
 parent.append(document.createTextNode(value.slice(start)));
}
function highlight(value, query = '') {
 const fragment = document.createDocumentFragment(), pattern = /(\/\/.*$|#[^\n]*$|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|\b(?:public|private|protected|static|class|interface|namespace|using|return|if|else|for|foreach|while|new|try|catch|throw|async|await|void|int|string|bool|true|false|null|import|package|def|fn|let|const|function|var|extends|implements)\b|\b\d+(?:\.\d+)?\b)/g;
 let last = 0;
 for (const match of value.matchAll(pattern)) { appendMatches(fragment, value.slice(last, match.index), query); const token = match[0], kind = token.startsWith('//') || token.startsWith('#') ? 'comment' : token.startsWith('"') || token.startsWith("'") ? 'string' : /^\d/.test(token) ? 'number' : 'keyword'; const span = node('span', undefined, 'syntax-' + kind); appendMatches(span, token, query); fragment.append(span); last = match.index + token.length; }
 appendMatches(fragment, value.slice(last), query); return fragment;
}
function findMatches() {
 const query = $('infilesearch').value.toLowerCase(); matches = []; matchIndex = -1; matchesLimited = false;
 if (query && entryMap.get(selectedFile)?.available) {
  for (let index = 0; index < sourceLines.length; index++) { const value = sourceLines[index].toLowerCase(); let start = 0, position; while ((position = value.indexOf(query, start)) !== -1) { if (matches.length >= MATCH_LIMIT) { matchesLimited = true; break; } matches.push({line: index + 1, column: position}); start = position + query.length; } if (matchesLimited) break; }
 }
 renderMatchCount();
}
function renderMatchCount() {
 const query = $('infilesearch').value;
 $('searchcount').textContent = !query ? '' : !matches.length ? 'No matches' : `${matchIndex >= 0 ? matchIndex + 1 + ' / ' : ''}${matches.length}${matchesLimited ? '+' : ''} matches`;
 $('searchprevious').disabled = $('searchnext').disabled = !matches.length;
}
function nextMatch(direction) {
 if (!matches.length) return;
 matchIndex = (matchIndex + direction + matches.length) % matches.length; selectedLine = matches[matchIndex].line; navigationNotice = '';
 renderCode(true); renderMatchCount();
}
function renderCode(scroll = false, startAt) {
 const file = entryMap.get(selectedFile);
 if (!file?.available) return;
 const query = $('infilesearch').value, total = sourceLines.length, target = Math.min(Math.max(1, selectedLine), total);
 if (startAt !== undefined) codeStart = Math.min(Math.max(0, startAt), Math.max(0, total - 1));
 else if (scroll || target <= codeStart || target > codeEnd) codeStart = Math.max(0, target - 41);
 const signature = JSON.stringify([sourceRevision, codeStart, target, query, navigationNotice, file.findings.map(finding => [finding.id, locationsOf(finding).filter(location => location.file === selectedFile).map(location => location.line)])]);
 if (signature === renderedCodeSignature) { if (scroll) $('line-' + target)?.scrollIntoView({block: 'center'}); return; }
 renderedCodeSignature = signature;
 codeEnd = Math.min(total, codeStart + CODE_LINES);
 const previousTop = $('code').scrollTop, previousLeft = $('code').scrollLeft;
 const rows = [], findingLines = new Set(file.findings.flatMap(finding => locationsOf(finding).filter(location => location.file === selectedFile).map(location => Number(location.line))));
 let characters = 0, clipped = false;
 for (let index = codeStart; index < codeEnd; index++) {
  if (characters >= CODE_CHARACTERS && index >= target) { codeEnd = index; break; }
  let value = sourceLines[index]; if (value.length > LINE_CHARACTERS) { value = value.slice(0, LINE_CHARACTERS); clipped = true; } characters += value.length;
  const row = node('div', undefined, 'codeline'); row.id = 'line-' + (index + 1);
  const lineButton = node('button', String(index + 1), 'linenumber'); lineButton.type = 'button'; lineButton.title = 'Select line ' + (index + 1); lineButton.setAttribute('aria-label', 'Select line ' + (index + 1)); lineButton.addEventListener('click', () => { selectedLine = index + 1; navigationNotice = ''; $('gotoline').value = String(selectedLine); renderCode(false); });
  const code = node('code'); code.append(highlight(value, query)); if (sourceLines[index].length > LINE_CHARACTERS) code.append(node('span', ' … [long line display clipped]', 'syntax-comment'));
  row.append(lineButton, code); if (index + 1 === target) row.classList.add('flaggedline'); if (findingLines.has(index + 1)) row.classList.add('hasfinding'); if (query && sourceLines[index].toLowerCase().includes(query.toLowerCase())) row.classList.add('searchmatch'); rows.push(row);
 }
 $('code').replaceChildren(...rows); if (!scroll) { $('code').scrollTop = previousTop; $('code').scrollLeft = previousLeft; } $('gotoline').value = String(target); $('gotoline').max = String(total);
 $('codewindow').textContent = (navigationNotice ? navigationNotice + ' · ' : '') + `Lines ${codeStart + 1}–${codeEnd} of ${total.toLocaleString()}${codeStart || codeEnd < total ? ' · bounded window; search or jump to inspect any retained line' : ''}${clipped ? ' · long lines clipped for display; search still uses retained text' : ''}`;
 $('previouswindow').disabled = codeStart === 0; $('nextwindow').disabled = codeEnd >= total;
 if (scroll) $('line-' + target)?.scrollIntoView({block: 'center'});
}
function legacySymbols(lines) {
 const symbols = [];
 for (let index = 0; index < Math.min(lines.length, 20000) && symbols.length < 200; index++) {
  const value = lines[index].slice(0, 2000), type = value.match(/\b(class|interface|struct|enum|namespace)\s+([\w.]+)/), method = value.match(/^\s*(?:(?:public|private|protected|internal|static|override|virtual|async|sealed|final|abstract)\s+)*(?:[\w<>[\],?.]+\s+)+(\w+)\s*\([^;]*\)\s*(?:\{|$)/), functionMatch = value.match(/^\s*(?:export\s+)?(?:async\s+)?(?:function|def|fn)\s+(\w+)/);
  if (type) symbols.push({kind: type[1], name: type[2], line: index + 1}); else if (functionMatch) symbols.push({kind: 'function', name: functionMatch[1], line: index + 1}); else if (method && !['if', 'for', 'while', 'switch', 'catch', 'return'].includes(method[1])) symbols.push({kind: 'method', name: method[1], line: index + 1});
 }
 return symbols;
}
function renderOutline(file) {
 const meta = records(report.review_overview?.files).find(item => item.name === file.name), supplied = records(meta?.symbols), symbols = Array.isArray(meta?.symbols) ? supplied : file.available ? legacySymbols(sourceLines) : [], entryPoints = records(meta?.entry_points), entryKeys = new Set(entryPoints.map(item => item.name + ':' + item.line));
 const signature = JSON.stringify([sourceRevision, supplied, entryPoints]); if (signature === renderedOutlineSignature) return; renderedOutlineSignature = signature;
 $('outlinehint').textContent = file.available ? text(meta?.outline_note || 'Pattern-based outline, not a complete call graph. Inspect the source to verify each candidate.') + (meta?.outline_limited ? ' Outline coverage is limited; use text search to inspect omitted areas.' : '') : 'No retained source to outline.';
 const rows = [...entryPoints, ...symbols.filter(symbol => !entryKeys.has(symbol.name + ':' + symbol.line))].slice(0, 200).filter(symbol => Number.isInteger(symbol.line) && symbol.line > 0 && symbol.line <= sourceLines.length);
 $('symboloutline').replaceChildren(...rows.map(symbol => { const button = node('button', undefined, 'outlineitem' + (entryKeys.has(symbol.name + ':' + symbol.line) ? ' outlineentry' : '')); button.type = 'button'; button.title = `${text(symbol.kind)} · ${text(symbol.name)} · line ${symbol.line}`; button.append(node('span', String(symbol.line), 'outline-line'), node('span', text(symbol.name))); button.addEventListener('click', () => codeFile(file.name, symbol.line, true)); return button; }));
 if (!rows.length) $('symboloutline').append(node('p', 'No candidate symbols found.', 'sidehint'));
}
function renderFileEvidence(file) {
 $('filefindingsbutton').disabled = !file;
 const findings = file ? file.findings : [], observations = file ? file.observations : [];
 $('filefindingsbutton').textContent = findings.length ? `Review ${findings.length} file finding${findings.length === 1 ? '' : 's'}` : 'Review file evidence';
 const signature = JSON.stringify([file?.name, findings, observations, report.status, invalidReportNotice, report.sha256]); if (signature === renderedFileEvidenceSignature) return; renderedFileEvidenceSignature = signature;
 $('fileevidence').replaceChildren(...findings.map(finding => findingCard(finding, false)), ...observations.slice(0, 10).map(observation => findingCard(observation, true)));
 if (!findings.length && !observations.length) $('fileevidence').append(node('p', 'No findings or informational observations link to this file. Absence of findings is not a safety verdict.', 'sidehint'));
}
function decompilations() { return records(report.review_overview?.decompilations || report.decompilations); }
function codeFile(name, line, focus = false, preserve = false) {
 const file = entryMap.get(name); if (!file) return;
 const changed = selectedFile !== name; selectedFile = name;
 if (changed) { selectedLine = 1; codeStart = 0; codeEnd = 0; navigationNotice = ''; if (!preserve) $('infilesearch').value = ''; }
 const requested = Number(line); if (Number.isInteger(requested) && requested > 0) selectedLine = requested;
 $('filename').textContent = file.name;
 const kind = file.kind === 'decompiled' ? 'Reconstructed source' : file.available ? 'Uploaded source' : 'Archive inventory · no text preview';
 $('filekind').textContent = kind + (file.language ? ' · ' + text(file.language) : '') + (file.decompiler ? ' · ' + text(file.decompiler) : '');
 const meta = [badge(file.available ? file.kind === 'decompiled' ? 'Reconstructed' : 'Uploaded' : 'Inventory only', file.available ? '' : 'muted')];
 if (Number.isFinite(file.byte_size ?? file.size)) meta.push(node('span', sizeLabel(file.byte_size ?? file.size), 'sidehint'));
 if (file.findings.length) meta.push(badge(`${file.findings.filter(f => !f.accepted).length} unresolved / ${file.findings.length} findings`, 'warning'));
 if (file.origin) meta.push(node('span', file.origin === 'archive/' ? 'Origin: archive-wide reconstruction; individual class linkage unavailable' : 'Origin: ' + text(file.origin), 'sidehint'));
 $('filemeta').replaceChildren(...meta);
 const newSignature = JSON.stringify([file.name, report.sha256]), newText = file.available ? file.text : '';
 if (newSignature !== sourceSignature || newText !== sourceText) { sourceSignature = newSignature; sourceRevision++; renderedCodeSignature = ''; sourceText = newText; sourceLines = file.available ? file.text.split('\n') : []; findMatches(); }
 for (const id of ['infilesearch', 'gotoline']) $(id).disabled = !file.available;
 $('lineform').querySelector('button').disabled = !file.available;
 if (file.available) {
  if (!preserve && Number.isInteger(requested) && requested > 0) navigationNotice = requested > sourceLines.length ? `Requested line ${requested} is outside the retained preview (ends at ${sourceLines.length}); inspect the analysis limits` : '';
  if (changed && !(Number.isInteger(requested) && requested > 0) && $('codesearch').value.trim()) { const query = $('codesearch').value.trim().toLowerCase(), index = sourceLines.findIndex(value => value.toLowerCase().includes(query)); if (index >= 0) selectedLine = index + 1; }
  selectedLine = Math.min(selectedLine, sourceLines.length); renderCode(focus && !preserve);
 }
 else {
  $('codewindow').textContent = 'Inventory metadata only · no source text retained'; $('previouswindow').disabled = $('nextwindow').disabled = true;
  const content = node('div', undefined, 'sourceempty'); content.append(node('h3', 'No source preview for this entry'), node('p', text(file.preview_reason) || 'This entry is present in the archive inventory, but this report did not retain a text preview. Binary and resource metadata cannot substitute for source inspection.'));
  const records = decompilations().filter(record => record.input === file.name || list(record.inputs).includes(file.name));
  for (const record of records.slice(0, 8)) { content.append(node('p', `${text(record.tool)}: ${text(record.status)}${record.scope === 'project' ? ' · project-wide reconstruction; source cannot be mapped to this individual class' : ''}`)); if (record.diagnostic) content.append(node('p', text(record.diagnostic))); const links = node('div', undefined, 'evidencelinks'); for (const source of list(record.generated_files).filter(source => typeof source === 'string' || isRecord(source)).slice(0, 12)) links.append(sourceLink({file: typeof source === 'string' ? source : source.name})); content.append(links); }
  $('code').replaceChildren(content); renderMatchCount();
 }
 renderOutline(file); renderFileEvidence(file); renderFiles(); if ($('findingfilefilter').value === 'selected') renderFindings();
 if (focus) $('code').focus({preventScroll: true});
}
function appendEvidence(card, item) {
 const links = node('div', undefined, 'evidencelinks'); links.append(sourceLink(item)); card.append(links, node('pre', text(item.evidence) || 'Review the analysis coverage and archive.'));
 if (item.context) card.append(node('p', 'Context: ' + contextText(item.context)));
 const locations = records(item.locations).filter(location => location.file !== item.file || location.line !== item.line || location.evidence !== item.evidence);
 if (locations.length) { const detail = node('details'); detail.append(node('summary', `${locations.length} related evidence location${locations.length === 1 ? '' : 's'}`)); for (const location of locations) { const row = node('div'); row.append(sourceLink(location)); if (location.evidence) row.append(node('pre', text(location.evidence))); detail.append(row); } card.append(detail); }
}
function findingCard(item, observation = false) {
 const card = node('article', undefined, 'finding'), headline = node('div', undefined, 'findingheadline');
 headline.append(node('strong', text(item.title) || 'Review evidence'), badge(observation ? 'Informational' : item.accepted ? 'Accepted' : 'Review required', observation || item.accepted ? 'muted' : 'warning')); card.append(headline);
 if (!observation) card.append(node('p', [text(item.severity), text(item.rule)].filter(Boolean).join(' · ')));
 appendEvidence(card, item); if (item.reason) card.append(node('p', 'Review reason: ' + text(item.reason)));
 if (!observation && report.status !== 'rejected') { const button = node('button', item.accepted ? 'Reopen finding' : 'Accept finding with reason', 'decisionbutton'); button.type = 'button'; button.disabled = !!invalidReportNotice || typeof item.id !== 'string' || !item.id || !/^[a-f0-9]{64}$/i.test(text(report.sha256)); button.addEventListener('click', () => { if (invalidReportNotice) return; decisionTarget = {id: item.id, accepted: !!item.accepted, sha256: report.sha256}; $('decisiontitle').textContent = item.accepted ? 'Reopen finding' : 'Accept finding'; $('decisionevidence').textContent = text(item.title) + ' · ' + text(item.file || 'archive'); $('decisionreason').value = ''; $('decisionrecord').disabled = false; $('findingdecision').showModal(); $('decisionreason').focus(); }); card.append(button); }
 return card;
}
function renderFindings() {
 const status = $('findingfilter').value || 'open', byFile = $('findingfilefilter').value === 'selected', query = $('findingsearch').value.trim().toLowerCase();
 const eligible = item => (!byFile || selectedFile && locationsOf(item).some(location => location.file === selectedFile)) && (!query || [item.title, item.rule, item.evidence, contextText(item.context), ...records(item.locations).flatMap(location => [location.file, location.evidence])].map(text).join(' ').toLowerCase().includes(query));
 const rows = list(report.findings).filter(item => eligible(item) && (status === 'all' || status === 'accepted' ? status === 'all' || item.accepted : !item.accepted));
 const observations = list(report.observations).filter(eligible), signature = JSON.stringify([status, byFile, byFile ? selectedFile : '', query, rows, observations, report.status, invalidReportNotice, report.sha256]);
 if (signature !== renderedFindingsSignature) {
  renderedFindingsSignature = signature; $('findings').replaceChildren(...rows.map(item => findingCard(item)));
  if (!rows.length) $('findings').append(empty(byFile && !selectedFile ? 'Choose a file in Code & files to focus this view.' : status === 'open' ? 'No unresolved findings match this view. Inspect source and analysis coverage before approving.' : 'No findings match these filters.'));
  $('observations').replaceChildren(...observations.map(item => findingCard(item, true)));
  if (!observations.length) $('observations').append(node('p', 'No informational observations match this view.', 'sidehint'));
 }
 const unresolved = list(report.findings).filter(item => !item.accepted).length; $('findingbadge').textContent = String(unresolved); $('approve').disabled = report.status !== 'complete' || unresolved > 0 || !!invalidReportNotice || !/^[a-f0-9]{64}$/i.test(text(report.sha256));
}
function fallbackSuggestions() {
 const suggestions = [];
 for (const finding of list(report.findings).filter(item => !item.accepted).slice(0, 8)) suggestions.push({id: finding.id, priority: 'high', category: 'Finding context', title: text(finding.title), why: 'This unresolved finding requires your review. Inspect its exact location and surrounding control flow before recording a decision.', checks: ['Trace who calls this code and whether it runs automatically.', 'Check inputs, path boundaries, network destinations and error handling as relevant to this evidence.'], locations: locationsOf(finding), advisory: true});
 if (!suggestions.length) suggestions.push({id: 'legacy-coverage', priority: 'normal', category: 'Coverage', title: 'Inspect startup behavior and analysis limits', why: 'This report has no unresolved findings. That alone does not establish safety or complete source coverage.', checks: ['Compare the full archive inventory with retained source previews.', 'Inspect initialization, patch registration and externally supplied inputs.', 'Review unavailable or incomplete engines and binaries before making a decision.'], locations: []});
 return suggestions;
}
function renderOverview() {
 const overview = report.review_overview || {}, unresolved = list(report.findings).filter(item => !item.accepted).length, sources = entries.filter(entry => entry.available), decompiled = sources.filter(entry => entry.kind === 'decompiled').length;
 const signature = JSON.stringify([overview, report.sha256, report.status, report.findings, report.observations, report.engines, report.decompilations, entries.map(entry => [entry.name, entry.kind])]); if (signature === renderedOverviewSignature) return; renderedOverviewSignature = signature;
 const metrics = [['Archive files', entries.length, `${sources.length} retained source previews`], ['Reconstructed source', decompiled, `${sources.length - decompiled} uploaded source files`], ['Unresolved findings', unresolved, `${list(report.findings).length} findings in this report`], ['No text preview', entries.length - sources.length, 'Inspect coverage and binary evidence']];
 $('reviewmetrics').replaceChildren(...metrics.map(([label, count, detail]) => { const card = node('article', undefined, 'reviewmetric'); card.append(node('strong', String(count)), node('span', label), node('small', detail)); return card; }));
 $('overviewbadge').textContent = overview.coverage?.state === 'incomplete' ? 'Limits' : '';
 const suggestions = records(overview.suggestions).length ? records(overview.suggestions) : fallbackSuggestions();
 $('suggestedchecks').replaceChildren(...suggestions.slice(0, 40).map(suggestion => { const card = node('article', undefined, 'suggestion'); card.append(badge(text(suggestion.category) || 'Inspection', suggestion.priority === 'high' ? 'warning' : ''), node('h3', text(suggestion.title)), node('p', text(suggestion.why))); const checks = node('ul'); for (const check of list(suggestion.checks)) checks.append(node('li', text(check))); card.append(checks); const links = node('div', undefined, 'evidencelinks'); for (const location of records(suggestion.locations).slice(0, 12)) links.append(sourceLink(location)); card.append(links); return card; }));
 const coverageState = overview.coverage?.state || 'unknown', coverage = node('div');
 coverage.append(node('p', coverageState === 'complete' ? 'Reported coverage complete' : coverageState === 'incomplete' ? 'Analysis coverage incomplete' : 'Coverage not established by this report', 'coveragestate' + (coverageState === 'complete' ? '' : ' coveragewarning')));
 coverage.append(node('p', text(overview.note) || 'Inspect engine results and retained source. A completed report is not a safety guarantee.', 'sidehint'));
 for (const record of decompilations().slice(0, 80)) {
  const detail = node('details', undefined, 'decompilation'), key = text(record.input) + ':' + text(record.tool); detail.open = overviewDisclosureState.get(key) || false; detail.addEventListener('toggle', () => overviewDisclosureState.set(key, detail.open)); detail.append(node('summary', `${text(record.input)} · ${text(record.tool)} · ${text(record.status)}`));
  detail.append(node('p', record.scope === 'project' ? 'Project-wide reconstruction; individual class-to-source linkage is unavailable.' : 'Binary-level reconstruction. Retained source remains an approximation.'));
  if (Number.isFinite(record.generated_count)) detail.append(node('p', `${record.generated_count} generated · ${Number(record.preview_count || 0)} retained previews${record.omitted_count ? ' · ' + record.omitted_count + ' omitted previews' : ''}`));
  if (Number.isFinite(record.duration_ms)) detail.append(node('p', `${(record.duration_ms / 1000).toFixed(2)} seconds${record.exit_code !== null && record.exit_code !== undefined ? ' · exit ' + text(record.exit_code) : ''}`));
  const links = node('div', undefined, 'evidencelinks');
  if (entryMap.has(record.input)) links.append(sourceLink({file: record.input}));
  for (const source of list(record.generated_files).filter(source => typeof source === 'string' || isRecord(source)).slice(0, 12)) links.append(sourceLink({file: typeof source === 'string' ? source : source.name}));
  detail.append(links); if (record.mapping_note) detail.append(node('p', text(record.mapping_note))); for (const limit of list(record.limitations)) detail.append(node('p', text(limit))); if (record.diagnostic) detail.append(node('pre', text(record.diagnostic))); coverage.append(detail);
 }
 $('coverage').replaceChildren(coverage);
 const engines = records(overview.engines).length ? records(overview.engines) : Object.entries(report.engines || {}).map(([name, engine]) => ({name, ...(isRecord(engine) ? engine : {status:'Unknown engine data'})}));
 $('engines').replaceChildren(...engines.map(engine => { const row = node('div', undefined, 'engineitem'); row.append(node('span', text(engine.name)), badge(text(engine.status) + (engine.version ? ' · ' + text(engine.version) : ''), ['complete', 'clean', 'available'].includes(engine.status) ? '' : 'muted')); return row; }));
 if (!engines.length) $('engines').append(node('p', 'Engine details unavailable in this legacy report.', 'sidehint'));
 const capabilities = records(overview.capabilities); $('capabilities').replaceChildren(...capabilities.map(capability => { const row = node('div', undefined, 'capabilityitem'); row.append(node('span', text(capability.label || capability.rule)), badge(`${Number(capability.count || 0)} locations${capability.unresolved_count ? ' · ' + capability.unresolved_count + ' open' : ''}`, capability.unresolved_count ? 'warning' : 'muted')); return row; }));
 if (!capabilities.length) $('capabilities').append(node('p', 'No capability summary in this report. Use the Findings view and source browser.', 'sidehint'));
 const components = records(overview.components).slice(0, 32);
 if (components.length) {
  const detail = node('details');
  detail.append(node('summary', `${components.length} reconstructed components`), node('p', 'Direct file findings grouped by their recorded binary origin. Grouped coverage limits remain in Findings. Names and archive placement do not verify trust or whether a caller runs.', 'sidehint'));
  for (const component of components) {
   const row = node('div', undefined, 'capabilityitem reviewcomponent');
   row.append(sourceLink({file: component.input}), badge(`${Number(component.finding_count || 0)} findings · ${text(component.status)}`, component.unresolved_count ? 'warning' : 'muted'));
   row.append(node('p', text(component.placement_hint) + ' · ' + records(component.rules).map(rule => `${text(rule.rule)}: ${Number(rule.count || 0)}`).join(' · '), 'sidehint'));
   if (isRecord(component.metadata)) {
    const names = list(component.metadata.payload_name_hints).filter(name => typeof name === 'string').slice(0, 64).map(name => name.slice(0, 256));
    const meta = node('details');
    meta.append(node('summary', 'CLR metadata hints · ' + text(component.metadata.status)), node('p', text(component.metadata.note) || 'String names do not prove resource use or clear packing findings.', 'sidehint'));
    if (names.length) meta.append(node('pre', names.join('\n'))); else meta.append(node('p', 'No patch-like names retained.'));
    if (component.metadata.heap_limit_reached || component.metadata.name_limit_reached) meta.append(node('p', 'Metadata hint coverage was limited.', 'sidehint'));
    row.append(meta);
   }
   detail.append(row);
  }
  $('capabilities').append(detail);
 }
 const dependencies = records(overview.dependencies); if (dependencies.length) { const detail = node('details'); detail.append(node('summary', `${dependencies.length} declared dependencies`), node('p', 'Declared metadata only; versions and compatibility are not verified here.', 'sidehint')); for (const dependency of dependencies.slice(0, 50)) { const row = node('p', text(dependency.name), 'sidehint'); if (dependency.file) row.append(sourceLink({file: dependency.file})); detail.append(row); } $('capabilities').append(detail); }
 $('reporthash').textContent = text(report.sha256) || 'Archive hash unavailable';
 const limits = Object.entries(overview.limits || {}).filter(([, value]) => value && (typeof value !== 'number' || value > 0)); $('reportlimits').textContent = limits.length ? 'Retained overview limits: ' + limits.map(([key, value]) => key.replaceAll('_', ' ') + ': ' + text(value)).join(' · ') : 'Source windows and outline lists are bounded for responsiveness. Inventory filters include files without a preview.';
}
async function load() {
 const generation = ++loadGeneration; clearTimeout(poll);
 try {
  const next = await api(`mods/${modId}/analysis`); if (generation !== loadGeneration) return;
  if (!isRecord(next)) throw Error('Invalid analysis response; approval is blocked until a readable report is loaded.');
  const invalid = [];
  for (const field of ['files','findings','observations','inventory','decompilations']) if (next[field] !== undefined && (!Array.isArray(next[field]) || list(next[field]).some(item => !isRecord(item)))) invalid.push(field);
  if (records(next.findings).some(item => typeof item.id !== 'string' || !item.id || item.accepted !== undefined && typeof item.accepted !== 'boolean')) invalid.push('finding decisions');
  invalidReportNotice = invalid.length ? 'Malformed report data (' + invalid.join(', ') + '); review decisions and approval are blocked. Run analysis again to replace this report.' : '';
  report = {...next, files: records(next.files), findings: records(next.findings).map(item => ({...item, accepted: item.accepted === true})), observations: records(next.observations), inventory: records(next.inventory)}; prepareEntries();
  $('modname').textContent = text(report.mod_name) || 'Mod review';
  const error = report.status === 'rejected' ? 'Denied by policy: packing, obfuscation or malware signature detected. See findings and Admin Logs.' : text(report.error);
  $('reviewstatus').textContent = `Analysis: ${text(report.status) || 'unknown'}${error ? ' · ' + error : ''} · ${report.findings.length} findings · ${report.observations.length} informational observations${invalidReportNotice ? ' · ' + invalidReportNotice : ''}`;
  renderOverview(); renderFindings(); renderFiles();
  if (selectedFile && entryMap.has(selectedFile)) codeFile(selectedFile, selectedLine, false, true);
  else if (entries.length) codeFile(entries.find(entry => entry.available)?.name || entries[0].name, undefined, false, true);
  else { selectedFile = ''; sourceSignature = ''; sourceLines = []; sourceText = ''; matches = []; matchIndex = -1; renderMatchCount(); for (const id of ['infilesearch', 'gotoline', 'previouswindow', 'nextwindow', 'filefindingsbutton']) $(id).disabled = true; $('lineform').querySelector('button').disabled = true; $('filename').textContent = 'No files available yet'; $('filekind').textContent = report.status === 'pending' || report.status === 'queued' ? 'Source previews appear as analysis completes.' : 'This report retained no archive or source entries.'; $('filemeta').replaceChildren(); $('code').replaceChildren(empty('No retained source in this report. Inspect analysis status and coverage.')); $('codewindow').textContent = ''; $('outlinehint').textContent = ''; $('symboloutline').replaceChildren(); $('fileevidence').replaceChildren(); }
  if (decisionTarget && (decisionTarget.sha256 !== report.sha256 || invalidReportNotice)) { $('decisionevidence').textContent = invalidReportNotice || 'The archive hash changed. Close this dialog and review the new report before recording a decision.'; $('decisionrecord').disabled = true; }
  if (report.status === 'pending') { try { await api(`mods/${modId}/analysis`, {}); } catch (requestError) { if (generation === loadGeneration) $('reviewstatus').textContent = requestError.message; } if (generation === loadGeneration) poll = setTimeout(load, 4000); }
  else if (report.status === 'queued') poll = setTimeout(load, 2500);
 } catch (error) { if (generation === loadGeneration) { $('reviewstatus').textContent = error.message; $('approve').disabled = true; } }
}
for (const tab of ['overview', 'code', 'findings']) {
 $('tab-' + tab).addEventListener('click', () => switchTab(tab));
 $('tab-' + tab).addEventListener('keydown', event => { const tabs = ['overview', 'code', 'findings']; if (['ArrowRight', 'ArrowLeft', 'Home', 'End'].includes(event.key)) { event.preventDefault(); const index = tabs.indexOf(activeTab); switchTab(event.key === 'Home' ? tabs[0] : event.key === 'End' ? tabs[2] : tabs[(index + (event.key === 'ArrowRight' ? 1 : 2)) % 3], true); } });
}
$('refreshreport').addEventListener('click', load);
$('rescan').addEventListener('click', async () => { if (!await cannaConfirm('Run analysis again? Previous finding decisions will be replaced, and downloads stay blocked until the new report is reviewed.')) return; try { await api(`mods/${modId}/analysis`, {force: true}); await load(); } catch (error) { $('reviewstatus').textContent = error.message; } });
$('approve').addEventListener('click', async () => { if (!await cannaConfirm('Publish this mod and its dependencies? Each dependency must have completed analysis and resolved findings. Approval is your review decision, not a safety guarantee.')) return; try { await api(`mods/${modId}/approve`, {}); $('reviewstatus').textContent = 'Mod and dependencies approved.'; } catch (error) { $('reviewstatus').textContent = error.message; } });
$('decisioncancel').addEventListener('click', () => { decisionTarget = undefined; $('findingdecision').close(); });
$('decisionform').addEventListener('submit', async event => { event.preventDefault(); if (!decisionTarget || decisionTarget.sha256 !== report.sha256 || invalidReportNotice) { $('decisionevidence').textContent = invalidReportNotice || 'Archive changed; review the new report before deciding.'; return; } const decision = {...decisionTarget}, button = $('decisionrecord'); button.disabled = true; try { await api(`mods/${modId}/analysis/${decision.id}`, {accepted: !decision.accepted, reason: $('decisionreason').value, sha256: decision.sha256}); $('findingdecision').close(); decisionTarget = undefined; await load(); } catch (error) { $('decisionevidence').textContent = error.message; button.disabled = false; } });
function filterFiles() { treeLimit = TREE_BATCH; renderFiles(); }
$('filesearch').addEventListener('input', filterFiles); $('filetype').addEventListener('change', filterFiles);
$('codesearch').addEventListener('input', () => { clearTimeout(fileSearchTimer); fileSearchTimer = setTimeout(filterFiles, 160); });
$('clearfiles').addEventListener('click', () => { $('filesearch').value = $('codesearch').value = ''; $('filetype').value = 'all'; filterFiles(); });
$('morefiles').addEventListener('click', () => { treeLimit += TREE_BATCH; renderFiles(); });
for (const [id, open] of [['expandfiles', true], ['collapsefiles', false]]) $(id).addEventListener('click', () => { for (const detail of $('filetree').querySelectorAll('details')) { detail.open = open; directoryState.set(detail.dataset.path, open); } });
$('infilesearch').addEventListener('input', () => { findMatches(); if (matches.length) nextMatch(1); else renderCode(false); });
$('infilesearch').addEventListener('keydown', event => { if (event.key === 'Enter') { event.preventDefault(); nextMatch(event.shiftKey ? -1 : 1); } else if (event.key === 'Escape') { event.preventDefault(); $('infilesearch').value = ''; findMatches(); renderCode(false); } });
$('searchprevious').addEventListener('click', () => nextMatch(-1)); $('searchnext').addEventListener('click', () => nextMatch(1));
$('lineform').addEventListener('submit', event => { event.preventDefault(); const line = Number($('gotoline').value); if (Number.isInteger(line) && sourceLines.length) { navigationNotice = line > sourceLines.length ? `Requested line ${line} is outside the retained preview (ends at ${sourceLines.length}); inspect the analysis limits` : ''; selectedLine = Math.min(Math.max(line, 1), sourceLines.length); renderCode(true); } });
$('previouswindow').addEventListener('click', () => { navigationNotice = ''; selectedLine = Math.max(1, codeStart - CODE_LINES + 1); renderCode(false, Math.max(0, codeStart - CODE_LINES)); $('code').scrollTop = 0; });
$('nextwindow').addEventListener('click', () => { navigationNotice = ''; selectedLine = Math.min(sourceLines.length, codeEnd + 1); renderCode(false, codeEnd); $('code').scrollTop = 0; });
$('code').addEventListener('keydown', event => { if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f') { event.preventDefault(); $('infilesearch').focus(); } });
$('filefindingsbutton').addEventListener('click', () => { $('findingfilefilter').value = 'selected'; $('findingfilter').value = 'all'; renderFindings(); switchTab('findings', true); });
for (const id of ['findingfilter', 'findingfilefilter']) $(id).addEventListener('change', renderFindings); $('findingsearch').addEventListener('input', renderFindings);
window.addEventListener('pagehide', () => { clearTimeout(poll); clearTimeout(fileSearchTimer); loadGeneration++; });
load();

