#!/usr/bin/env python3
"""Bounded, non-executing PE/DiE fixtures for packing false positives and evasion."""
import importlib.util
import struct
import unittest
from pathlib import Path

spec=importlib.util.spec_from_file_location('review_worker',Path(__file__).with_name('review-worker.py'))
worker=importlib.util.module_from_spec(spec);spec.loader.exec_module(worker)

READ=0x40000000
EXEC=0x20000000
WRITE=0x80000000

def pe(sections,entry_section=0,magic=0x10b,resource_section=None):
 """Make inert PE bytes, with optional/section metadata sufficient for analysis."""
 header=128;optional=224 if magic==0x10b else 240;opt=header+24
 end=opt+optional+40*len(sections);raw_start=(end+511)//512*512
 data=bytearray(raw_start);data[:2]=b'MZ';struct.pack_into('<I',data,60,header)
 data[header:header+4]=b'PE\0\0';struct.pack_into('<HH',data,header+4,0x14c,len(sections))
 struct.pack_into('<H',data,header+20,optional);struct.pack_into('<H',data,opt,magic)
 minimum=96 if magic==0x10b else 112;struct.pack_into('<I',data,opt+minimum-4,16)
 resource=None;rva=0x1000
 for index,(name,raw,flags,virtual) in enumerate(sections):
  start=opt+optional+40*index;data[start:start+8]=name.encode().ljust(8,b'\0')[:8]
  offset=len(data) if raw else 0;virtual=virtual if virtual is not None else len(raw)
  struct.pack_into('<IIII',data,start+8,virtual,rva,len(raw),offset);struct.pack_into('<I',data,start+36,flags)
  if index==entry_section:struct.pack_into('<I',data,opt+16,rva)
  if index==resource_section:resource=(rva,len(raw))
  data.extend(raw);data.extend(b'\0'*((-len(data))%512));rva+=(max(virtual,len(raw))+4095)//4096*4096 or 4096
 if resource:struct.pack_into('<II',data,opt+minimum+16,*resource)
 return bytes(data)

def findings(data):return worker.packing_evidence(data)
def rules(data):return {f[0] for f in findings(data)}

class MarkerTests(unittest.TestCase):
 def test_compression_is_not_mpress(self):
  self.assertEqual(findings(b'Compression Compressed System.IO.Compression'),[])

 def test_product_names_are_contextual_hints(self):
  for marker in (b'UPX!',b'MPRESS',b'VMProtect',b'Themida',b'ConfusedByAttribute',b'Obfuscar'):
   with self.subTest(marker=marker):
    result=findings(b'Asset documentation: '+marker+b'\0')
    self.assertTrue(result);self.assertEqual({f[0] for f in result},{'packing-review'})
    self.assertIn('alone does not establish',result[0][2])

 def test_identifier_substrings_are_not_markers(self):
  self.assertEqual(findings(b'MPRESSion XVMProtect VMProtectX MyObfuscarName'),[])

 def test_resource_marker_kept_with_context(self):
  data=pe([('.text',b'\x90'*1024,READ|EXEC,None),('.rsrc',b'Asset: Themida\0',READ,None)],resource_section=1)
  result=findings(data);self.assertEqual({f[0] for f in result},{'packing-review'})
  self.assertIn('non-executable',result[0][2]);self.assertIn('resource-directory range',result[0][2])

 def test_overlay_marker_is_not_confirmed_signature(self):
  data=pe([('.text',b'\x90'*1024,READ|EXEC,None)])+b'VMProtect\0'
  self.assertIn('unclassified',findings(data)[0][2]);self.assertNotIn('packer-marker',rules(data))

 def test_early_resource_token_does_not_hide_later_executable_token(self):
  data=pe([('.rsrc',b'Asset: Themida\0',READ,None),('.text',b'\x90'*1024+b' Themida\0',READ|EXEC,None)],entry_section=1,resource_section=0)
  result=findings(data);self.assertEqual(len(result),2)
  self.assertTrue(any('(executable section)' in f[2] for f in result))

 def test_section_name_alone_is_not_confirmed_signature(self):
  data=pe([('UPX0',b'',READ|EXEC,0x20000),('UPX1',b'\x90'*1024,READ|EXEC,None)],entry_section=1)
  self.assertIn('packing-review',rules(data));self.assertNotIn('packer-marker',rules(data))

 def test_upx_marker_without_conventional_layout_stays_review(self):
  data=pe([('.text',b'UPX!'+b'\x90'*1024,READ|EXEC,None)])
  self.assertEqual(rules(data),{'packing-review'})

 def test_upx_layout_magic_and_executable_entry_are_stronger(self):
  data=pe([('UPX0',b'',READ|EXEC|WRITE,0x20000),('UPX1',b'\x90'*16+b'UPX!'+b'\x90'*1004,READ|EXEC,None)],entry_section=1)
  self.assertIn('packer-marker',rules(data));self.assertIn('packing-review',rules(data))
  self.assertIn('does not establish malware',next(f[2] for f in findings(data) if f[0]=='packer-marker'))

 def test_upx_magic_in_resources_does_not_confirm_code_packing(self):
  data=pe([('UPX0',b'',READ|EXEC,0x20000),('UPX1',b'\x90'*1024,READ|EXEC,None),('.rsrc',b'UPX!',READ,None)],entry_section=1,resource_section=2)
  self.assertNotIn('packer-marker',rules(data));self.assertIn('packing-review',rules(data))

