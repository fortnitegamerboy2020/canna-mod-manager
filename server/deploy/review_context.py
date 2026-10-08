"""Bounded source heuristics, never a parser or a proof of safe behavior.

Comments and literal references are distinguished from API use. Unknown paths
still require review; non-executable extensions are not a trust boundary.
"""
import hashlib
import re
from bisect import bisect_right

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


def source_views(text, suffix, issues=None):
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
    content, code = source_views(text, suffix, issues)
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
            emit(entry(rule, title, name, index, evidence, severity))
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
    if not file_findings:
        return findings
    content, code = source_views(text, '.cs')
    filesystem_pattern = next(pattern for rule, _, pattern, _ in RULES if rule == 'filesystem')
    starts = [0] + [m.end() for m in re.finditer('\n', code)]
    operation_lines = [bisect_right(starts, match.start()) for match in filesystem_pattern.finditer(code)]
    if len(operation_lines) != len(set(operation_lines)):
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
