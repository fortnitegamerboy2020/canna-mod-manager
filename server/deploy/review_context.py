"""Bounded source heuristics, never a parser or a proof of safe behavior.

Comments and literal references are distinguished from API use. Unknown paths
still require review; non-executable extensions are not a trust boundary.
"""
import hashlib
import importlib.util
import re
from bisect import bisect_right
from pathlib import Path

_trace_spec = importlib.util.spec_from_file_location('canna_review_trace', Path(__file__).with_name('review_trace.py'))
trace = importlib.util.module_from_spec(_trace_spec)
_trace_spec.loader.exec_module(trace)


def trace_operations(text, findings):
    return trace.annotate(text, findings, source_views, EXECUTABLE)

RULES = [
    ('network', 'Network API use', r'\b(?:HttpClient|WebClient|UnityWebRequest|Socket|TcpClient|UdpClient|URLConnection|HttpURLConnection|XMLHttpRequest|Invoke-WebRequest|Invoke-RestMethod)\b|\brequests\s*\.\s*(?:get|post|put|delete|request)\s*\(|\b(?:fetch|urlopen|curl|wget)\s*(?:\(|\s)|\bApplication\s*\.\s*OpenURL\b|\baxios\s*\.', 'review'),
    ('identity', 'Device or account information', r'\b(?:GetPhysicalAddress|NetworkInterface|GetHostAddresses|GetHostName)\b|\bEnvironment\s*\.\s*(?:MachineName|UserName|GetEnvironmentVariable)\b|\bSystem\s*\.\s*getProperty\b|\bgetenv\s*\(', 'review'),
    ('filesystem', 'File operations requiring path review', r'\bFile\s*\.\s*(?:Read\w*|Write\w*|Delete|Move|Copy|Open\w*)\b|\bDirectory\s*\.\s*(?:Delete|GetFiles|Enumerate\w*|CreateDirectory)\b|\b(?:FileStream|FileInputStream|FileOutputStream|fstream|ifstream|ofstream)\b|\bFiles\s*\.\s*(?:read\w*|write\w*|delete\w*|move|copy)\b|(?<![.\w])open\s*\(|\b(?:fopen|unlink|remove)\s*\(|\b(?:std\s*::\s*)?fs\s*::\s*(?:read\w*|write|remove\w*|rename|copy|create_dir\w*)\b', 'review'),
    ('commands', 'Process or shell execution', r'\bProcess\s*\.\s*Start\b|\bProcessStartInfo\b|\bRuntime\s*\.\s*getRuntime\b|\bProcessBuilder\b|\bos\s*\.\s*system\b|\bsubprocess\s*\.|\b(?:Invoke-Expression|Start-Process|iex)\b|\bExec\s+Command\b', 'high'),
    ('privileges', 'Privilege changes or persistence', r'\b(?:AdjustTokenPrivileges|OpenProcessToken|CreateService|setuid)\b|\b(?:runas|schtasks|sudo)\b', 'high'),
    ('native', 'Native calls or memory access', r'\b(?:DllImport|LibraryImport|VirtualAlloc|WriteProcessMemory|CreateRemoteThread|GetProcAddress|LoadLibrary)\b|\bUnsafe\s*\.|\bsun\s*\.\s*misc\s*\.\s*Unsafe\b', 'review'),
    ('dynamic', 'Dynamic code or decoded data', r'\bAssembly\s*\.\s*Load\w*\b|\bActivator\s*\.\s*CreateInstance\b|\b(?:FromBase64String|defineClass|DownloadString)\b|(?<![.\w])eval\s*\(|\bInvoke-Expression\b', 'review'),
]
RULES = [(rule, title, re.compile(pattern), severity) for rule, title, pattern, severity in RULES]
SENSITIVE = re.compile(r'Login Data|Local State|Cookies|\.ssh\b|wallet\.dat|key4\.db|logins\.json|discord.{0,30}token|CryptUnprotectData|ProtectedData\s*\.\s*Unprotect', re.I)
PERSISTENCE = re.compile(r'CurrentVersion[\\/]+(?:Run|RunOnce)\b', re.I)
URL = re.compile(r'https?://[^\s"\'<>]+', re.I)
SHELL = re.compile(r'\b(?:powershell|cmd\.exe)\b|/bin/(?:sh|bash)\b', re.I)
EXECUTABLE = re.compile(r'\.(?:exe|dll|com|scr|msi|bat|cmd|ps1|sh|bash|py|js|jar|class|so|dylib|vbs|hta|lnk)\b', re.I)
RUST_RAW = re.compile(r'r(#{0,32})"')
CPP_RAW = re.compile(r'R"([^()\s\\]{0,16})\(')
LUA_RAW = re.compile(r'\[(=*)\[')
RUST_LIFETIME = re.compile(r"'[A-Za-z_]\w*")
QUOTE_RUN = re.compile(r'"+')

