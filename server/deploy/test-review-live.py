#!/usr/bin/env python3
"""Check the installed v7 worker with two disposable production-identity jobs.

Run as an authorized systemd operator after installation. Builds only a trusted
fixture; never executes the resulting DLL, approves a mod or reanalyzes a live
mod. The harmless EICAR test string verifies the actual antivirus path.
"""
import hashlib,json,shutil,subprocess,tempfile,time,uuid,zipfile
from pathlib import Path

EXPECTED_VERSION='canna-static-7'
API_UNIT=['systemd-run','--quiet','--wait','--pipe','--collect',
 '-p','User=canna','-p','Group=canna','-p','SupplementaryGroups=canna-review',
 '-p','UMask=0077','-p','RestrictSUIDSGID=true','-p','NoNewPrivileges=true',
 '-p','CapabilityBoundingSet=','-p','ProtectSystem=strict',
 '-p','ReadWritePaths=/var/lib/canna-review/jobs']

# Import the installed root-owned worker and its sibling module under the review
# identity, without calling main/analyze or importing any submitted code.
installed_check='''import importlib.util,json,sys
from pathlib import Path
spec=importlib.util.spec_from_file_location('installed_review_worker',Path('/opt/canna-review/review-worker.py'))
worker=importlib.util.module_from_spec(spec);spec.loader.exec_module(worker)
assert worker.VERSION==sys.argv[1],worker.VERSION
value={'type':'~packer','name':'Generic','string':'(Heur)Packer: Generic'}
result=worker.die_packing_finding(value)
assert result is not None and result[0]=='packing-review',result
assert result[2].startswith('(Heur)Packer: Generic'),result
print(json.dumps({'installed_version':worker.VERSION,'installed_tilde_packer_retained':True}))
'''
checked=subprocess.run(['systemd-run','--quiet','--wait','--pipe','--collect',
 '-p','User=canna-review','-p','Group=canna-review','-p','NoNewPrivileges=true',
 '-p','CapabilityBoundingSet=','-p','PrivateNetwork=true','-p','ProtectHome=true',
 '-p','ProtectSystem=strict','/usr/bin/python3','-c',installed_check,EXPECTED_VERSION],
 check=True,capture_output=True,text=True)
installed=json.loads(checked.stdout)

