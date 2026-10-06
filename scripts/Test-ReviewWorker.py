import importlib.util, tempfile, zipfile
from pathlib import Path
spec=importlib.util.spec_from_file_location('worker','server/deploy/review-worker.py');w=importlib.util.module_from_spec(spec);spec.loader.exec_module(w)
with tempfile.TemporaryDirectory() as tmp:
 p=Path(tmp)
 with zipfile.ZipFile(p/'input.zip','w') as z:z.writestr('../escape.cs','File.Delete("test");')
 report=w.analyze(p);assert any('Unsafe archive' in f['evidence'] for f in report['findings']);assert not (p.parent/'escape.cs').exists()
with tempfile.TemporaryDirectory() as tmp:
 p=Path(tmp)
 with zipfile.ZipFile(p/'input.zip','w') as z:z.writestr('Source.cs','// harmless review fixture\nProcess.Start("cmd.exe");\nvar client = new HttpClient();\nFile.ReadAllText("sample");')
 report=w.analyze(p)
 assert any(f['rule']=='commands' and f['line']==2 for f in report['findings'])
 assert any(f['rule']=='network' and f['line']==3 for f in report['findings'])
 assert any(f['rule']=='filesystem' and f['line']==4 for f in report['findings'])
 assert report['files'][0]['name']=='archive/Source.cs'
print('Path traversal rejection and suspicious code file/line findings passed.')

import struct
sample=bytearray(1024+65536);sample[:2]=b'MZ';struct.pack_into('<I',sample,60,64);sample[64:68]=b'PE\0\0';struct.pack_into('<H',sample,70,1);struct.pack_into('<H',sample,84,224);struct.pack_into('<I',sample,104,4096)
start=64+24+224;sample[start:start+8]=b'changed\0';struct.pack_into('<IIII',sample,start+8,65536,4096,65536,1024);struct.pack_into('<I',sample,start+36,0xe0000020);sample[1024:]=bytes(range(256))*256
findings=w.packing_evidence(sample);assert any(f[0]=='packer-heuristic' and 'entropy' in f[2] for f in findings);assert any('Writable' in f[1] for f in findings)
assert any(f[0]=='packer-marker' for f in w.packing_evidence(b'MZ'+b'UPX!'))
print('Known packing markers and unknown high-entropy/RWX PE structure passed.')

# Self-contained VPK fixtures verify extraction, CRC and path boundaries.
import zlib
def vpk(name='fixture',directory='cfg',content=b'echo original'):
 tree=b'cfg\0'+directory.encode()+b'\0'+name.encode()+b'\0'+struct.pack('<IHHIIH',zlib.crc32(content),0,0x7fff,0,len(content),0xffff)+b'\0\0\0'
 return struct.pack('<III',0x55aa1234,1,len(tree))+tree+content
with tempfile.TemporaryDirectory() as tmp:
 root=Path(tmp);packed=root/'addon.vpk';packed.write_bytes(vpk());out=root/'expanded'
 files=w.unpack_vpk(packed,out);assert files[0].read_bytes()==b'echo original'
 for raw in [vpk(directory='../escape'),vpk()[:-1]]:
  packed.write_bytes(raw)
  try:w.unpack_vpk(packed,root/('bad'+str(len(raw))));raise AssertionError('Unsafe VPK accepted')
  except w.Limit:pass
 try:w.unpack_vpk(packed,out);raise AssertionError('Existing output collision accepted')
 except w.Limit:pass
print('VPK source extraction, traversal, truncation and output collisions passed.')