# Exact canonical prose, not a trust rule for LICENSE or .txt filenames. Only
# whitespace varies; changed, prepended and appended content stays inspectable.
LICENSE_PROSE = (
    ('GPL-3.0', 'GNU GENERAL PUBLIC LICENSE', 28640,
     'db4017480bcedfc101e5e54d3befbabe89352069d0dd192799e56feda43556f6'),
    ('Apache-2.0', 'Apache License', 7717,
     'c3716932c5840f8354d59b26fad06bc508567480da225392ab9b982ab4bde9b3'),
    ('MIT', 'Permission is hereby granted, free of charge', 859,
     'eaf9b7a559ea4e12a265565be41d1c5b45215e8686a22e1cacd5f7e110819728'),
)
DOCUMENT_SUFFIXES = {'', '.txt', '.md'}
SCRIPT_TEXT = re.compile(r'(?im)^\s*(?:\$[\w{]|param\s*\(|#requires\b|#!|(?:Start-Process|Invoke-Expression|Invoke-WebRequest|Invoke-RestMethod|iwr|irm|iex)\b)')
SOURCE_TEXT = re.compile(r'(?m)^\s*(?:using\s+[\w.]+\s*;|(?:public|private|internal|class|namespace|function|def)\s+[\w<])|\b(?:File|Directory|Process|Assembly|Activator)\s*\.|\b(?:HttpClient|DllImport|LibraryImport)\b')


def license_prose_regions(text, suffix):
    """Recognize exact full canonical bodies within a bounded leading region.

    Matching a header, extension or an excerpt is insufficient. Only the
    verified body is ignored by the code lexer; all other bytes stay visible.
    """
    if suffix not in DOCUMENT_SUFFIXES:
        return []
    regions = []
    for title, marker, length, digest in LICENSE_PROSE:
        candidate = re.search(r'\s+'.join(map(re.escape, marker.split())), text[:4096])
        if candidate is None:
            continue
        begin = candidate.start()
        normalized = []
        end = begin
        while end < len(text) and len(normalized) < length:
            char = text[end]
            if not char.isspace():
                normalized.append(char)
            end += 1
        if len(normalized) == length and hashlib.sha256(''.join(normalized).encode()).hexdigest() == digest:
            regions.append((begin, end, title))
    return sorted(regions)


def document_scan_suffix(text, suffix, regions):
    """Detect script syntax outside verified prose without trusting the filename."""
    if suffix not in DOCUMENT_SUFFIXES:
        return suffix
    remaining = list(text)
    for begin, end, _ in regions:
        for pos in range(begin, end):
            if remaining[pos] not in '\r\n':
                remaining[pos] = ' '
    remaining = ''.join(remaining)
    if SCRIPT_TEXT.search(remaining):
        return '.ps1'
    if SOURCE_TEXT.search(remaining):
        return '.cs'
    return suffix


def markdown_has_code(text):
    """A Markdown suffix is not permission to skip recognized code or commands."""
    if SCRIPT_TEXT.search(text) or SOURCE_TEXT.search(text):
        return True
    return any(pattern.search(text) for _, _, pattern, _ in RULES) or bool(SENSITIVE.search(text) or PERSISTENCE.search(text))