class SectionTests(unittest.TestCase):
 def test_data_entropy_is_not_executable_packing(self):
  data=pe([('.text',b'\x90'*8192,READ|EXEC,None),('.rsrc',bytes(range(256))*128,READ,None)],resource_section=1)
  self.assertEqual(findings(data),[])

 def test_entropy_in_executable_resource_named_section_remains_review(self):
  data=pe([('.rsrc',bytes(range(256))*128,READ|EXEC,None)],resource_section=0)
  self.assertEqual(rules(data),{'packing-review'});self.assertIn('compressed assets',findings(data)[0][2])

 def test_low_entropy_prefix_does_not_hide_high_entropy_tail(self):
  data=pe([('.text',b'\x90'*131072+bytes(range(256))*256,READ|EXEC,None)])
  self.assertTrue(any('entropy' in f[1] for f in findings(data)))

 def test_low_entropy_edges_do_not_hide_high_entropy_middle(self):
  data=pe([('.text',b'\x90'*65536+bytes(range(256))*256+b'\x90'*65536,READ|EXEC,None)])
  self.assertTrue(any('entropy' in f[1] for f in findings(data)))

 def test_rwx_is_preserved_even_for_resource_section(self):
  data=pe([('.rsrc',b'\0'*8192,READ|EXEC|WRITE,None)],resource_section=0)
  self.assertTrue(any(f[1]=='Writable and executable section' for f in findings(data)))

 def test_expanded_entry_section_is_preserved(self):
  data=pe([('.text',b'\x90'*512,READ|EXEC,0x30000)])
  self.assertTrue(any('entry-point' in f[1] for f in findings(data)))

 def test_pe32_plus_is_supported(self):
  data=pe([('.text',bytes(range(256))*32,READ|EXEC,None)],magic=0x20b)
  self.assertEqual(rules(data),{'packing-review'})

 def test_zero_entry_does_not_invent_an_expanded_entry_finding(self):
  data=bytearray(pe([('.text',b'\x90'*512,READ|EXEC,0x30000)]));struct.pack_into('<I',data,128+24+16,0)
  self.assertEqual(findings(data),[])

