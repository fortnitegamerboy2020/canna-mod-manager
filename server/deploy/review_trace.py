"""Bounded C# navigation hints. Never executes code or clears review findings.

This is a lexical call-site index, not a C# compiler/data-flow proof. Ambiguous
symbols, branches, callbacks, external calls and dynamic paths stay unresolved.
"""
import hashlib
import json
import re
from bisect import bisect_right

METHOD = re.compile(r'(?m)^\s*(?:(?:public|private|internal|protected|static|virtual|override|async|sealed|new|unsafe|extern)\s+)+(?:[\w.<>,?\[\]()+ ]+\s+)?(?P<name>\w+)\s*\([^;{}]*\)\s*(?:where[^{};]+)?\{')
CALL = re.compile(r'\b(?P<name>[A-Za-z_]\w*(?:\s*\.\s*[A-Za-z_]\w*)*)\s*\(')
ASSIGN = re.compile(r'\b(?P<name>[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*)\s*=(?!=|>)\s*(?P<value>[^;\n]{1,600})[;\n]')
FILE_API = re.compile(r'\b(?:File|Directory)\s*\.\s*(?:Read\w*|Write\w*|Delete|Move|Copy|Open\w*|GetFiles|Enumerate\w*|CreateDirectory)\s*\(|\bFileStream\s*\(')


def closing(code, start, left='(', right=')'):
    depth = 0
    for i in range(start, min(len(code), start + 10000)):
        if code[i] == left:
            depth += 1
        elif code[i] == right:
            depth -= 1
            if depth == 0:
                return i
    return None


def arguments(content, code, start, end):
    depth, begin, result = 0, start + 1, []
    for i in range(begin, end):
        if code[i] in '([{':
            depth += 1
        elif code[i] in ')]}':
            depth -= 1
        elif code[i] == ',' and depth == 0:
            result.append(content[begin:i].strip()[:600]); begin = i + 1
    result.append(content[begin:end].strip()[:600])
    return result[:8]


