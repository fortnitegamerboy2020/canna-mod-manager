"""Bounded C# path/capability classification; no execution or mod allowlists.

Only recognized loader-root expressions, literal child segments and unambiguous
constructor/record bindings become informational. Unknown, mutable, aliased or
dynamic destinations retain review. This is source evidence, not a filesystem
confinement proof (runtime configuration and symlinks are outside static scope).
"""
import hashlib
import re
import json

ROOTS = {'CachePath': 'loader-cache', 'ManagedPath': 'game-managed', 'PluginPath': 'loader-plugins'}
READS = {'ReadAllBytes', 'ReadAllText', 'ReadAllLines', 'ReadLines', 'OpenRead', 'OpenText'}
ENUMERATE = {'GetFiles', 'GetDirectories', 'EnumerateFiles', 'EnumerateDirectories', 'EnumerateFileSystemEntries'}
OPERATIONS = re.compile(r'\b(?P<name>(?:(?:global::)?System\.IO\.)?(?:File|Directory)\s*\.\s*(?:Read\w*|Write\w*|Append\w*|Delete|Move|Copy|Replace|Create\w*|Open\w*|GetFiles|GetDirectories|Enumerate\w*)|(?:(?:global::)?System\.IO\.)?FileStream)\s*\(')
PRIMARY = re.compile(r'\b(?:class|record)\s+(\w+)\s*\(([^{};]*)\)\s*(?:\{|;)')
SHADOW = re.compile(r'\b(?:class|struct|record|interface|enum)\s+(?:Paths|Path|File|Directory|FileStream)\b|\busing\s+(?:Paths|Path|File|Directory|FileStream)\s*=|\bnamespace\s+(?:BepInEx|System\.IO)\b|\b(?:var|object|dynamic|\w+)\s+(?:Paths|Path|File|Directory|FileStream)\s*[=;,)]')


def split_args(expression, views, trace):
    _, code = views(expression, '.cs')
    start = code.find('(')
    end = trace.closing(code, start) if start >= 0 else None
    if end != len(expression.rstrip()) - 1:
        return None
    return trace.arguments(expression, code, start, end)


