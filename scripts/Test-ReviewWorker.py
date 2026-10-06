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