def dynamic_description(token):
    compact = re.sub(r'\s+', '', token).lower()
    if compact.startswith('activator.'):
        return ('Reflection-based instance construction',
                'A runtime-selected type is instantiated. This can implement ordinary framework or value-type defaults; trace the type origin, constructor and callers. This match does not establish decoded payload execution.')
    if compact.startswith('assembly.'):
        return ('Assembly loading',
                'Trace the assembly path or bytes and their source before loading, including user/network input and cached or embedded resources. The call alone does not identify the loaded code.')
    if compact == 'frombase64string':
        return ('Base64 data decoding',
                'Decoding can produce configuration, assets or executable bytes. Follow the result to its consumers; decoding alone is not code execution.')
    if compact == 'downloadstring':
        return ('Downloaded text retrieval',
                'Inspect the destination, caller and subsequent use of the retrieved text. Retrieval alone does not establish evaluation or loading of code.')
    if compact == 'defineclass':
        return ('Runtime class definition', 'Trace the class bytes and source before definition; the final implementation may not be visible at this call site.')
    return ('Dynamic expression or script evaluation', 'Inspect the evaluated expression or script and its input sources. This capability remains subject to review.')


def source_views(text, suffix, issues=None, prose_regions=()):
    """Return comment-free text and a same-offset view with literals masked.

    Interpolated/template strings stay visible conservatively: expressions in
    those strings may execute. Newlines/offsets survive for source navigation.
    """
    content, code = list(text), list(text)
    size, i = len(text), 0
    hash_comments = suffix in {'.py', '.sh', '.ps1', '.toml', '.yml', '.yaml', '.cfg', '.ini', '.properties'}
    slash_comments = suffix not in {'.py', '.sh', '.toml', '.yml', '.yaml', '.cfg', '.ini', '.properties'}

    def mask(target, start, end):
        for pos in range(start, end):
            if text[pos] not in '\r\n':
                target[pos] = ' '

    while i < size:
        prose = next((region for region in prose_regions if region[0] == i), None)
        if prose:
            mask(code, i, prose[1])
            i = prose[1]
            continue
        # Raw strings have different delimiters/escaping from ordinary quotes.
        raw = RUST_RAW.match(text, i) if suffix == '.rs' and text[i] == 'r' else None
        cpp_raw = CPP_RAW.match(text, i) if suffix in {'.cpp', '.c', '.h', '.hpp'} and text[i] == 'R' else None
        lua_raw = LUA_RAW.match(text, i) if suffix == '.lua' and text[i] == '[' else None
        if raw or cpp_raw or lua_raw:
            token = raw or cpp_raw or lua_raw
            delimiter = '"' + raw[1] if raw else ')' + cpp_raw[1] + '"' if cpp_raw else ']' + lua_raw[1] + ']'
            closing = text.find(delimiter, token.end())
            end = size if closing < 0 else closing + len(delimiter)
            if closing < 0 and issues is not None:
                issues.append('Unterminated raw string')
            mask(code, i, end)
            i = end
            continue
        # A JS regex literal can contain quotes without starting a string.
        if suffix in {'.js', '.ts'} and text[i] == '/' and not text.startswith(('//', '/*'), i):
            prior = text[max(0, i - 128):i].rstrip()
            if not prior or prior[-1] in '=(:,![{;?' or re.search(r'\b(?:return|case|throw)\s*$', prior):
                begin, i, bracket, closed = i, i + 1, False, False
                while i < size and text[i] not in '\r\n':
                    if text[i] == '\\':
                        i += 2
                        continue
                    if text[i] == '[':
                        bracket = True
                    elif text[i] == ']':
                        bracket = False
                    elif text[i] == '/' and not bracket:
                        i += 1
                        closed = True
                        break
                    i += 1
                if not closed and issues is not None:
                    issues.append('Unterminated regular expression')
                mask(code, begin, min(i, size))
                continue
        end, block_comment = None, False
        if slash_comments and text.startswith('/*', i):
            block_comment = True
            closing = text.find('*/', i + 2)
            end = size if closing < 0 else closing + 2
        elif suffix == '.ps1' and text.startswith('<#', i):
            block_comment = True
            closing = text.find('#>', i + 2)
            end = size if closing < 0 else closing + 2
        elif suffix in {'.lua', '.nut'} and text.startswith('--[[', i):
            block_comment = True
            closing = text.find(']]', i + 4)
            end = size if closing < 0 else closing + 2
        elif (slash_comments and text.startswith('//', i)) or (hash_comments and text[i] == '#' and (suffix != '.sh' or i == 0 or text[i - 1].isspace() or text[i - 1] in ';|&(')) or (suffix == '.lua' and text.startswith('--', i)):
            closing = text.find('\n', i)
            end = size if closing < 0 else closing
        if end is not None:
            if block_comment and closing < 0 and issues is not None:
                issues.append('Unterminated comment')
            mask(content, i, end)
            mask(code, i, end)
            i = end
            continue
        if suffix == '.rs' and text[i] == "'":
            lifetime = RUST_LIFETIME.match(text, i)
            if lifetime and (lifetime.end() == size or text[lifetime.end()] != "'"):
                i = lifetime.end()
                continue
        if text[i] in {'"', "'", '`'}:
            quote, begin = text[i], i
            triple = suffix in {'.py', '.cs', '.java'} and text.startswith(quote * 3, i)
            delimiter_length = 3
            if triple and suffix == '.cs':
                delimiter_length = len(QUOTE_RUN.match(text, i)[0])
            verbatim = suffix == '.cs' and i > 0 and text[i - 1] == '@'
            prefix = text[max(0, i - 2):i].lower()
            interpolated = quote == '`' or (suffix == '.cs' and '$' in prefix) or (suffix == '.py' and 'f' in prefix) or (suffix == '.ps1' and quote == '"')
            i += delimiter_length if triple else 1
            braces, closed = 0, False
            while i < size:
                if interpolated and text[i] == '{':
                    braces += 1
                elif interpolated and text[i] == '}' and braces:
                    braces -= 1
                if interpolated and braces and text[i] in {'"', "'", '`'}:
                    inner_quote = text[i]
                    i += 1
                    while i < size:
                        if text[i] == '\\':
                            i += 2
                        elif text[i] == inner_quote:
                            i += 1
                            break
                        else:
                            i += 1
                    continue
                if triple and text.startswith(quote * delimiter_length, i) and not braces:
                    i += delimiter_length
                    closed = True
                    break
                if not triple and text[i] == quote and not braces:
                    if (verbatim or suffix == '.ps1') and text.startswith(quote * 2, i):
                        i += 2
                        continue
                    i += 1
                    closed = True
                    break
                if (text[i] == '\\' and not verbatim and suffix != '.ps1' and not (suffix == '.sh' and quote == "'")) or (suffix == '.ps1' and quote == '"' and text[i] == '`'):
                    i += 2
                else:
                    i += 1
            if not interpolated:
                mask(code, begin, min(i, size))
            if not closed and issues is not None:
                issues.append('Unterminated string or unsupported interpolation')
            continue
        i += 1
    return ''.join(content), ''.join(code)


