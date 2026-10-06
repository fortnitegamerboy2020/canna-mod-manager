import json,os,subprocess,tempfile,time,uuid,zipfile,shutil
from pathlib import Path
with tempfile.TemporaryDirectory(prefix='canna-review-fixture-') as tmp:
 root=Path(tmp);project=root/'fixture';project.mkdir()
 (project/'fixture.csproj').write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>')
 (project/'Example.cs').write_text('namespace ReviewFixture; public class Example { public string GetUrl() { return "https://example.invalid"; } public void ExampleOnly() { System.Diagnostics.Process.Start("not-a-real-command"); } }')
 subprocess.run(['dotnet','build',str(project),'-c','Release','--nologo','-v','quiet'],check=True,stdout=subprocess.DEVNULL)
 job=Path('/var/lib/canna-review/jobs')/str(uuid.uuid4())
 root.chmod(0o755)
 staged=root/'handoff.zip'
 with zipfile.ZipFile(staged,'w') as z:
  z.write(project/'bin/Release/net8.0/fixture.dll','fixture.dll')
  # Harmless standard antivirus test string, not executable malware.
  z.writestr('antivirus-test.txt',b'X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*')
 staged.chmod(0o644)
 handoff="""import os,sys,shutil;from pathlib import Path
job=Path(sys.argv[1]);job.mkdir();os.chmod(job,0o770);gid=job.stat().st_gid
shutil.copyfile(sys.argv[2],job/'input.zip');os.chown(job/'input.zip',-1,gid);os.chmod(job/'input.zip',0o660)
(job/'ready').write_text('ready');os.chown(job/'ready',-1,gid);os.chmod(job/'ready',0o660)
"""
 subprocess.run(['systemd-run','--quiet','--wait','--pipe','--collect','-p','User=canna','-p','Group=canna','-p','SupplementaryGroups=canna-review','-p','RestrictSUIDSGID=true','-p','NoNewPrivileges=true','-p','CapabilityBoundingSet=','-p','ProtectSystem=strict','-p','ReadWritePaths=/var/lib/canna-review/jobs','/usr/bin/python3','-c',handoff,str(job),str(staged)],check=True)

 deadline=time.monotonic()+180
 while not (job/'result.json').exists() and time.monotonic()<deadline:time.sleep(2)
 try:
  result=json.loads((job/'result.json').read_text());assert result['status']=='complete',result
  assert any(f['kind']=='decompiled' and 'Example' in f['name'] for f in result['files']),result['findings']
  assert any(f['rule']=='commands' and f['line'] for f in result['findings']),result['findings']
  assert any(f['rule']=='signature' for f in result['findings']),result['engines']
  print(json.dumps({'decompiled_files':len(result['files']),'findings':len(result['findings']),'engines':result['engines'],'production_identity_handoff':True,'passed':True}))
 finally:shutil.rmtree(job)