class MalformedTests(unittest.TestCase):
 def test_truncated_headers_are_coverage_limits(self):
  for data in (b'MZ',b'MZ'+b'\0'*62,pe([('.text',b'\x90'*8192,READ|EXEC,None)])[:-1000]):
   with self.subTest(size=len(data)):self.assertIn('coverage',rules(data))

 def test_invalid_optional_header_does_not_read_section_bytes_as_entry(self):
  data=bytearray(pe([('.text',b'\x90'*8192,READ|EXEC,None)]));struct.pack_into('<H',data,128+24,0)
  self.assertEqual(rules(data),{'coverage'})

 def test_section_offset_in_headers_is_invalid(self):
  data=bytearray(pe([('.text',b'\x90'*8192,READ|EXEC,None)]));struct.pack_into('<I',data,128+24+224+20,2)
  self.assertEqual(rules(data),{'coverage'})

 def test_overlapping_sections_do_not_provide_trusted_context(self):
  data=bytearray(pe([('.text',b'\x90'*8192,READ|EXEC,None),('.rsrc',b'Themida\0',READ,None)]))
  table=128+24+224;offset=struct.unpack_from('<I',data,table+20)[0];struct.pack_into('<I',data,table+40+20,offset)
  self.assertIn('coverage',rules(data));self.assertNotIn('resource-directory range',' '.join(f[2] for f in findings(data)))

 def test_oversized_section_count_is_coverage_limit(self):
  data=bytearray(pe([('.text',b'\x90'*8192,READ|EXEC,None)]));struct.pack_into('<H',data,128+6,97)
  self.assertEqual(rules(data),{'coverage'})

 def test_address_overflow_is_coverage_limit(self):
  data=bytearray(pe([('.text',b'\x90'*8192,READ|EXEC,None)]));struct.pack_into('<I',data,128+24+224+12,0xfffff000)
  self.assertEqual(rules(data),{'coverage'})

class DieTests(unittest.TestCase):
 def test_compiler_library_package_words_do_not_create_packing_findings(self):
  for value in ({'type':'Compiler','string':'Microsoft Visual C/C++ [stack protection]'}, {'type':'Compiler','string':'Protection: stack protection'}, {'type':'Library','string':'Package utility'}, {'type':'Library','string':'VMProtect integration library'}, {'type':'Language','string':'C# protected members'}, {'string':'unpacked package'}):
   with self.subTest(value=value):self.assertIsNone(worker.die_packing_finding(value))

 def test_role_detections_are_preserved(self):
  for kind in ('Packer','Protector','Protection','Obfuscator','Obfuscation','Cryptor','Crypter','Virtualizer','Virtualization'):
   with self.subTest(kind=kind):self.assertEqual(worker.die_packing_finding({'type':kind,'string':kind+': NamedTool'})[0],'packer-signature')

 def test_generic_and_heuristic_type_are_review(self):
  for value in ({'type':'Packer','string':'Generic'}, {'type':'(Heur)Packer','string':'SomeTool'}, {'type':'Packer (Heur)','string':'SomeTool'}, {'string':'(Heur)Packer: Generic'}, {'type':'Protection','string':'Anti analysis'}, {'type':'Packer','name':'Generic','string':'SomeTool'}, {'type':'Packer','string':'UPX','heuristic':True}, {'type':'Packer'}, {'type':'Packer','string':'Packer:'}):
   with self.subTest(value=value):self.assertEqual(worker.die_packing_finding(value)[0],'packing-review')

 def test_hollowpurple_bare_generic_remains_unresolved_review(self):
  value={'type':'Packer','string':'(Heur)Packer: Generic'}
  for data in (None,pe([('.text',b'readable source',READ|EXEC,None)]),pe([('.rsrc',bytes(range(256))*128,READ,None)],entry_section=None,resource_section=0)):
   self.assertEqual(worker.die_packing_finding(value,data)[0],'packing-review')

 def test_resource_only_compression_does_not_suppress_generic(self):
  data=pe([('.text',b'\x90'*8192,READ|EXEC,None),('.rsrc',bytes(range(256))*128,READ,None)],resource_section=1)
  value={'type':'Packer','string':'(Heur)Packer: Generic [Section #1 (".rsrc") compressed + High entropy]'}
  self.assertEqual(worker.die_packing_finding(value,data)[0],'packing-review')

 def test_specific_signature_is_not_downgraded_by_resources(self):
  data=pe([('.rsrc',bytes(range(256))*128,READ,None)],entry_section=None,resource_section=0)
  self.assertEqual(worker.die_packing_finding({'type':'Packer','string':'UPX(4.2)'},data)[0],'packer-signature')

 def test_missing_strings_do_not_crash_classifier(self):
  for value in (None,[],{'type':None},{'string':42},{'type':'Packer','name':{}},'Packer: Generic'):
   with self.subTest(value=value):self.assertIsNone(worker.die_packing_finding(value))

if __name__=='__main__':unittest.main()