def entry(rule, title, name, line, evidence, severity='review', context=None):
    evidence = evidence[:350]
    key = '\0'.join(map(str, [rule, name, line, evidence]))
    result = {'id': hashlib.sha256(key.encode()).hexdigest(), 'rule': rule,
              'title': title, 'file': name, 'line': line,
              'evidence': evidence, 'severity': severity}
    if context:
        result['context'] = context
    return result


def scan_source(text, name, suffix):
    issues = []
    prose_regions = license_prose_regions(text, suffix)
    suffix = document_scan_suffix(text, suffix, prose_regions)
    content, code = source_views(text, suffix, issues, prose_regions)
    findings, observations = [], []
    overflow = False

    def emit(item, informational=False):
        nonlocal overflow
        target = observations if informational else findings
        if len(target) < (500 if informational else 2000):
            target.append(item)
            return
        if not overflow:
            findings.append(entry('coverage', 'Source finding limit reached', name, None,
                                  'Some source matches or observations were omitted; coverage is incomplete.', 'high'))
            overflow = True
        if not informational and item['severity'] == 'high':
            replace = next((i for i, old in enumerate(findings) if old['severity'] == 'review'), None)
            if replace is not None:
                findings[replace] = item
    original_lines = text.splitlines()
    content_lines, code_lines = content.splitlines(), code.splitlines()
    starts = [0] + [m.end() for m in re.finditer('\n', text)]
    matched_lines = {}
    declarations = [(m.start(), m.end()) for m in trace.METHOD.finditer(code)] if suffix == '.cs' else []
    network_declarations = list(re.finditer(r'\b(?:HttpClient|WebClient|TcpClient|UdpClient|Socket)\s+(\w+)\s*(?:[;=,)]|$)', code)) if suffix == '.cs' else []
    for begin, end, license_name in prose_regions:
        emit(entry('documentation-reference', 'Recognized canonical license prose', name,
                   bisect_right(starts, begin), license_name + ': exact canonical prose matched after whitespace normalization.', 'info',
                   'Only this unchanged license body is excluded from code lexing. Added or changed text remains inspectable; this is not a mod approval or a redistribution-rights determination.'), True)
    if issues:
        emit(entry('coverage', 'Source lexical coverage is incomplete', name, None,
                   '; '.join(sorted(set(issues))), 'high'))
    active_rules = RULES
    if suffix in {'.ps1', '.bat', '.cmd'}:
        active_rules = [(rule, title, re.compile(pattern.pattern, re.I), severity) for rule, title, pattern, severity in RULES]
    if suffix == '.ps1':
        active_rules = active_rules + [('network', 'Network API use', re.compile(r'(?<![\w$])(?:iwr|irm)\b', re.I), 'review')]
    if suffix in {'.sh', '.bat', '.cmd'}:
        active_rules = active_rules + [('commands', 'Shell execution', re.compile(r'(?<![\w$])(?:sh|bash|source|call|exec)\b', re.I), 'high')]
    for rule, title, pattern, severity in active_rules:
        for match in pattern.finditer(code):
            index = bisect_right(starts, match.start())
            if (rule, index) in matched_lines:
                continue
            evidence = original_lines[index - 1].strip()
            if '\n' in match[0]:
                evidence = match[0].strip()
            if len(matched_lines) < 2500:
                matched_lines[rule, index] = True
            contextual = None
            if rule == 'network' and any(declaration.start() == match.start() and ';' in declaration[0] for declaration in network_declarations):
                matched_lines.pop((rule,index),None)
                emit(entry('declaration-reference', 'Network client declaration, not a request', name, index, evidence, 'info',
                           'A field or variable type is declared here. Construction and identified request call sites are inspected separately; this declaration alone does not contact a server.'), True)
                continue
            if rule == 'dynamic':
                title, contextual = dynamic_description(match[0])
                if match[0] in {'FromBase64String', 'DownloadString'} and any(begin <= match.start() < end for begin, end in declarations):
                    matched_lines.pop((rule,index),None)
                    emit(entry('declaration-reference', 'API-like method declaration, not a call', name, index, evidence, 'info',
                               'This match names a method declaration. Its body and call sites are scanned separately; the name itself is not decoding or a network request.'), True)
                    continue
            emit(entry(rule, title, name, index, evidence, severity, contextual))
    if network_declarations:
        names = sorted({declaration[1] for declaration in network_declarations})
        requests = re.compile(r'\b(?:' + '|'.join(map(re.escape, names)) + r')\s*\.\s*(?:Send(?:Async)?|Get(?:Async|StringAsync|ByteArrayAsync|StreamAsync)|PostAsync|PutAsync|DeleteAsync|Download(?:Data|String|File)(?:Async)?)\s*\(')
        for match in requests.finditer(code):
            index = bisect_right(starts, match.start())
            if ('network', index) not in matched_lines:
                emit(entry('network', 'Request through a declared network client', name, index, original_lines[index-1].strip(), 'review',
                           'The receiver has a network-client declaration in this source file. Inspect the request destination, returned data and callers; lexical matching does not resolve every alias or shadowed variable.'))
                matched_lines['network',index] = True
    for index, (visible, executable) in enumerate(zip(content_lines, code_lines), 1):
        evidence = original_lines[index - 1].strip()
        matches = {rule for rule, _, _, _ in active_rules if (rule, index) in matched_lines}
        if SENSITIVE.search(visible):
            emit(entry('sensitive-files', 'Sensitive credential or browser paths', name, index, evidence, 'high'))
        if PERSISTENCE.search(visible):
            emit(entry('privileges', 'Persistence registry path', name, index, evidence, 'high'))
        if URL.search(visible) and 'network' not in matches:
            emit(entry('url-reference', 'Referenced URL', name, index, evidence, 'info',
                       'A URL reference is not itself a network request. It can still be used elsewhere; this does not establish offline behavior.'), True)
        if SHELL.search(visible) and 'commands' not in matches:
            # Shell scripts have executable command words without a process API.
            if suffix in {'.ps1', '.sh', '.bat', '.cmd'}:
                emit(entry('commands', 'Shell command', name, index, evidence, 'high'))
            else:
                emit(entry('shell-reference', 'Shell name in source text', name, index, evidence, 'info',
                           'No process-start API was matched on this line. Other call sites and dynamic code still require review.'), True)
    return findings, observations