class Paths:
    def __init__(self, files, views, trace):
        self.views, self.trace = views, trace
        self.files = [(f['name'], *views(f['text'], '.cs')) for f in files if f.get('language') == 'csharp' or f['name'].endswith('.cs')]
        # Unrelated DTO members named File/Path in another namespace do not
        # shadow System.IO in this source. Loader/type redefinitions do.
        self.disabled = any(re.search(r'\b(?:class|struct|record|interface|enum)\s+(?:Paths|Path|File|Directory|FileStream)\b|\bnamespace\s+(?:BepInEx|System\.IO)\b', code) for _, _, code in self.files)
        self.primary = {}
        self.generated_records = set()
        for name, content, code in self.files:
            for match in PRIMARY.finditer(code):
                parameters = [re.findall(r'\w+', p) for p in content[match.start(2):match.end(2)].split(',')]
                if all(len(p) >= 2 for p in parameters):
                    self.primary.setdefault(match[1], []).append((name, [(p[-2], p[-1]) for p in parameters]))
                    if re.search(r'\brecord\s', match[0]) and match[0].rstrip().endswith(';'):
                        self.generated_records.add((name,match[1]))

    def file(self, name):
        return next((f for f in self.files if f[0] == name), None)

    def mode_shadowed(self, name):
        source = self.file(name)
        if source is None:
            return True
        code = source[2]
        if re.search(r'\busing\s+(?:FileMode|FileAccess)\s*=|\b(?:var|object|dynamic|\w+)\s+(?:FileMode|FileAccess)\s*[=;,)]', code):
            return True
        namespaces = set(re.findall(r'\bnamespace\s+([\w.]+)', code))
        imports = set(re.findall(r'\busing\s+([\w.]+)\s*;', code))
        for _, _, other in self.files:
            # Inherited or externally initialized members can substitute mode
            # values. Their absence needs scoped binding before lowering reads.
            if any(m[1] not in {'class','struct','record','interface','enum','return','throw','new'} for m in re.finditer(r'\b(\w+)\s+(?:FileMode|FileAccess)\s*[=;,){]', other)):
                return True
            if not re.search(r'\b(?:class|struct|record|interface|enum)\s+(?:FileMode|FileAccess)\b', other):
                continue
            owners = set(re.findall(r'\bnamespace\s+([\w.]+)', other))
            if len(owners) != 1 or owners.intersection(namespaces | imports):
                return True
        return False

    def resolve(self, expression, name, depth=0, seen=()):
        expression = expression.strip()
        key = (name, expression)
        if self.disabled or depth >= 8 or key in seen or len(expression) > 600:
            return None
        source = self.file(name)
        if source is None:
            return None
        _, content, code = source
        if SHADOW.search(code):
            return None
        seen = (*seen, key)
        root = re.fullmatch(r'(?:(?:global::)?BepInEx\.)?Paths\.(\w+)', expression)
        if root and root[1] in ROOTS and (expression.startswith(('BepInEx.', 'global::BepInEx.')) or re.search(r'\busing\s+BepInEx\s*;', code)):
            return {'root': ROOTS[root[1]], 'segments': [], 'bindings': [{'file': name, 'expression': expression}]}
        combine = re.match(r'(?:(?:global::)?System\.IO\.)?Path\.Combine\(', expression)
        if combine:
            args = split_args(expression, self.views, self.trace)
            if not args or len(args) < 2:
                return None
            result = self.resolve(args[0], name, depth+1, seen)
            if result is None:
                return None
            for arg in args[1:]:
                try:
                    child = json.loads(arg)
                except (ValueError, TypeError):
                    return None
                if not isinstance(child, str) or not re.fullmatch(r'[A-Za-z0-9_. -]{1,120}', child) or child in {'.', '..'} or child.rstrip(' .') != child:
                    return None
                result['segments'].append(child)
            return result
        if not re.fullmatch(r'[A-Za-z_]\w*(?:\.\w+)?', expression):
            return None
        symbol, _, member = expression.partition('.')
        declarations = [m for m in re.finditer(r'\b([\w.<>\[\]?]+)\s+'+re.escape(symbol)+r'\s*(?=[;,)=])', code) if m[1] not in {'return','throw','yield','case','ref','out'}]
        if len(declarations) > 1:
            return None
        # A method/lambda parameter can hide an immutable field or a primary
        # constructor parameter. Without scoped binding, fail closed even if
        # the shadow is in another method in this retained source.
        for signature in re.finditer(r'\b[\w.<>\[\]?]+\s+\w+(?:<[^>]*>)?\s*\(([^{};]*)\)\s*(?:\{|=>)', code):
            if signature[0].split()[0] in {'class', 'record', 'struct'}:
                continue
            if re.search(r'\b'+re.escape(symbol)+r'\b', signature[1]):
                return None
        if re.search(r'(?:\b'+re.escape(symbol)+r'|\([^()]*\b'+re.escape(symbol)+r'\b[^()]*\))\s*=>', code):
            return None
        # Reassignments, ref/out aliases and shadowed declarations make paths unknown.
        if re.search(r'\b(?:ref|out)\s+'+re.escape(symbol)+r'\b|\b'+re.escape(symbol)+r'(?:\.\w+)*\s*(?:\+=|-=|\+\+|--)', code) or member and re.search(r'\b'+re.escape(expression)+r'\s*=(?!=)', code):
            return None
        assignments = list(re.finditer(r'\b'+re.escape(symbol)+r'\s*=(?!=|>)\s*([^;\n]+)', code))
        if assignments:
            if len(assignments) != 1 or member:
                return None
            assignment = assignments[0]
            # Only immutable field initializers; locals need scoped control-flow
            # analysis before they can lower a finding (assignment order matters).
            if not re.search(r'\b(?:readonly|const)\s+string\s*$', code[max(0,assignment.start()-80):assignment.start()]):
                return None
            value = content[assignment.start(1):assignment.end(1)].strip()
            result = self.resolve(value, name, depth+1, seen)
            if result:
                result['bindings'].append({'file': name, 'line': code.count('\n', 0, assignment.start())+1, 'expression': symbol+' = '+value})
            return result
        candidates = [(cls, params) for cls, definitions in self.primary.items() for owner, params in definitions if owner == name and any(p[1] == symbol for p in params)]
        if len(candidates) != 1:
            return None
        cls, parameters = candidates[0]
        if len(self.primary[cls]) != 1:
            return None
        index = next(i for i,p in enumerate(parameters) if p[1] == symbol)
        kind = parameters[index][0]
        constructors = []
        for caller, caller_content, caller_code in self.files:
            for match in re.finditer(r'\bnew\s+'+re.escape(cls)+r'\s*\(', caller_code):
                end = self.trace.closing(caller_code, match.end()-1)
                if end is None:
                    return None
                args = self.trace.arguments(caller_content, caller_code, match.end()-1, end)
                if len(args) <= index or len(constructors) >= 16:
                    return None
                argument = args[index]
                if member:
                    definitions = self.primary.get(kind, [])
                    if len(definitions) != 1 or (definitions[0][0],kind) not in self.generated_records:
                        return None
                    fields = definitions[0][1]
                    indices = [i for i,p in enumerate(fields) if p[1] == member]
                    if len(indices) != 1 or not re.match(r'new\s+(?:'+re.escape(cls)+r'\.)?'+re.escape(kind)+r'\s*\(', argument):
                        return None
                    values = split_args(argument, self.views, self.trace)
                    if not values or len(values) <= indices[0]:
                        return None
                    argument = values[indices[0]]
                resolved = self.resolve(argument, caller, depth+1, seen)
                if resolved is None:
                    return None
                resolved['bindings'].append({'file': caller, 'line': caller_code.count('\n', 0, match.start())+1, 'expression': cls+' constructor: '+argument})
                constructors.append(resolved)
        if not constructors or any((r['root'],r['segments']) != (constructors[0]['root'],constructors[0]['segments']) for r in constructors):
            return None
        result = constructors[0]
        result['bindings'] = [binding for r in constructors for binding in r['bindings']][:12]
        return result


