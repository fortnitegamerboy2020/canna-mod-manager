#!/usr/bin/env python3
"""Static analysis only. Run under the isolated canna-review systemd service."""
import hashlib,json,os,re,shutil,signal,stat,subprocess,time,zipfile,struct,math
from collections import Counter
from pathlib import Path,PurePosixPath
ROOT=Path(os.environ.get('CANNA_REVIEW_JOBS','/var/lib/canna-review/jobs'))
VERSION='canna-static-3'
RULES=[
 ('network','Network access',r'https?://|\b(?:HttpClient|WebClient|UnityWebRequest|Socket|TcpClient|UdpClient|URLConnection|requests\.(?:get|post)|fetch\s*\()','review'),
 ('identity','Device or account information',r'GetPhysicalAddress|NetworkInterface|Environment\.(?:MachineName|UserName)|GetHostAddresses|GetHostName|System\.getProperty\s*\(\s*"(?:user|os)\.|getenv\s*\(|Environment\.GetEnvironmentVariable','review'),
 ('sensitive-files','Sensitive credential or browser paths',r'Login Data|Local State|Cookies|\.ssh|wallet\.dat|key4\.db|logins\.json|discord.{0,30}token|CryptUnprotectData|ProtectedData\.Unprotect','high'),
 ('filesystem','File system access',r'File\.(?:Read|Write|Delete|Move|Copy|Open)|Directory\.(?:Delete|GetFiles|Enumerate|Create)|FileStream|Files\.(?:read|write|delete)|FileInputStream|FileOutputStream|open\s*\(','review'),
 ('commands','Starting processes or shell commands',r'Process\.Start|ProcessStartInfo|Runtime\.getRuntime|ProcessBuilder|os\.system|subprocess\.|powershell|cmd\.exe|/bin/(?:sh|bash)','high'),
 ('privileges','Privilege changes or persistence',r'\brunas\b|AdjustTokenPrivileges|OpenProcessToken|CreateService|schtasks|CurrentVersion\\(?:Run|RunOnce)|setuid|sudo\b','high'),
 ('native','Native calls, memory access or injection',r'DllImport|LibraryImport|VirtualAlloc|WriteProcessMemory|CreateRemoteThread|GetProcAddress|LoadLibrary|Unsafe\.|sun\.misc\.Unsafe','review'),
 ('dynamic','Dynamic code or encoded payloads',r'Assembly\.Load|Activator\.CreateInstance|FromBase64String|eval\s*\(|defineClass|Invoke-Expression|DownloadString','review')]
RULES=[(a,b,re.compile(c,re.I),d) for a,b,c,d in RULES]
TEXT={'.cs','.java','.rs','.js','.ts','.lua','.py','.cpp','.c','.h','.hpp','.shader','.sh','.ps1','.json','.xml','.toml','.yml','.yaml','.txt','.md','.properties','.cfg','.ini'}
class Limit(Exception):pass

def packing_evidence(data):
 findings=[]
 for marker in [b'UPX!',b'.vmp0',b'.vmp1',b'VMProtect',b'Themida',b'.aspack',b'MPRESS',b'ConfusedByAttribute',b'Dotfuscator',b'Eazfuscator',b'Obfuscar',b'Enigma Protector']:
  if marker.lower() in data.lower():findings.append(('packer-marker','Possible packer or obfuscator marker',marker.decode()))
 if not data.startswith(b'MZ') or len(data)<64:return findings
 try:
  header=struct.unpack_from('<I',data,60)[0]
  if data[header:header+4]!=b'PE\0\0':return findings
  count=struct.unpack_from('<H',data,header+6)[0];optional=struct.unpack_from('<H',data,header+20)[0]
  if not 1<=count<=96 or header+24+optional+40*count>len(data):return findings
  entry=struct.unpack_from('<I',data,header+40)[0]
  for n in range(count):
   start=header+24+optional+40*n;name=data[start:start+8].split(b'\0')[0].decode('ascii','replace')
   virtual,rva,size,offset=struct.unpack_from('<IIII',data,start+8);flags=struct.unpack_from('<I',data,start+36)[0]
   raw=data[offset:offset+min(size,65536)];counts=Counter(raw);entropy=-sum((v/len(raw))*math.log2(v/len(raw)) for v in counts.values()) if raw else 0
   executable=bool(flags&0x20000000);writable=bool(flags&0x80000000)
   if executable and len(raw)>4096 and entropy>7.2:findings.append(('packer-heuristic','Possible packed executable section',f'{name}: entropy {entropy:.2f}/8, executable; compression or encryption is possible, not proven'))
   if executable and writable:findings.append(('packer-heuristic','Writable and executable section',f'{name}: RWX permissions may indicate unpacking, self-modifying code or a legitimate runtime'))
   if executable and rva<=entry<rva+max(size,virtual) and virtual>max(size*4,65536):findings.append(('packer-heuristic','Unusual entry-point section layout',f'{name}: entry point in expanded executable section, raw {size} bytes, virtual {virtual} bytes'))
 except (struct.error,ValueError):pass
 return findings