def annotate(text, findings, source_views, executable):
    if not findings:
        return findings
    content, code = source_views(text, '.cs')
    starts = [0] + [m.end() for m in re.finditer('\n', code)]
    line = lambda pos: bisect_right(starts, pos)
    methods, declarations = [], []
    for match in METHOD.finditer(code):
        declarations.append((match.start(), match.end()))
        if re.search(r'\b(?:class|record|struct|interface)\s', match[0].split('(',1)[0]):
            continue
        end = closing(code, match.end() - 1, '{', '}')
        if end is not None:
            signature = content[match.start():match.end()]
            parameter_text = signature[signature.find('(')+1:signature.rfind(')')]
            parameters = [re.findall(r'\b\w+\b', part.split('=')[0])[-1] for part in parameter_text.split(',') if re.findall(r'\b\w+\b', part.split('=')[0])]
            methods.append({'name': match['name'], 'parameters': parameters, 'start': match.start(), 'body': match.end(), 'end': end, 'line': line(match.start() + len(match[0]) - len(match[0].lstrip()))})
        if len(methods) >= 512:
            break

    def owner(pos):
        candidates = [m for m in methods if m['body'] <= pos <= m['end']]
        return min(candidates, key=lambda m: m['end'] - m['body'], default=None)

    calls = []
    for match in CALL.finditer(code):
        if any(begin <= match.start() < end for begin,end in declarations):
            continue
        end = closing(code, match.end() - 1)
        if end is not None:
            calls.append({'name': re.sub(r'\s+', '', match['name']), 'pos': match.start(), 'end': end, 'line': line(match.start()), 'args': arguments(content, code, match.end() - 1, end), 'owner': owner(match.start())})
        if len(calls) >= 4096:
            break
    assignments = []
    for match in ASSIGN.finditer(code):
        assignments.append({'name': match['name'], 'pos': match.start(), 'line': line(match.start()), 'expression': content[match.start('value'):match.end('value')].strip(), 'owner': owner(match.start())})
        if len(assignments) >= 2048:
            break

    trace_bytes = 0
    for finding in findings:
        if finding['rule'] not in {'filesystem', 'dynamic', 'network', 'commands', 'native', 'diagnostic-output'} or not isinstance(finding.get('line'), int):
            continue
        pos = starts[min(len(starts)-1, finding['line']-1)]
        expected = finding.get('operation')
        operation = next((c for c in calls if c['line'] == finding['line'] and (c['name'].removeprefix('System.IO.') == expected if expected else FILE_API.match(code, c['pos']) or finding['rule'] != 'filesystem')), None)
        method = operation['owner'] if operation else owner(pos)
        trace = {'scope': 'same-source-file', 'method': method['name'] if method else 'unresolved', 'callers': [], 'paths': [], 'limits': 'Lexical hints only; branches and aliases are not resolved. External callers, reflection and callbacks may be missing. No path confinement or safety is established.'}
        if method:
            pending, seen = [(method['name'], 0)], set()
            while pending and len(trace['callers']) < 12:
                name, depth = pending.pop(0)
                if name in seen or depth >= 3:
                    continue
                seen.add(name)
                for call in calls:
                    if call['name'].split('.')[-1] == name:
                        caller = call['owner']
                        trace['callers'].append({'file': finding['file'], 'line': call['line'], 'method': caller['name'] if caller else 'unresolved', 'arguments': call['args'], 'depth': depth + 1, 'ambiguous': sum(m['name'] == name for m in methods) != 1})
                        if caller:
                            pending.append((caller['name'], depth + 1))
                        if len(trace['callers']) >= 12:
                            break
        if operation and finding['rule'] == 'filesystem':
            destinations = operation['args'][:2] if operation['name'] in {'File.Copy', 'File.Move'} else operation['args'][:1]
            for expression in destinations:
                path = {'expression': expression, 'assignments': [], 'resolved': False}
                pending, seen = [(expression, method, operation['pos'], 0)], set()
                while pending and len(path['assignments']) < 10:
                    value, scope, before, depth = pending.pop(0)
                    if depth >= 4:
                        continue
                    for token in re.findall(r'\b[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*', source_views(value, '.cs')[1]):
                        key = (token, scope['start'] if scope else -1, before)
                        if key in seen:
                            continue
                        seen.add(key)
                        # Include competing/conditional assignments. Never pick one as proof.
                        candidates = [a for a in assignments if a['name'] == token and a['pos'] < before and (a['owner'] is None or a['owner'] is scope)]
                        for assignment in candidates[-4:]:
                            path['assignments'].append({'file': finding['file'], 'line': assignment['line'], 'name': token, 'expression': assignment['expression']})
                            pending.append((assignment['expression'], scope, assignment['pos'], depth+1))
                            if len(path['assignments']) >= 10:
                                break
                        if scope and token in scope['parameters'] and sum(m['name'] == scope['name'] for m in methods) == 1:
                            index = scope['parameters'].index(token)
                            for caller in [c for c in calls if c['name'].split('.')[-1] == scope['name'] and len(c['args']) > index][:3]:
                                if len(path['assignments']) >= 10:
                                    break
                                path['assignments'].append({'file':finding['file'], 'line':caller['line'], 'name':token+' (candidate caller argument)', 'expression':caller['args'][index]})
                                pending.append((caller['args'][index],caller['owner'],caller['pos'],depth+1))
                        if len(path['assignments']) >= 10:
                            break
                trace['paths'].append(path)
            values = ' '.join(p['expression'] + ' ' + ' '.join(a['expression'] for a in p['assignments']) for p in trace['paths'])
            write = bool(re.search(r'(?:Write|Copy|Move|Open|FileStream)', operation['name']))
            if operation['name'] == 'FileStream' and re.search(r'\bFileAccess\s*\.\s*Read\b', ' '.join(operation['args'])) and not re.search(r'\bFileMode\s*\.\s*(?:Create\w*|Truncate|Append)\b', ' '.join(operation['args'])):
                write = False
            if write and executable.search(values):
                finding['title'] = 'Executable file write or replacement requires review'
                finding['severity'] = 'high'
                trace['effect'] = 'executable-file-operation'
            elif operation['name'] == 'Directory.CreateDirectory':
                finding['title'] = 'Directory creation: inspect destination and callers'
                trace['effect'] = 'directory-creation'
            else:
                trace['effect'] = 'file-operation'
                if not write and re.search(r'Read|FileStream', operation['name']):
                    finding['title'] = 'File read: inspect path and use of returned bytes'
        trace['callers'] = trace['callers'][:6]
        for caller in trace['callers']:
            caller['arguments'] = [arg[:160] for arg in caller['arguments'][:4]]
        for path in trace['paths']:
            path['expression'] = path['expression'][:350]
            for assignment in path['assignments']:
                assignment['expression'] = assignment['expression'][:350]
        size = len(json.dumps(trace).encode())
        if trace_bytes + size > 1024 * 1024:
            trace = {'scope':'same-source-file', 'method':trace['method'], 'callers':[], 'paths':[], 'limits':'Trace detail budget reached. Inspect original evidence and source manually; no findings were cleared.'}
        else:
            trace_bytes += size
        finding['trace'] = trace
        finding['context'] = finding.get('context', '') + ' Trace hints identify the enclosing method, candidate callers and destination assignments. Unknown paths remain unresolved; executable replacement may be legitimate mod repair but still requires review.'
    # Repeated reflection/decoding in serializers is one capability review per
    # file/method/title. Every original evidence location remains inspectable.
    groups = {}
    result = []
    for finding in findings:
        if finding['rule'] not in {'filesystem', 'dynamic'} or not finding.get('trace') or finding['trace']['method'] == 'unresolved':
            result.append(finding); continue
        key = (finding['rule'], finding['file'], finding['title'], finding['trace']['method'], finding['severity'])
        groups.setdefault(key, []).append(finding)
    for group in groups.values():
        if len(group) == 1:
            result.extend(group); continue
        first = dict(group[0])
        first['id'] = hashlib.sha256(('trace-group\0' + '\0'.join(f['id'] for f in group)).encode()).hexdigest()
        first.pop('accepted', None)
        first['locations'] = [{'file': f['file'], 'line': f['line'], 'evidence': f['evidence'], 'trace': f['trace']} for f in group]
        first['context'] += f' {len(group)} related operations are grouped; inspect each retained location. Grouping does not accept findings.'
        result.append(first)
    return result