def classify(report, context):
    """Rebuild C# filesystem sites, including multiple operations on one line."""
    sources = report.get('files', [])
    paths = Paths(sources, context.source_views, context.trace)
    if any(f['rule'] == 'coverage' for f in report['findings']):
        paths.disabled = True
    names = {name for name, _, _ in paths.files}
    retained = [f for f in report['findings'] if f['rule'] != 'filesystem' or f.get('file') not in names]
    observations = report.get('observations', [])
    for name, content, code in paths.files:
        if not OPERATIONS.search(code):
            continue
        sites = []
        for match in list(OPERATIONS.finditer(code))[:4097]:
            if len(sites) >= 4096:
                retained.append(context.entry('coverage', 'Filesystem site limit reached', name, None, 'Further operations were omitted.', 'high'))
                break
            end = context.trace.closing(code, match.end()-1)
            if end is None:
                retained.append(context.entry('coverage', 'Filesystem arguments could not be inspected', name, code.count('\n',0,match.start())+1, 'Incomplete or oversized operation.', 'high'))
                continue
            args = context.trace.arguments(content, code, match.end()-1, end)
            api = re.sub(r'\s+', '', match['name']).replace('global::', '').removeprefix('System.IO.')
            operation = api.split('.')[-1]
            readonly = operation in READS or api.startswith('Directory.') and operation in ENUMERATE
            if operation in {'FileStream', 'Open'}:
                readonly = len(args) == 3 and args[1].strip() == 'FileMode.Open' and args[2].strip() == 'FileAccess.Read' and not paths.mode_shadowed(name)
            destinations = args[:2] if operation in {'Copy', 'Move', 'Replace'} else args[:1]
            resolved = [paths.resolve(arg, name) for arg in destinations]
            line = code.count('\n',0,match.start())+1
            evidence = content[match.start():end+1].replace('\n',' ')[:350]
            finding = context.entry('filesystem', 'File operation: destination remains unresolved', name, line, evidence)
            finding['operation'] = api
            finding['path_classification'] = {'access': 'read' if readonly else 'create-directory' if api == 'Directory.CreateDirectory' else 'write-or-delete', 'destinations': [{'expression':arg,'resolved':value} for arg,value in zip(destinations,resolved)], 'note': 'Bounded source expressions; runtime loader configuration, external callers and symlinks are not verified.'}
            known = bool(resolved) and all(resolved)
            suspicious = bool(context.SENSITIVE.search(evidence) or context.PERSISTENCE.search(evidence))
            advisory = known and not suspicious and (readonly or api == 'Directory.CreateDirectory' and resolved[0]['root'] == 'loader-cache')
            if advisory:
                finding['rule'] = 'filesystem-context'
                finding['severity'] = 'info'
                finding['title'] = 'Loader cache directory setup' if not readonly else 'Read-only game or loader file access'
                finding['context'] = 'Recognized loader-root construction and candidate constructor bindings are retained. This operation does not replace executable code. Separate writes, loading, networking, sensitive data and coverage findings still require review.'
            else:
                finding['title'] = 'File read: unresolved destination or consumer' if readonly else 'Directory creation: unresolved destination' if api == 'Directory.CreateDirectory' else 'File write, replacement or deletion requires review'
                finding['context'] = 'Unknown paths and destructive operations remain review findings. A cache-like name or non-executable extension is insufficient to clear a write.'
            # Distinct call sites on one line must not inherit one another's decisions.
            finding['id'] = hashlib.sha256(('filesystem-v10\0'+name+'\0'+str(match.start())+'\0'+evidence+'\0'+finding['rule']).encode()).hexdigest()
            if not readonly and api != 'Directory.CreateDirectory' and context.EXECUTABLE.search(evidence+' '+json.dumps(resolved)):
                finding['severity'] = 'high'
                finding['title'] = 'Executable file write or replacement requires review'
            sites.append(finding)
        traced = context.trace_operations(content, sites)
        for finding in traced:
            if finding['severity'] == 'info':
                if len(observations) < 500:
                    observations.append(finding)
                else:
                    retained.append(context.entry('coverage', 'Observation preview limit reached', name, None, 'Filesystem context could not be retained.', 'high'))
            else:
                retained.append(finding)
    if len(retained) > 2000:
        # Fail closed rather than silently dropping an executable operation.
        retained = sorted(retained, key=lambda f: f['severity'] != 'high')[:1999]
        retained.append(context.entry('coverage', 'Finding limit reached', None, None, 'Further semantic findings were omitted; coverage is incomplete.', 'high'))
    report['findings'], report['observations'] = retained, observations
    report['filesystem_context'] = {'version': 1, 'scope': 'retained C# sources', 'informational_sites':sum(f['rule']=='filesystem-context' for f in observations), 'root_resolution_disabled':paths.disabled, 'note': 'No mod identity, filename extension or framework name is an allowlist. Unknown paths, executable writes and incomplete coverage retain review.'}