def analyze(job):
 report={'version':VERSION,'files':[],'inventory':[],'findings':[],'engines':{},'note':'Static analysis cannot prove a mod safe. Decompiled code is reconstructed, not the original project. Mods are never launched.'}
 total_text=0;start=time.monotonic();archive=job/'input.zip';work=job/'work';shutil.rmtree(work,ignore_errors=True);work.mkdir()
 def finding(rule,title,file=None,line=None,evidence='',severity='review'):
  if len(report['findings'])>=2000:return
  evidence=evidence[:350];key='\0'.join(map(str,[rule,file,line,evidence]));fid=hashlib.sha256(key.encode()).hexdigest()
  if any(f['id']==fid for f in report['findings']):return
  report['findings'].append({'id':fid,'rule':rule,'title':title,'file':file,'line':line,'evidence':evidence,'severity':severity})
 def command(args,timeout):
  if time.monotonic()-start>240:raise Limit('Analysis time budget reached')
  log=job/'tool.log'
  with log.open('wb') as out:
   proc=subprocess.Popen(args,stdout=out,stderr=subprocess.STDOUT,start_new_session=True)
   deadline=time.monotonic()+timeout
   try:
    while True:
     try:code=proc.wait(timeout=0.5);break
     except subprocess.TimeoutExpired:
      files=[p for p in job.rglob('*') if p.is_file()]
      if time.monotonic()>deadline or len(files)>6000 or sum(p.stat().st_size for p in files)>400*1024*1024:raise Limit('Analyzer time or disk budget reached')
   except Limit:
    os.killpg(proc.pid,signal.SIGKILL);proc.wait();raise
  text=log.read_bytes()[:32768].decode('utf-8','replace');log.unlink(missing_ok=True)
  return code,text
 def add_text(path,name,kind):
  nonlocal total_text
  if len(report['files'])>=500 or total_text+path.stat().st_size>8*1024*1024 or path.stat().st_size>1024*1024:
   finding('coverage','Source preview limit reached',name,severity='high');return
  try:text=path.read_text(encoding='utf-8')
  except (UnicodeError,OSError):finding('coverage','Text file could not be decoded',name);return
  if '\0' in text:finding('coverage','Binary content in text file',name);return
  total_text+=len(text.encode());report['files'].append({'name':name,'text':text,'kind':kind})
  # Documentation and package metadata are displayed but do not execute behavior.
  if path.suffix.lower() in ('.md','.xml') or path.name.lower()=='manifest.json':return
  for line_no,line in enumerate(text.splitlines(),1):
   for rule,title,pattern,severity in RULES:
    if pattern.search(line):finding(rule,title,name,line_no,line.strip(),severity)
 def extract(source,destination,prefix,depth=0):
  with zipfile.ZipFile(source) as z:
   infos=z.infolist()
   if len(infos)>2000 or sum(i.file_size for i in infos)>256*1024*1024:raise Limit('Archive exceeds 2000 entries or 256 MiB expanded size')
   seen=set();expanded=0
   for i in infos:
    if i.is_dir():continue
    name=i.filename.replace('\\','/');parts=PurePosixPath(name)
    if name.startswith('/') or any(p in ('..','.','') for p in name.split('/')) or ':' in name or '\0' in name or stat.S_ISLNK(i.external_attr>>16):raise Limit('Unsafe archive path or symbolic link')
    if name.casefold() in seen:raise Limit('Duplicate archive path')
    seen.add(name.casefold());expanded+=i.file_size
    if i.file_size>32*1024*1024: finding('coverage','File exceeds analysis size limit',prefix+name,severity='high');continue
    path=destination.joinpath(*parts.parts);path.parent.mkdir(parents=True,exist_ok=True)
    with z.open(i) as inp,path.open('wb') as out:
     count=0
     while chunk:=inp.read(65536):
      count+=len(chunk)
      if count>i.file_size or count>32*1024*1024:raise Limit('Archive entry size mismatch')
      out.write(chunk)
    report['inventory'].append({'name':prefix+name,'size':i.file_size,'kind':'archive'})
   return list(destination.rglob('*'))
 try:
  paths=extract(archive,work/'archive','archive/')
  binaries=0
  root_java=any(p.is_file() and p.suffix.lower()=='.class' for p in paths)
  if root_java:
   jar=work/'project.jar';shutil.copyfile(archive,jar);out=work/'java-project';out.mkdir()
   try:
    code,tool_log=command(['/usr/bin/java','-Xmx384m','-jar','/opt/canna-review/tools/cfr.jar',str(jar),'--outputdir',str(out),'--silent','true'],60)
    generated=[p for p in out.rglob('*.java') if p.is_file()]
    if code or not generated:finding('coverage','Java project decompilation incomplete',severity='high')
    for p in generated:add_text(p,'decompiled/java-project/'+p.relative_to(out).as_posix(),'decompiled')
   except (Limit,OSError):finding('coverage','Java project decompiler timed out or unavailable',severity='high')
  for path in paths:
   if not path.is_file():continue
   name='archive/'+path.relative_to(work/'archive').as_posix();ext=path.suffix.lower()
   with path.open('rb') as inp:signature=inp.read(4)
   pe=signature[:2]==b'MZ'
   if pe:
    for rule,title,evidence in packing_evidence(path.read_bytes()):finding(rule,title,name,evidence=evidence,severity='high')
    try:
     code,log=command(['/usr/bin/diec','-j','-u','-d',str(path)],25)
     if code!=0:raise Limit('Packer scanner failed')
     detection=json.loads(log);report['engines']['detect-it-easy']={'status':'complete','version':'3.21','heuristics':True}
     for group in detection.get('detects',[]):
      for value in group.get('values',[]):
       if value.get('type','').lower() in ('packer','protector','obfuscator') or re.search('pack|protect|obfuscat|virtualiz',value.get('string',''),re.I):
        finding('packer-signature','Detect It Easy packing / protection finding',name,evidence=value.get('string',str(value)),severity='high')
    except (Limit,OSError,ValueError):finding('coverage','Packer signature scanner failed or timed out',name,severity='high');report['engines']['detect-it-easy']={'status':'error','version':'3.21'}

   if ext in TEXT:add_text(path,name,'uploaded')
   elif root_java and ext=='.class':continue
   elif pe or ext in ('.dll','.exe','.jar','.class'):
    binaries+=1
    if binaries>16:finding('coverage','Decompiler assembly limit reached',name,severity='high');continue
    out=work/'decompiled'/str(binaries);out.mkdir(parents=True)
    try:
     if pe or ext in ('.dll','.exe'):
      code,tool_log=command(['/opt/canna-review/tools/ilspycmd','--disable-updatecheck','--nested-directories','-p','-o',str(out),str(path)],35)
     else:
      code,tool_log=command(['/usr/bin/java','-Xmx384m','-jar','/opt/canna-review/tools/cfr.jar',str(path),'--outputdir',str(out),'--silent','true'],40)
     if code!=0: finding('coverage','Decompilation failed or unsupported binary',name,evidence=f'Analyzer exit {code}: '+tool_log[-300:],severity='high')
     generated=[p for p in out.rglob('*') if p.is_file() and p.suffix.lower() in TEXT]
     if not generated:finding('coverage','No source reconstructed from binary',name,severity='high')
     for p in generated:add_text(p,'decompiled/'+name+'/'+p.relative_to(out).as_posix(),'decompiled')
    except (Limit,OSError):finding('coverage','Decompiler unavailable or time limit reached',name,severity='high')
   elif ext not in ('.png','.jpg','.jpeg','.webp','.gif','.ogg','.wav','.mp3','.ttf','.otf'):
    finding('coverage','File format not inspected as source',name)
  try:
   code,log=command(['/usr/bin/clamscan','--no-summary','--infected','--max-filesize=32M','--max-scansize=256M','--max-files=2000','--max-recursion=8','--alert-exceeds-max=yes','--alert-encrypted=yes',str(archive)],100)
   report['engines']['clamav']={'status':'complete' if code in (0,1) else 'error','version':subprocess.check_output(['/usr/bin/clamscan','--version'],timeout=5).decode().strip()}
   if code==1:
    for line in log.splitlines():
     if ' FOUND' in line:finding('signature','ClamAV malware signature match',None,None,line.split(': ',1)[-1],'critical')
   if code not in (0,1):finding('coverage','Antivirus scan failed',severity='high')
  except (Limit,OSError,subprocess.SubprocessError):report['engines']['clamav']={'status':'error'};finding('coverage','Antivirus scanner unavailable or timed out',severity='high')
 except PermissionError:
  raise
 except Exception as error:
  finding('coverage','Archive analysis incomplete',evidence=str(error)[:200],severity='high')
 report['engines']['heuristics']={'status':'complete','version':VERSION}
 report['status']='complete';report['created']=int(time.time())
 shutil.rmtree(work,ignore_errors=True);return report

def once(job):
 try:
  if (job/'input.zip').stat().st_size>128*1024*1024:raise Limit('Archive too large')
  report=analyze(job)
 except Exception as error:report={'status':'failed','error':str(error)[:200],'files':[],'findings':[]}
 tmp=job/'result.tmp';tmp.write_text(json.dumps(report),encoding='utf-8');os.chmod(tmp,0o660);tmp.rename(job/'result.json')
 (job/'input.zip').unlink(missing_ok=True);(job/'ready').unlink(missing_ok=True)

def main():
 os.umask(0o007)
 while True:
  for result in ROOT.glob('*/result.json'):
   if time.time()-result.stat().st_mtime>7200:shutil.rmtree(result.parent,ignore_errors=True)
  for ready in sorted(ROOT.glob('*/ready')):
   job=ready.parent
   if job.is_symlink() or not re.fullmatch('[0-9a-f-]{36}',job.name):continue
   once(job)
  time.sleep(1)
if __name__=='__main__':main()
