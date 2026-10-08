#!/usr/bin/env python3
"""Coverage presentation regressions; no analyzer executable or mod runs."""
import copy
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    'review_worker', Path(__file__).with_name('review-worker.py'))
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)


def coverage(ident, file, line=None, title='Source preview limit reached',
             evidence='Code heuristics ran; this file is omitted from the preview.',
             severity='high', **extra):
    return dict(id=ident, rule='coverage', title=title, file=file, line=line,
                evidence=evidence, severity=severity, **extra)


def provenance(items):
    return {(item['id'], item['file'], item['line'], item['evidence']) for item in items}


class CoverageGroupingTests(unittest.TestCase):
    def test_many_preview_omissions_preserve_every_original_location_and_review_gate(self):
        omitted = [coverage(f'original-{i}', f'decompiled/assembly/Class{i}.cs', i + 1)
                   for i in range(422)]
        result = worker.group_coverage_findings(omitted)
        self.assertEqual(len(result), 1)
        group = result[0]
        self.assertEqual(group['rule'], 'coverage')
        self.assertEqual(group['severity'], 'high')
        self.assertIsNot(group.get('accepted'), True,
                         'Presentation grouping cannot resolve an incomplete review')
        self.assertEqual(len(group['locations']), len(omitted))
        self.assertEqual(provenance(group['locations']), provenance(omitted))

    def test_group_identity_and_location_order_do_not_depend_on_input_order(self):
        omitted = [coverage('c', 'decompiled/C.cs', 12),
                   coverage('a', 'decompiled/A.cs', 2),
                   coverage('b', 'decompiled/A.cs', 10)]
        expected = worker.group_coverage_findings(omitted)
        for ordered in (list(reversed(omitted)), [omitted[1], omitted[2], omitted[0]]):
            with self.subTest(ordered=[item['id'] for item in ordered]):
                self.assertEqual(worker.group_coverage_findings(ordered), expected)

    def test_new_omission_changes_group_identity(self):
        omitted = [coverage('a', 'decompiled/A.cs'), coverage('b', 'decompiled/B.cs')]
        first = worker.group_coverage_findings(omitted)[0]
        changed = worker.group_coverage_findings(omitted + [coverage('c', 'decompiled/C.cs')])[0]
        self.assertNotEqual(changed['id'], first['id'],
                            'New omitted content must require a fresh hash-bound decision')

    def test_changed_provenance_invalidates_identity_even_with_reused_original_id(self):
        omitted = [coverage('a', 'decompiled/A.cs', 1), coverage('b', 'decompiled/B.cs', 2)]
        first = worker.group_coverage_findings(omitted)[0]
        for field, value in (('file', 'decompiled/Changed.cs'), ('line', 99),
                             ('evidence', 'The analyzer failed rather than merely omitting a preview.')):
            with self.subTest(field=field):
                changed = copy.deepcopy(omitted)
                changed[0][field] = value
                group = worker.group_coverage_findings(changed)[0]
                self.assertNotEqual(group['id'], first['id'])
                self.assertEqual(provenance(group['locations']), provenance(changed))

    def test_errors_are_not_merged_with_unrelated_coverage_reasons(self):
        omitted = [coverage('preview-a', 'decompiled/A.cs'), coverage('preview-b', 'decompiled/B.cs')]
        malformed = [coverage('malformed-a', 'archive/A.dll',
                              title='PE packing analysis could not inspect malformed headers',
                              evidence='Section data is truncated.'),
                     coverage('malformed-b', 'archive/B.dll',
                              title='PE packing analysis could not inspect malformed headers',
                              evidence='Section data overlaps headers.')]
        failed = [coverage('die-a', 'archive/A.dll',
                           title='Packer signature scanner failed or timed out',
                           evidence='Analyzer timeout.'),
                  coverage('die-b', 'archive/B.dll',
                           title='Packer signature scanner failed or timed out',
                           evidence='Malformed detector JSON.')]
        result = worker.group_coverage_findings(omitted + malformed + failed)
        self.assertEqual(len(result), 3)
        for originals in (omitted, malformed, failed):
            group = next(item for item in result if item['title'] == originals[0]['title'])
            self.assertEqual(provenance(group['locations']), provenance(originals))
            self.assertEqual(group['severity'], 'high')

    def test_different_severities_are_never_collapsed(self):
        high = [coverage('high-a', 'archive/A.bin'), coverage('high-b', 'archive/B.bin')]
        review = [coverage('review-a', 'archive/A.data', severity='review'),
                  coverage('review-b', 'archive/B.data', severity='review')]
        result = worker.group_coverage_findings(high + review)
        self.assertEqual(len(result), 2)
        self.assertEqual({item['severity'] for item in result}, {'high', 'review'})
        for originals in (high, review):
            group = next(item for item in result if item['severity'] == originals[0]['severity'])
            self.assertEqual(provenance(group['locations']), provenance(originals))

    def test_malware_other_risk_and_finding_limit_sentinel_survive_unchanged(self):
        omitted = [coverage('a', 'decompiled/A.cs'), coverage('b', 'decompiled/B.cs')]
        malware = dict(id='malware', rule='signature', title=omitted[0]['title'],
                       severity='critical', file='archive/plugin.dll', line=None,
                       evidence='TestFixture.Signature')
        risk = dict(id='sensitive', rule='sensitive-files', title='Credential paths',
                    severity='high', file='decompiled/Plugin.cs', line=7,
                    evidence='File.ReadAllText(browserPath);')
        limit = coverage('finding-limit', None,
                         title='Finding limit reached', evidence='Further findings were omitted.')
        result = worker.group_coverage_findings([omitted[0], malware, limit, risk, omitted[1]])
        self.assertEqual(len(result), 4)
        for original in (malware, limit, risk):
            self.assertEqual(next(item for item in result if item['id'] == original['id']), original)
        self.assertNotIn('locations', next(item for item in result if item['id'] == 'finding-limit'))

    def test_grouping_does_not_copy_old_decisions_or_modify_inputs(self):
        omitted = [coverage('a', 'decompiled/A.cs', accepted=True, reason='Old acceptance',
                            reviewer=1, reviewed=10),
                   coverage('b', 'decompiled/B.cs', accepted=False, reason='Reopened',
                            reviewer=2, reviewed=20)]
        snapshot = copy.deepcopy(omitted)
        group = worker.group_coverage_findings(omitted)[0]
        self.assertEqual(omitted, snapshot)
        for field in ('accepted', 'reason', 'reviewer', 'reviewed'):
            self.assertNotIn(field, group)
            self.assertTrue(all(field not in location for location in group['locations']))

    def test_existing_group_and_single_error_remain_idempotent(self):
        omitted = [coverage('a', 'decompiled/A.cs'), coverage('b', 'decompiled/B.cs')]
        single = coverage('single', 'archive/plugin.dll', title='Archive analysis incomplete',
                          evidence='Checksum mismatch.')
        first = worker.group_coverage_findings(omitted + [single])
        self.assertEqual(worker.group_coverage_findings(first), first)
        self.assertEqual(next(item for item in first if item['id'] == 'single'), single)


if __name__ == '__main__':
    unittest.main(verbosity=2)