def contextualize_file_operations(text, findings):
    """Summarize repeated CLI-directed diagnostics, retaining manual review.

    This deliberately does not whitelist a mod, path variable or extension. A
    recognizable CLI assignment adds context, not a safety/acceptance decision.
    Any delete/copy/read/unknown output prevents the diagnostic summary.
    """
    file_findings = [f for f in findings if f['rule'] == 'filesystem']
    if not file_findings or any(f['rule'] == 'coverage' for f in findings):
        return findings
    content, code = source_views(text, '.cs')
    filesystem_pattern = next(pattern for rule, _, pattern, _ in RULES if rule == 'filesystem')
    starts = [0] + [m.end() for m in re.finditer('\n', code)]
    operation_lines = [bisect_right(starts, match.start()) for match in filesystem_pattern.finditer(code)]
    if len(operation_lines) != len(set(operation_lines)) or len(operation_lines) != len(file_findings):
        return findings
    args = re.search(r'\b(?:string\s*\[\s*\]|var)\s+(\w+)\s*=\s*Environment\s*\.\s*GetCommandLineArgs\s*\(\s*\)', content)
    if not args:
        return findings
    arg_name = re.escape(args[1])
    flag = re.search(r'if\s*\(\s*' + arg_name + r'\s*\[\s*(\w+)\s*\]\s*==\s*"(--[a-zA-Z0-9-]+)"\s*\)', content)
    if not flag:
        return findings
    assignment = re.search(r'\b(?:\w+\s*\.\s*)?(\w+)\s*=\s*' + arg_name + r'\s*\[\s*' + re.escape(flag[1]) + r'\s*\+\s*1\s*\]', content[flag.end():])
    if not assignment:
        return findings
    output = re.escape(assignment[1])
    expected = re.compile(r'^(?:Directory\s*\.\s*CreateDirectory\s*\(\s*' + output + r'\s*\)|File\s*\.\s*WriteAll(?:Text|Lines|Bytes)\s*\(\s*Path\s*\.\s*Combine\s*\(\s*' + output + r'\s*,\s*(?:"[^"\\/:]+\.(?:txt|log|png|json|cfg)"|\w+\s*\+\s*"\.png")\s*\))')
    if not all(expected.search(f['evidence']) and not EXECUTABLE.search(f['evidence'].split('),', 1)[0]) for f in file_findings):
        return findings
    # A string variable name can hide path traversal. Always disclose that it
    # is operator selected and filenames/callers still need human inspection.
    first = file_findings[0]
    locations = [{'file': f['file'], 'line': f['line'], 'evidence': f['evidence']} for f in file_findings]
    key = 'diagnostic-output\0' + '\0'.join(f['id'] for f in file_findings)
    grouped = dict(first, id=hashlib.sha256(key.encode()).hexdigest(), rule='diagnostic-output',
                   title='Operator-selected diagnostic output', severity='review', locations=locations,
                   context=f"{len(locations)} directory/text/PNG operations share an output variable assigned from {flag[2]} command-line arguments. The destination is not confined to a trusted folder and existing files can be overwritten. Inspect the guard, all output-name construction and call sites before accepting. This grouping is not a safety decision.")
    return [f for f in findings if f['rule'] != 'filesystem'] + [grouped]