def group_capabilities(findings):
    """One capability row per binary; every source site remains unresolved."""
    groups, result = {}, []
    for finding in findings:
        name = finding.get('file') or ''
        binary = re.match(r'(?:decompiled/)?(archive/.+?\.(?:dll|exe))(?:/|$)', name)
        if not binary or finding['rule'] not in {'dynamic','packing-review'}:
            result.append(finding)
            continue
        title = finding['title'] if finding['rule'] == 'dynamic' else 'Packing indicators requiring context'
        groups.setdefault((binary[1],finding['rule'],title,finding['severity']),[]).append(finding)
    for (binary,rule,title,severity), members in groups.items():
        if len(members) == 1:
            result.extend(members)
            continue
        members.sort(key=lambda f: (f.get('file') or '', f.get('line') or 0, f['id']))
        first = dict(members[0])
        first['id'] = hashlib.sha256(('capability-v10\0'+ '\0'.join(f['id'] for f in members)).encode()).hexdigest()
        first.pop('accepted',None)
        first['locations'] = [location for f in members for location in (f.get('locations') or [{k:v for k,v in f.items() if k not in {'id','accepted','reason'}}])]
        first['title'] = title+f' ({len(first["locations"])} sites)'
        first['binary'] = binary
        first['context'] = (first.get('context') or '')+f' Repeated capability evidence in {binary} is grouped for inspection, not accepted. All original locations remain unresolved under this finding. A library name, readable source or generic detector does not establish safety or maliciousness.'
        result.append(first)
    return result
