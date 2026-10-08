#!/usr/bin/env python3
"""Bounded decompilation/provenance integration using inert fixtures only.

No submitted code or analyzer executable runs. The real-tools counterpart is
test-review-offline.py --fixture, run separately in an isolated Linux directory.
"""
import hashlib
import importlib.util
import os
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    'review_adversarial', Path(__file__).with_name('test-review-adversarial.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)
worker = fixtures.worker


class SourceTools(fixtures.ToolFixture):
    def __init__(self, *, sources=None, code=0, log=b'', error=None,
                 symbolic_link=None):
        super().__init__(reconstructed='public class Fixture {}')
        self.sources = sources or {'Fixture.cs': 'public class Fixture {}'}
        self.code = code
        self.log = log
        self.error = error
        self.symbolic_link = symbolic_link

    def popen(self, args, stdout, **kwargs):
        tool = Path(args[0]).name
        if tool not in ('ilspycmd', 'java'):
            return super().popen(args, stdout, **kwargs)
        self.calls.append(args)
        if self.error:
            raise self.error
        output = Path(args[args.index('-o' if tool == 'ilspycmd' else '--outputdir') + 1])
        for name, source in self.sources.items():
            path = output / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(source, encoding='utf-8')
        if self.symbolic_link:
            os.symlink(self.symbolic_link, output / 'Escaped.cs')
        stdout.write(self.log)
        stdout.flush()
        code = self.code

        class Process:
            pid = -1

            def wait(self, timeout=None):
                return code

        return Process()


class DecompilationTests(unittest.TestCase):
    def test_exact_binary_origin_and_generated_source_integrity(self):
        tools = SourceTools(sources={'Namespace/Fixture.cs': 'public class Fixture {}'})
        report = fixtures.analyze_fixture([('plugins/A.dll', fixtures.inert_pe())], tools)
        record = report['decompilations'][0]
        source = report['files'][0]
        self.assertEqual(report['version'], 'canna-static-8')
        self.assertEqual(record['input'], 'archive/plugins/A.dll')
        self.assertEqual(record['scope'], 'binary')
        self.assertEqual(record['tool'], 'ilspycmd')
        self.assertEqual(record['status'], 'complete')
        self.assertEqual(record['generated_files'], [source['name']])
        self.assertEqual(record['generated_count'], record['preview_count'])
        self.assertEqual(record['generated_count'], record['scanned_count'])
        self.assertEqual(source['origin'], record['input'])
        self.assertEqual(source['language'], 'C#')
        self.assertEqual(source['sha256'], hashlib.sha256(source['text'].encode()).hexdigest())
        self.assertEqual(source['byte_size'], len(source['text'].encode()))
        self.assertGreaterEqual(record['duration_ms'], 0)
        call = next(args for args in tools.calls if Path(args[0]).name == 'ilspycmd')
        self.assertEqual(Path(call[call.index('-r') + 1]).name, 'plugins')
        self.assertIs(record['dependency_resolution']['external_downloads'], False)
        self.assertIn('does not provide', record['dependency_resolution']['verification'])

    def test_same_binary_basename_keeps_distinct_origins(self):
        report = fixtures.analyze_fixture([
            ('one/Same.dll', fixtures.inert_pe()),
            ('two/Same.dll', fixtures.inert_pe())], SourceTools())
        self.assertEqual({record['input'] for record in report['decompilations']},
                         {'archive/one/Same.dll', 'archive/two/Same.dll'})
        self.assertEqual(len({source['name'] for source in report['files']}), 2)
        self.assertEqual({source['origin'] for source in report['files']},
                         {'archive/one/Same.dll', 'archive/two/Same.dll'})

    def test_nonzero_exit_retains_partial_source_without_clean_status(self):
        tools = SourceTools(code=7, log=b'fixture dependency could not be resolved')
        report = fixtures.analyze_fixture([('A.dll', fixtures.inert_pe())], tools)
        record = report['decompilations'][0]
        self.assertEqual(record['status'], 'incomplete')
        self.assertEqual(record['exit_code'], 7)
        self.assertEqual(record['preview_count'], 1)
        self.assertIn('could not be resolved', record['diagnostic'])
        self.assertEqual(report['engines']['ilspycmd']['status'], 'incomplete')
        self.assertIs(report['coverage_complete'], False)

    def test_output_log_limit_still_retains_safe_generated_source(self):
        tools = SourceTools(log=b'x' * (256 * 1024 + 1))
        report = fixtures.analyze_fixture([('A.dll', fixtures.inert_pe())], tools)
        record = report['decompilations'][0]
        self.assertEqual(record['status'], 'limited')
        self.assertEqual(record['preview_count'], 1)
        self.assertTrue(any('output limit' in item for item in record['limitations']))
        self.assertIs(report['coverage_complete'], False)

    def test_missing_tool_is_visible_per_binary(self):
        tools = SourceTools(error=FileNotFoundError('fixture tool unavailable'))
        report = fixtures.analyze_fixture([('A.dll', fixtures.inert_pe())], tools)
        record = report['decompilations'][0]
        self.assertEqual(record['status'], 'unavailable')
        self.assertEqual(record['generated_count'], 0)
        self.assertIsNone(record['exit_code'])
        self.assertEqual(report['engines']['ilspycmd']['status'], 'error')
        self.assertIs(report['coverage_complete'], False)

    def test_native_source_omission_is_explicit(self):
        report = fixtures.analyze_fixture([('image.png', b'\x7fELF inert fixture')], SourceTools())
        record = report['decompilations'][0]
        self.assertEqual(record['input'], 'archive/image.png')
        self.assertEqual(record['status'], 'not-supported')
        self.assertEqual(record['generated_count'], 0)
        self.assertIsNone(record['tool'])
        self.assertIs(report['coverage_complete'], False)

    def test_oversized_candidate_retains_inventory_and_unverified_omission(self):
        report = fixtures.analyze_fixture([('Large.dll', b'0' * (32 * 1024 * 1024 + 1))], SourceTools())
        record = report['decompilations'][0]
        self.assertEqual(record['input'], 'archive/Large.dll')
        self.assertEqual(record['language'], 'Unknown')
        self.assertEqual(record['status'], 'limited')
        self.assertEqual(report['inventory'][0]['analysis'], 'entry-size-limit')
        self.assertEqual(report['files'], [])
        self.assertIs(report['coverage_complete'], False)

    def test_standalone_limit_keeps_per_binary_omissions(self):
        report = fixtures.analyze_fixture(
            [(f'{i:02d}.dll', fixtures.inert_pe()) for i in range(18)], SourceTools())
        records = report['decompilations']
        self.assertEqual(len(records), 18)
        self.assertEqual(sum(record['status'] == 'complete' for record in records), 16)
        self.assertEqual(sum(record['status'] == 'limited' for record in records), 2)
        self.assertEqual(report['engines']['ilspycmd']['attempted'], 16)
        self.assertIs(report['coverage_complete'], False)

    def test_source_omission_reports_generated_but_unread_source(self):
        report = fixtures.analyze_fixture([('A.dll', fixtures.inert_pe())],
                                          SourceTools(sources={'Huge.cs': 'x' * (1024 * 1024 + 1)}))
        record = report['decompilations'][0]
        self.assertEqual(record['generated_count'], 1)
        self.assertEqual(record['preview_count'], 0)
        self.assertEqual(record['scanned_count'], 0)
        self.assertEqual(record['preview_omitted_count'], 1)
        self.assertEqual(record['scan_omitted_count'], 1)
        self.assertEqual(record['status'], 'incomplete')
        self.assertEqual(report['files'], [])

    def test_preview_limit_does_not_claim_unscanned_source(self):
        sources = {f'{i:03d}.cs': 'public class Fixture {}' for i in range(502)}
        report = fixtures.analyze_fixture([('A.dll', fixtures.inert_pe())], SourceTools(sources=sources))
        record = report['decompilations'][0]
        self.assertEqual(record['generated_count'], 502)
        self.assertEqual(record['preview_count'], 500)
        self.assertEqual(record['preview_omitted_count'], 2)
        self.assertEqual(record['scanned_count'], 502)
        self.assertEqual(record['scan_omitted_count'], 0)
        self.assertEqual(record['status'], 'incomplete')

    def test_project_provenance_does_not_invent_per_class_linkage(self):
        report = fixtures.analyze_fixture([
            ('pkg/One.class', b'\xca\xfe\xba\xbe inert fixture'),
            ('pkg/Two.class', b'\xca\xfe\xba\xbe inert fixture')],
            SourceTools(sources={'pkg/One.java': 'public class One {}'}))
        record = report['decompilations'][0]
        self.assertEqual(record['scope'], 'project')
        self.assertEqual(record['inputs'], ['archive/pkg/One.class', 'archive/pkg/Two.class'])
        self.assertEqual(report['files'][0]['origin'], 'archive/')
        self.assertEqual(report['files'][0]['origins'], record['inputs'])
        self.assertIn('does not establish', record['mapping_note'])

    def test_pe_renamed_class_is_not_skipped_by_java_project(self):
        tools = SourceTools(sources={'Fixture.java': 'public class Fixture {}'})
        report = fixtures.analyze_fixture([
            ('Actual.class', b'\xca\xfe\xba\xbe inert fixture'),
            ('Disguised.class', fixtures.inert_pe())], tools)
        managed = next(record for record in report['decompilations']
                       if record['input'] == 'archive/Disguised.class')
        self.assertEqual(managed['tool'], 'ilspycmd')
        self.assertTrue(any(Path(args[0]).name == 'ilspycmd' for args in tools.calls))
        project = next(record for record in report['decompilations'] if record['scope'] == 'project')
        self.assertEqual(project['inputs'], ['archive/Actual.class'])

    def test_symlink_output_is_not_read(self):
        with tempfile.TemporaryDirectory(prefix='canna-decomp-symlink-') as tmp:
            outside = Path(tmp) / 'Private.cs'
            outside.write_text('private sentinel', encoding='utf-8')
            try:
                probe = Path(tmp) / 'probe'
                os.symlink(outside, probe)
                probe.unlink()
            except OSError:
                self.skipTest('Creating symlinks requires a privileged Windows account; Linux test required.')
            report = fixtures.analyze_fixture([('A.dll', fixtures.inert_pe())],
                                              SourceTools(symbolic_link=outside))
        self.assertEqual(report['decompilations'][0]['status'], 'failed')
        self.assertEqual(report['files'], [])
        self.assertNotIn('private sentinel', str(report))
        self.assertIs(report['coverage_complete'], False)

    def test_case_colliding_output_is_not_read(self):
        with tempfile.TemporaryDirectory(prefix='canna-case-sensitive-probe-') as tmp:
            (Path(tmp) / 'A').write_text('a')
            (Path(tmp) / 'a').write_text('b')
            if len(list(Path(tmp).iterdir())) != 2:
                self.skipTest('Case-sensitive decompiler output test requires Linux filesystem.')
        tools = SourceTools(sources={'Fixture.cs': 'public class Upper {}',
                                     'fixture.cs': 'public class Lower {}'})
        report = fixtures.analyze_fixture([('A.dll', fixtures.inert_pe())], tools)
        self.assertEqual(report['decompilations'][0]['status'], 'failed')
        self.assertEqual(report['files'], [])
        self.assertTrue(any('collision' in limitation
                            for limitation in report['decompilations'][0]['limitations']))

    def test_unreadable_output_directory_cannot_be_silently_omitted(self):
        with tempfile.TemporaryDirectory(prefix='canna-decomp-unreadable-') as tmp:
            output = Path(tmp)
            hidden = output / 'Hidden'
            hidden.mkdir()
            (hidden / 'Hidden.cs').write_text('public class Hidden {}')
            (output / 'Visible.cs').write_text('public class Visible {}')
            hidden.chmod(0)
            try:
                if os.access(hidden, os.R_OK):
                    self.skipTest('Unreadable-directory fixture requires an unprivileged Linux user.')
                with self.assertRaises(OSError):
                    worker.generated_sources(output)
            finally:
                hidden.chmod(0o700)

    def test_instant_exit_cannot_bypass_output_entry_limit(self):
        with tempfile.TemporaryDirectory(prefix='canna-decomp-budget-') as tmp:
            job = Path(tmp)
            with zipfile.ZipFile(job / 'input.zip', 'w') as archive:
                archive.writestr('A.dll', fixtures.inert_pe())
            tools = SourceTools(sources={f'{i}.cs': 'public class Fixture {}' for i in range(6001)})
            with patch.object(worker.subprocess, 'Popen', tools.popen), \
                 patch.object(worker.subprocess, 'check_output', return_value=b'fixture-version'):
                report = worker.analyze(job)
        self.assertEqual(report['decompilations'][0]['status'], 'failed')
        self.assertEqual(report['files'], [])
        self.assertIs(report['coverage_complete'], False)


if __name__ == '__main__':
    unittest.main(verbosity=2)
