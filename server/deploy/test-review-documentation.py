#!/usr/bin/env python3
"""Canonical legal prose and disguised-code regressions; no submitted code runs."""
import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).parent


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


context = module('review_document_context', 'review_context.py')
fixture = module('review_document_fixture', 'test-review-adversarial.py')
LICENSES = ROOT / 'test-fixtures' / 'licenses'
GPL = (LICENSES / 'GPL-3.0.txt').read_text(encoding='utf-8-sig')
APACHE = (LICENSES / 'Apache-2.0.txt').read_text(encoding='utf-8-sig')
MIT = (LICENSES / 'MIT.txt').read_text(encoding='utf-8-sig')


class DocumentationTests(unittest.TestCase):
    def scan(self, text, suffix='.txt'):
        return context.scan_source(text, 'archive/LICENSE-GPL-3.0' + suffix, suffix)

    def test_actual_full_gpl_prose_is_not_unterminated_program_source(self):
        findings, observations = self.scan(GPL)
        self.assertFalse(findings)
        self.assertTrue(any(item['rule'] == 'documentation-reference' for item in observations))
        self.assertTrue(any(item['rule'] == 'url-reference' for item in observations))
        self.assertTrue(all('accepted' not in item for item in observations))

    def test_canonical_apache_and_mit_bodies_are_recognized(self):
        for text in (APACHE, MIT):
            with self.subTest(first=text[:25]):
                findings, observations = self.scan(text)
                self.assertFalse(findings)
                self.assertTrue(any(item['rule'] == 'documentation-reference' for item in observations))

    def test_bom_crlf_and_spacing_changes_preserve_only_canonical_prose(self):
        text = '\ufeff  ' + GPL.replace(' ', '  ').replace('\n', '\r\n')
        self.assertFalse(self.scan(text)[0])
        self.assertEqual(len(context.license_prose_regions(text, '.txt')), 1)

    def test_header_and_filename_are_insufficient_to_recognize_a_license(self):
        text = 'GNU GENERAL PUBLIC LICENSE\nVersion 3\nProcess.Start("fixture");'
        self.assertFalse(context.license_prose_regions(text, '.txt'))
        findings, _ = self.scan(text)
        self.assertIn('commands', {item['rule'] for item in findings})

    def test_changed_license_body_is_not_fingerprint_trusted(self):
        text = GPL.replace('Everyone is permitted', 'Process.Start("fixture"); Everyone is permitted', 1)
        self.assertFalse(context.license_prose_regions(text, '.txt'))
        findings, _ = self.scan(text)
        self.assertTrue(any(item['rule'] in {'commands', 'coverage'} for item in findings))

    def test_appended_csharp_is_inspected_with_original_lines_and_decisions_unset(self):
        findings, _ = self.scan(GPL + '\nProcess.Start("fixture");\nFile.Delete(path);')
        self.assertEqual({item['rule'] for item in findings}, {'commands', 'filesystem'})
        self.assertTrue(all(item['line'] > len(GPL.splitlines()) for item in findings))
        self.assertTrue(all('accepted' not in item for item in findings))

    def test_appended_powershell_aliases_and_case_are_still_inspected(self):
        findings, _ = self.scan(GPL + '\nstart-process "fixture"\nIRM https://example.invalid/\nIEX $payload')
        self.assertIn('commands', {item['rule'] for item in findings})
        self.assertIn('network', {item['rule'] for item in findings})

    def test_prepended_code_is_not_hidden_by_following_canonical_prose(self):
        findings, _ = self.scan('File.Delete(path);\n' + GPL)
        self.assertTrue(any(item['rule'] == 'filesystem' and item['line'] == 1 for item in findings))

    def test_incomplete_code_after_license_keeps_real_lexical_coverage_limit(self):
        findings, _ = self.scan(GPL + '\npublic class Fixture { string value = "unterminated')
        self.assertTrue(any(item['rule'] == 'coverage' and item['severity'] == 'high' for item in findings))

    def test_source_extension_never_receives_the_license_prose_exemption(self):
        self.assertFalse(context.license_prose_regions(GPL, '.cs'))
        self.assertTrue(any(item['rule'] == 'coverage' for item in self.scan(GPL, '.cs')[0]))

    def test_worker_scans_disguised_csharp_powershell_and_metadata_names(self):
        for name, text, rule in (
                ('LICENSE.txt', 'Process.Start("fixture");', 'commands'),
                ('LICENSE', 'File.Delete(path);', 'filesystem'),
                ('addoninfo.txt', 'Start-Process "fixture"', 'commands'),
                ('manifest.json', 'Process.Start("fixture");', 'commands'),
                ('README.md', 'public class X { void Run() { File.Delete(path); } }', 'filesystem'),
                ('README.md', 'IEX $payload', 'commands')):
            with self.subTest(name=name, rule=rule):
                report = fixture.analyze_fixture([(name, text)], fixture.ToolFixture())
                self.assertTrue(any(item['rule'] == rule for item in report['findings']))

    def test_worker_keeps_license_preview_but_not_false_lexical_finding(self):
        report = fixture.analyze_fixture([('patchers/LICENSE-GPL-3.0.txt', GPL)], fixture.ToolFixture())
        self.assertEqual(len(report['files']), 1)
        self.assertFalse(report['findings'])
        self.assertTrue(any(item['rule'] == 'documentation-reference' for item in report['observations']))

    def test_worker_pe_magic_still_precedes_license_text_names(self):
        report = fixture.analyze_fixture([('LICENSE-GPL-3.0.txt', fixture.inert_pe())], fixture.ToolFixture())
        self.assertTrue(any(item['kind'] == 'decompiled' for item in report['files']))
        self.assertTrue(any(item['rule'] == 'commands' for item in report['findings']))

    def test_reflection_decode_load_and_eval_keep_distinct_labels_and_review_binding(self):
        findings, _ = self.scan('Activator.CreateInstance(type);\nConvert.FromBase64String(data);\nAssembly.Load(bytes);\neval(expression);', '.cs')
        self.assertEqual(len(findings), 4)
        self.assertEqual({item['rule'] for item in findings}, {'dynamic'})
        self.assertEqual(len({item['title'] for item in findings}), 4)
        self.assertTrue(all(item['severity'] == 'review' and 'accepted' not in item for item in findings))
        self.assertTrue(any('value-type defaults' in item['context'] for item in findings))


if __name__ == '__main__':
    unittest.main(verbosity=2)