with tempfile.TemporaryDirectory(prefix='canna-review-fixture-') as tmp:
 root=Path(tmp);project=root/'fixture';project.mkdir()
 (project/'fixture.csproj').write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>',encoding='utf-8')
 (project/'Example.cs').write_text('''namespace ReviewFixture;
public class Example {
 public string GetUrl() { return "https://example.invalid/fixture"; }
 public void NeverRun() { System.Diagnostics.Process.Start("fixture-only-never-run"); }
 public void UnknownDirectory(string path) { System.IO.File.WriteAllText(System.IO.Path.Combine(path,"report.txt"),"fixture"); }
}''',encoding='utf-8')
 (root/'NuGet.Config').write_text('<configuration><packageSources><clear /></packageSources></configuration>',encoding='utf-8')
 subprocess.run(['dotnet','restore',str(project),'--configfile',str(root/'NuGet.Config'),'--nologo','-v','quiet'],check=True,stdout=subprocess.DEVNULL)
 subprocess.run(['dotnet','build',str(project),'-c','Release','--no-restore','--nologo','-v','quiet'],check=True,stdout=subprocess.DEVNULL)
 jobs=[Path('/var/lib/canna-review/jobs')/str(uuid.uuid4()) for _ in range(2)]
 root.chmod(0o755)
 staged=root/'handoff.zip'
 with zipfile.ZipFile(staged,'w') as z:
  z.write(project/'bin/Release/net8.0/fixture.dll','fixture.dll')
  z.writestr('antivirus-test.txt',b'X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*')
 staged.chmod(0o644)
 handoff='''import grp,os,shutil,stat,sys
from pathlib import Path
os.umask(0o077)
gid=grp.getgrnam('canna-review').gr_gid
jobs=[Path(value) for value in sys.argv[2:]]
for job in jobs:
 job.mkdir();os.chmod(job,0o770)
 assert stat.S_IMODE(job.stat().st_mode)==0o770
 assert job.stat().st_gid==gid,'Job must inherit the spool review group'
 # Mirror scans.rs worker_file: RestrictSUIDSGID rejects a setgid chmod,
 # so files receive the job review group explicitly before mode 660.
 shutil.copyfile(sys.argv[1],job/'input.zip');os.chown(job/'input.zip',-1,job.stat().st_gid);os.chmod(job/'input.zip',0o660)
 assert (job/'input.zip').stat().st_gid==gid,'Archive must receive the job review group'
for job in jobs:
 (job/'ready').write_text('ready',encoding='utf-8');os.chown(job/'ready',-1,job.stat().st_gid);os.chmod(job/'ready',0o660)
 assert (job/'ready').stat().st_gid==gid,'Ready marker must receive the job review group'
'''
 read_result='''import grp,json,stat,sys
from pathlib import Path
result=Path(sys.argv[1])/'result.json'
assert result.stat().st_gid==grp.getgrnam('canna-review').gr_gid
assert stat.S_IMODE(result.stat().st_mode)==0o660
print(json.dumps(json.loads(result.read_text(encoding='utf-8'))))
'''
 try:
  # Stage both archives before either ready marker so the concurrency check is
  # not dominated by separate transient-unit startup times.
  subprocess.run(API_UNIT+['/usr/bin/python3','-c',handoff,str(staged),*[str(job) for job in jobs]],check=True)
  deadline=time.monotonic()+180;overlap=False
  while time.monotonic()<deadline:
   overlap=overlap or all((job/'work').is_dir() and not (job/'result.json').exists() for job in jobs)
   if all((job/'result.json').exists() for job in jobs):break
   time.sleep(0.25)
  results=[]
  for job in jobs:
   assert (job/'result.json').exists(),'Fixture worker result timed out'
   # Reading as the API identity verifies result handoff, not only operator access.
   read=subprocess.run(API_UNIT+['/usr/bin/python3','-c',read_result,str(job)],check=True,capture_output=True,text=True)
   result=json.loads(read.stdout);results.append(result)
   assert result['version']==EXPECTED_VERSION,result.get('version')
   assert result['status']=='complete',result
   assert any(f['kind']=='decompiled' and 'Example' in f['name'] for f in result['files']),result['findings']
   assert any(f['rule']=='commands' and f['line'] for f in result['findings']),result['findings']
   assert any(f['rule']=='filesystem' and f['line'] and 'report.txt' in f['evidence'] for f in result['findings']),result['findings']
   assert any(o['rule']=='url-reference' and 'https://example.invalid/fixture' in o['evidence'] for o in result['observations']),result.get('observations')
   assert any(f['rule']=='signature' and f['severity']=='critical' and 'EICAR' in f['evidence'].upper() for f in result['findings']),result['findings']
   assert result['engines']['detect-it-easy']['status']=='complete',result['engines']
   assert result['engines']['detect-it-easy']['scanned']>=1,result['engines']
   assert result['engines']['clamav']['status']=='complete',result['engines']
   assert result['coverage_complete'] is True,result['findings']
   assert not any(f['rule']=='coverage' for f in result['findings']),result['findings']
   assert not any('accepted' in f for f in result['findings']),'Fixture findings must have no staff decisions'
   records=[item for item in result['decompilations'] if item['input']=='archive/fixture.dll']
   assert len(records)==1,result['decompilations']
   record=records[0]
   assert record['tool']=='ilspycmd' and record['status']=='complete',record
   assert record['generated_count']==record['preview_count']==record['scanned_count'],record
   assert record['generated_count']>=1 and record['duration_ms']>=0,record
   assert record['dependency_resolution']['external_downloads'] is False,record
   for source in result['files']:
    if source['kind']!='decompiled':continue
    assert source['origin']==record['input'] and source['name'] in record['generated_files'],source
    assert source['language']=='C#' and source['decompiler']=='ilspycmd',source
    assert source['sha256']==hashlib.sha256(source['text'].encode()).hexdigest(),source['name']
    assert source['byte_size']==len(source['text'].encode()),source['name']
  assert overlap,'Two fixture jobs did not run concurrently'
  print(json.dumps({**installed,'concurrent_jobs':2,'overlap_observed':overlap,
   'decompiled_files':[len(result['files']) for result in results],
   'findings':[len(result['findings']) for result in results],
   'decompilation_provenance':True,'displayed_source_hashes_verified':True,
   'engines':[result['engines'] for result in results],
   'production_identity_handoff':True,'job_group_inherited':True,
   'explicit_file_group_assignment':True,'passed':True}))
 finally:
  for job in jobs:shutil.rmtree(job,ignore_errors=True)
