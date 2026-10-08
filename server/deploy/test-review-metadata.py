#!/usr/bin/env python3
"""Inert, bounded CLR metadata hints; packing findings always remain actionable."""
import importlib.util
import struct
import unittest
from pathlib import Path

ROOT = Path(__file__).parent
spec = importlib.util.spec_from_file_location('packing_metadata_fixture', ROOT / 'test-review-packing.py')
packing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packing)
worker = packing.worker


def managed(names=b'Fixture.compat.bsdf\0', heap=None):
    heap = b'\0' + names if heap is None else heap
    metadata = bytearray(64)
    metadata[:4] = b'BSJB'
    struct.pack_into('<I', metadata, 12, 12)
    metadata[16:28] = b'v4.0.30319\0\0'
    struct.pack_into('<H', metadata, 30, 1)
    struct.pack_into('<II', metadata, 32, 64, len(heap))
    metadata[40:49] = b'#Strings\0'
    metadata.extend(heap)
    raw = bytearray(bytes(range(256)) * ((max(32768, 256 + len(metadata)) + 255) // 256))
    struct.pack_into('<I', raw, 16, 72)
    struct.pack_into('<II', raw, 24, 0x1100, len(metadata))
    raw[256:256 + len(metadata)] = metadata
    data = bytearray(packing.pe([('.text', bytes(raw), packing.READ | packing.EXEC, None)]))
    struct.pack_into('<II', data, 128 + 24 + 96 + 14 * 8, 0x1010, 72)
    return data


class MetadataTests(unittest.TestCase):
    def test_patch_names_are_untrusted_hints_and_entropy_remains_review(self):
        data = managed()
        result = worker.managed_payload_metadata(data)
        self.assertEqual(result['payload_name_hints'], ['Fixture.compat.bsdf'])
        self.assertEqual(result['status'], 'complete')
        self.assertIn('not verified', result['note'])
        self.assertIn('packing-review', {item[0] for item in worker.packing_evidence(data)})

    def test_random_binary_names_do_not_become_clr_resource_hints(self):
        self.assertIsNone(worker.managed_payload_metadata(b'Asset Fixture.compat.bsdf\0'))
        data = packing.pe([('.text', b'Fixture.compat.bsdf\0' * 80, packing.READ | packing.EXEC, None)])
        self.assertIsNone(worker.managed_payload_metadata(data))

    def test_names_and_string_heap_work_have_explicit_bounds(self):
        many = b''.join(f'fixture-{index}.bsdf\0'.encode() for index in range(90))
        result = worker.managed_payload_metadata(managed(many))
        self.assertEqual(len(result['payload_name_hints']), 64)
        self.assertTrue(result['name_limit_reached'])
        result = worker.managed_payload_metadata(managed(heap=b'\0' * (1024 * 1024 + 16)))
        self.assertEqual(result['heap_bytes_inspected'], 1024 * 1024)
        self.assertTrue(result['heap_limit_reached'])
        self.assertEqual(result['status'], 'limited')

    def test_truncated_or_out_of_range_cli_metadata_is_unavailable_not_trusted(self):
        for field, value in ((128 + 24 + 96 + 14 * 8, 0xfffffff0),):
            data = managed()
            struct.pack_into('<I', data, field, value)
            result = worker.managed_payload_metadata(data)
            self.assertEqual(result['status'], 'unavailable')
            self.assertFalse(result['payload_name_hints'])
            self.assertIn('packing-review', {item[0] for item in worker.packing_evidence(data)})

    def test_stream_count_and_overlap_are_rejected(self):
        for offset, value, format_string in ((30, 33, '<H'), (32, 0, '<I')):
            data = managed()
            layout = worker.pe_packing_layout(data)
            metadata_offset = layout['sections'][0]['offset'] + 256
            struct.pack_into(format_string, data, metadata_offset + offset, value)
            result = worker.managed_payload_metadata(data)
            self.assertEqual(result['status'], 'unavailable')
            self.assertFalse(result['payload_name_hints'])

    def test_partial_or_nonliteral_names_are_not_reported(self):
        data = managed(heap=b'\0<script>.bsdf\0valid.bsdf\0unterminated.bsdf')
        self.assertEqual(worker.managed_payload_metadata(data)['payload_name_hints'], ['valid.bsdf'])

    def test_invalid_pe_headers_do_not_supply_metadata_context(self):
        self.assertIsNone(worker.managed_payload_metadata(b'MZ'))
        self.assertTrue(any(item[0] == 'coverage' for item in worker.packing_evidence(b'MZ')))


if __name__ == '__main__':
    unittest.main(verbosity=2)
