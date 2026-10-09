#!/usr/bin/env python3
"""Exercise documentation exclusions without launching analyzer tools or mods."""
import importlib.util,json,tempfile,zipfile
from pathlib import Path
spec=importlib.util.spec_from_file_location('review_worker',Path(__file__).with_name('review-worker.py'))
worker=importlib.util.module_from_spec(spec);spec.loader.exec_module(worker)
with tempfile.TemporaryDirectory() as tmp:
 root=Path(tmp)
 old=root/'ffffffff-ffff-4fff-8fff-ffffffffffff';old.mkdir();(old/'ready').touch()
 new=root/'00000000-0000-4000-8000-000000000000';new.mkdir();(new/'ready').touch()
 worker.os.utime(old/'ready',ns=(1000000000,1000000000));worker.os.utime(new/'ready',ns=(2000000000,2000000000))
 invalid=root/'invalid';invalid.mkdir();(invalid/'ready').touch()
 assert worker.ready_jobs(root)==[old,new], 'Worker must use FIFO arrival order, not random UUID order'
class AnalyzerFixture:
 def __init__(self,*args,**kwargs):pass
 def wait(self,timeout=None):return 0
worker.subprocess.Popen=AnalyzerFixture
worker.subprocess.check_output=lambda *args,**kwargs:b'fixture-version'
with tempfile.TemporaryDirectory() as tmp:
 job=Path(tmp)
 with zipfile.ZipFile(job/'input.zip','w') as archive:
  archive.writestr('manifest.json',json.dumps({'website_url':'https://example.org'}))
  archive.writestr('README.md','https://example.org; Process.Start is documentation')
  archive.writestr('Plugin.cs','[BepInPlugin("com.YourUsername.Test", "Test", "1")]\nProcess.Start("cmd.exe");\nHttpClient client; client.GetAsync(url);\nEnvironment.UserName;')
 report=worker.analyze(job)
 assert len(report['files'])==3
 assert all(f['file']=='archive/Plugin.cs' for f in report['findings'])
 assert {f['rule'] for f in report['findings']}=={'commands','network','identity'}
 assert not any(f['line']==1 for f in report['findings'])
print('Documentation stays readable; metadata URLs are excluded; code behavior and line numbers remain flagged.')
