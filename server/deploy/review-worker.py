#!/usr/bin/env python3
"""Static analysis only. Run under the isolated canna-review systemd service."""
import hashlib,json,os,re,shutil,signal,stat,subprocess,time,zipfile,struct,math,importlib.util
from collections import Counter
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path,PurePosixPath
ROOT=Path(os.environ.get('CANNA_REVIEW_JOBS','/var/lib/canna-review/jobs'))
VERSION='canna-static-7'
# Fixed sibling module; no dependency or submitted plugin code is imported.
_context_spec=importlib.util.spec_from_file_location('canna_review_context',Path(__file__).with_name('review_context.py'))
context=importlib.util.module_from_spec(_context_spec);_context_spec.loader.exec_module(context)
RULES=context.RULES
TEXT={'.cs','.java','.rs','.js','.ts','.lua','.nut','.py','.cpp','.c','.h','.hpp','.shader','.sh','.ps1','.bat','.cmd','.json','.xml','.toml','.yml','.yaml','.txt','.md','.properties','.cfg','.ini','.res'}
class Limit(Exception):pass

def source_language(path):
 return {'.cs':'C#','.java':'Java','.rs':'Rust','.js':'JavaScript','.ts':'TypeScript','.lua':'Lua','.nut':'Squirrel','.py':'Python','.cpp':'C++','.c':'C','.h':'C/C++','.hpp':'C++','.shader':'Shader','.sh':'Shell','.ps1':'PowerShell','.bat':'Batch','.cmd':'Batch'}.get(path.suffix.lower(),'Text / metadata')

def generated_sources(output):
 """Read only regular, contained decompiler output; never follow tool symlinks.

 A submitted type/resource name can influence decompiler output paths. Tools
 still run in the service sandbox, and output is checked again before reading.
 """
 if not stat.S_ISDIR(output.lstat().st_mode):raise Limit('Decompiler output root is not a regular directory')
 found=[];seen=set();count=0;size=0;entries=0;root=output.resolve()
 def walk_error(error):raise error
 for directory,folders,files in os.walk(output,followlinks=False,onerror=walk_error):
  folders.sort();entries+=len(folders)+len(files)
  if entries>6000:raise Limit('Decompiler output exceeds entry limits')
  for name in sorted(folders+files):
   path=Path(directory)/name
   info=path.lstat()
   if stat.S_ISLNK(info.st_mode):raise Limit('Decompiler output contains a symbolic link')
   if not path.resolve().is_relative_to(root):raise Limit('Decompiler output escapes its output directory')
   relative=path.relative_to(output).as_posix()
   key=relative.casefold()
   if key in seen:raise Limit('Decompiler output contains a case-insensitive path collision')
   seen.add(key)
   if stat.S_ISDIR(info.st_mode):continue
   if not stat.S_ISREG(info.st_mode):raise Limit('Decompiler output contains a non-regular file')
   count+=1;size+=info.st_size
   if count>6000 or size>400*1024*1024:raise Limit('Decompiler output exceeds file or disk limits')
   if path.suffix.lower() in TEXT:found.append(path)
 return sorted(found,key=lambda path:path.relative_to(output).as_posix())

def unpack_vpk(source,destination,max_bytes=256*1024*1024,max_files=2000):
 if destination.exists():raise Limit('VPK extraction path collision')
 destination.mkdir(parents=True)
 data=source.read_bytes()
 if len(data)<12 or data[:4]!=b'\x34\x12\xaa\x55':raise Limit('Invalid VPK')
 version,tree_size=struct.unpack_from('<II',data,4);header={1:12,2:28}.get(version)
 if not header or not tree_size or header+tree_size>len(data):raise Limit('Invalid VPK directory')
 tree=data[header:header+tree_size];pos=0;files=[];seen=set();total=0
 def text():
  nonlocal pos
  end=tree.find(b'\0',pos)
  if end<0:raise Limit('Unterminated VPK path')
  value=tree[pos:end].decode('utf-8');pos=end+1;return value
 while extension:=text():
  while directory:=text():
   while filename:=text():
    if pos+18>len(tree):raise Limit('Truncated VPK entry')
    crc,preload,index,offset,length,end=struct.unpack_from('<IHHIIH',tree,pos);pos+=18
    if index!=0x7fff or end!=0xffff or pos+preload>len(tree) or offset+length>len(data)-header-tree_size:raise Limit('Split or truncated VPK')
    name=(directory+'/' if directory!=' ' else '')+filename+('.'+extension if extension!=' ' else '')
    parts=PurePosixPath(name)
    if name.startswith('/') or any(p in ('..','.','') for p in name.split('/')) or any(c in name for c in ('\\',':','\0')) or name.casefold() in seen:raise Limit('Unsafe VPK path')
    seen.add(name.casefold());total+=preload+length
    if len(seen)>max_files or total>max_bytes or preload+length>32*1024*1024:raise Limit('VPK extraction limit')
    content=tree[pos:pos+preload]+data[header+tree_size+offset:header+tree_size+offset+length];pos+=preload
    import zlib
    if zlib.crc32(content)!=crc:raise Limit('VPK checksum mismatch')
    target=destination.joinpath(*parts.parts);target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(content);files.append(target)
 return files

def pe_packing_layout(data):
 """Read bounded PE section metadata. Untrusted section names are only hints."""
 if not data.startswith(b'MZ'):return None
 try:
  if len(data)<64:raise ValueError('Truncated DOS header')
  header=struct.unpack_from('<I',data,60)[0]
  if header<64 or header+24>len(data) or data[header:header+4]!=b'PE\0\0':raise ValueError('Missing or invalid PE header')
  count,optional=struct.unpack_from('<H',data,header+6)[0],struct.unpack_from('<H',data,header+20)[0]
  opt=header+24;table=opt+optional;end=table+40*count
  if not 1<=count<=96 or end>len(data) or optional<2:raise ValueError('Truncated or unsupported PE section table')
  magic=struct.unpack_from('<H',data,opt)[0];minimum={0x10b:96,0x20b:112}.get(magic)
  if minimum is None or optional<minimum:raise ValueError('Invalid PE optional header')
  entry=struct.unpack_from('<I',data,opt+16)[0];sections=[]
  for n in range(count):
   start=table+40*n;name=data[start:start+8].split(b'\0')[0].decode('ascii','replace')
   virtual,rva,size,offset=struct.unpack_from('<IIII',data,start+8);flags=struct.unpack_from('<I',data,start+36)[0]
   if size and (offset<end or offset+size>len(data)):raise ValueError('PE section data is truncated or overlaps headers')
   if rva+max(size,virtual)>0x100000000:raise ValueError('PE section address range overflows')
   section={'name':name,'rva':rva,'virtual':virtual,'size':size,'offset':offset,'header':start,'executable':bool(flags&0x20000000),'writable':bool(flags&0x80000000)}
   for old in sections:
    if size and old['size'] and offset<old['offset']+old['size'] and old['offset']<offset+size:raise ValueError('Overlapping PE section data')
    if max(size,virtual) and max(old['size'],old['virtual']) and rva<old['rva']+max(old['size'],old['virtual']) and old['rva']<rva+max(size,virtual):raise ValueError('Overlapping PE section address ranges')
   sections.append(section)
  directory_count=struct.unpack_from('<I',data,opt+minimum-4)[0]
  resource=None
  if directory_count>2 and optional>=minimum+24:
   rva,size=struct.unpack_from('<II',data,opt+minimum+16)
   for section in sections:
    if size and section['rva']<=rva and rva+size<=section['rva']+section['size']:
     resource=(section['offset']+rva-section['rva'],size);break
  return {'entry':entry,'sections':sections,'resource':resource}
 except (struct.error,ValueError) as error:return {'error':str(error),'sections':[]}

def packing_evidence(data):
 findings=[];layout=pe_packing_layout(data)
 if layout and 'error' in layout:findings.append(('coverage','PE packing analysis could not inspect malformed headers',layout['error']))
 sections=layout['sections'] if layout else []
 entry=layout.get('entry',0) if layout else 0
 upx=[s for s in sections if s['name'].casefold() in ('upx0','upx1')]
 upx_entry=next((s for s in upx if s['executable'] and s['rva']<=entry<s['rva']+max(s['size'],s['virtual'])),None)
 # Magic plus a conventional UPX layout is stronger than an arbitrary string.
 # These are packing indicators, never a malware verdict.
 strong_upx=len({s['name'].casefold() for s in upx})==2 and upx_entry is not None and b'UPX!' in data[upx_entry['offset']:upx_entry['offset']+upx_entry['size']]
 if strong_upx:findings.append(('packer-marker','UPX packing indicators',f'UPX0/UPX1 section layout, executable entry in {upx_entry["name"]}, UPX! marker in that section; packing does not establish malware'))
 for marker in [b'UPX!',b'.vmp0',b'.vmp1',b'VMProtect',b'Themida',b'.aspack',b'MPRESS',b'ConfusedByAttribute',b'Dotfuscator',b'Eazfuscator',b'Obfuscar',b'Enigma Protector']:
  # Names inside assets/metadata are not packer signatures. Retain the hint with
  # its location, rather than automatically rejecting a mod for a text token.
  pattern=re.compile(re.escape(marker) if marker==b'UPX!' else rb'(?<![A-Za-z0-9_])'+re.escape(marker)+rb'(?![A-Za-z0-9_])',re.I)
  match=pattern.search(data)
  if not match or (marker==b'UPX!' and strong_upx):continue
  matches={match.start():match}
  # A resource token near the beginning must not conceal a later occurrence in
  # an executable section. Keep at most one occurrence per executable section.
  for section in sections:
   if section['executable'] and section['size']:
    executable_match=pattern.search(data,section['offset'],section['offset']+section['size'])
    if executable_match:matches[executable_match.start()]=executable_match
  for offset,match in sorted(matches.items()):
   containing=[s for s in sections if s['offset']<=offset and match.end()<=s['offset']+s['size']]
   where=f'offset 0x{offset:x}'
   if containing:
    section=containing[0];where+=f', {section["name"]!r} ({"executable" if section["executable"] else "non-executable"} section)'
    resource=layout.get('resource')
    if resource and resource[0]<=offset and match.end()<=sum(resource):where+=', resource-directory range'
   else:where+=', header/overlay or unclassified bytes'
   findings.append(('packing-review','Packer-related text marker requiring context',f'{marker.decode()}: {where}; a name or asset string alone does not establish packing, obfuscation or malware'))
 if not layout or 'error' in layout:return findings
 for section in sections:
  name=section['name'];size=section['size'];offset=section['offset'];virtual=section['virtual'];rva=section['rva']
  executable=section['executable'];writable=section['writable']
  if name.casefold() in ('upx0','upx1','.vmp0','.vmp1','.aspack','.mpress1','.mpress2') and not (strong_upx and name.casefold() in ('upx0','upx1')):
   findings.append(('packing-review','Packer-style section name requiring review',f'{name}: section names can be copied and do not establish packing by themselves'))
  if executable and size>4096:
   # Inspect beginning, middle and end: a low-entropy prefix must not hide a
   # high-entropy tail. Sampling remains a heuristic, not full code analysis.
   width=min(size,65536);positions=sorted({0,(size-width)//2,size-width});entropies=[]
   for position in positions:
    raw=data[offset+position:offset+position+width];counts=Counter(raw)
    entropies.append(-sum((v/len(raw))*math.log2(v/len(raw)) for v in counts.values()))
   entropy=max(entropies)
   if entropy>7.2:findings.append(('packing-review','High entropy bytes in executable section',f'{name}: maximum sampled entropy {entropy:.2f}/8 across {len(positions)} window(s); code sections may also contain compressed assets or data, and packing is not proven'))
  if executable and writable:findings.append(('packing-review','Writable and executable section',f'{name}: RWX permissions may indicate unpacking, self-modifying code or a legitimate runtime'))
  if executable and rva<=entry<rva+max(size,virtual) and virtual>max(size*4,65536):findings.append(('packing-review','Unusual entry-point section layout',f'{name}: entry point in expanded executable section, raw {size} bytes, virtual {virtual} bytes'))
 return findings

def die_packing_finding(value,data=None):
 """Classify DiE's detection role, not incidental words in compiler/library text."""
 if not isinstance(value,dict):return None
 evidence=value.get('string','');kind=value.get('type','');name=value.get('name','')
 if not all(isinstance(v,str) for v in (evidence,kind,name)):return None
 roles=r'packer|protector|protection|obfuscator|obfuscation|cryptor|crypter|virtualizer|virtualization|anti[ -]+analysis'
 role_label=r'\s*(?:~\s*)?(?:\(\s*heur\s*\)\s*)?(?:'+roles+r')\s*(?:\(\s*heur\s*\)\s*)?:?\s*'
 typed=re.fullmatch(role_label,kind,re.I)
 labelled=re.match(r'^\s*(?:\(\s*heur\s*\)\s*)?(?:'+roles+r')\s*:',evidence,re.I)
 if kind.strip() and not typed:return None
 if not typed and not labelled:return None
 combined=' '.join((kind,name,evidence))
 heuristic=kind.lstrip().startswith('~') or bool(re.search(r'\(\s*heur\s*\)|\bheuristic\b|\bgeneric\b|\banti[ -]+analysis\b',combined,re.I)) or value.get('heuristic') is True
 # An unnamed role is not a specific detector signature. Missing details stay
 # visible and unresolved rather than becoming automatic denial/acceptance.
 if (not evidence and not name) or (not name and re.fullmatch(role_label,evidence,re.I)):heuristic=True
 evidence=evidence or ': '.join(v for v in (kind,name) if v)
 if heuristic:
  evidence+='; heuristic evidence is inconclusive; compressed assets, DLL extension or readable source do not settle this finding'
  return ('packing-review','Detect It Easy heuristic requiring review',evidence)
 return ('packer-signature','Detect It Easy packing / protection signature',evidence)

def group_coverage_findings(findings):
 """Consolidate repeated limitations without changing their review requirement.

 Every retained occurrence stays in locations with its original finding ID.
 This is presentation postprocessing, not additional inspection or acceptance.
 """
 groups={}
 for item in findings:
  if item.get('rule')=='coverage' and item.get('id')!='finding-limit' and not item.get('locations'):
   groups.setdefault((item.get('title'),item.get('severity')),[]).append(item)
 replacements={}
 for key,items in groups.items():
  if len(items)<2:continue
  ordered=sorted(items,key=lambda item:(str(item.get('file') or ''),str(item.get('line') or ''),str(item.get('evidence') or ''),str(item.get('id') or '')))
  provenance=[{field:item.get(field) for field in ('id','file','line','evidence')} for item in ordered]
  # Full provenance binds the group identity to the exact evidence, even if an
  # input fixture supplies a reused ID. New omissions require a fresh decision.
  payload=json.dumps({'title':key[0],'severity':key[1],'locations':provenance},sort_keys=True,separators=(',',':'))
  replacements[key]={'id':hashlib.sha256(payload.encode()).hexdigest(),'rule':'coverage','title':key[0],'severity':key[1],
   'file':None,'line':None,'evidence':f'{len(items)} matching coverage limitations; each retained occurrence is listed below.',
   'context':'These limitations remain unresolved and require staff review. Grouping does not establish that omitted content was inspected. A finding-limit entry, when present, records additional omitted findings.',
   'locations':provenance}
 result=[];emitted=set()
 for item in findings:
  key=(item.get('title'),item.get('severity'))
  eligible=item.get('rule')=='coverage' and item.get('id')!='finding-limit' and not item.get('locations')
  if eligible and key in replacements:
   if key not in emitted:result.append(replacements[key]);emitted.add(key)
  else:result.append(item)
 return result

def analyze(job):
 report={'version':VERSION,'files':[],'inventory':[],'findings':[],'observations':[],'engines':{},'decompilations':[],
   'limits':{'analysis_seconds':240,'archive_entries':2000,'expanded_bytes':256*1024*1024,'entry_bytes':32*1024*1024,'decompiler_binaries':16,'java_projects':1,'tool_output_bytes':256*1024,'source_file_bytes':1024*1024,'scanned_text_bytes':16*1024*1024,'preview_files':500,'preview_text_bytes':8*1024*1024},
  'note':'Static analysis cannot prove a mod safe. Decompiled code is reconstructed, not the original project. Mods are never launched.'}
 total_text=0;scanned_text=0;finding_ids=set();observation_ids=set();start=time.monotonic();archive=job/'input.zip';work=job/'work';shutil.rmtree(work,ignore_errors=True);work.mkdir()
 def finding(rule,title,file=None,line=None,evidence='',severity='review',**details):
  evidence=evidence[:350];key='\0'.join(map(str,[rule,file,line,evidence]));fid=hashlib.sha256(key.encode()).hexdigest()
  item={'id':details.pop('id',fid),'rule':rule,'title':title,'file':file,'line':line,'evidence':evidence,'severity':severity,**details}
  if item['id'] in finding_ids:return
  if len(report['findings'])>=1999:
   # Reserve a permanent coverage row. Later signatures/coverage replace low
   # priority noise so padded code cannot hide an antivirus rejection.
   limit_id='finding-limit'
   if limit_id not in finding_ids:
    report['findings'].append({'id':limit_id,'rule':'coverage','title':'Finding limit reached','file':None,'line':None,'evidence':'Further findings were omitted; this report is incomplete and requires review.','severity':'high'});finding_ids.add(limit_id)
   if rule not in ('signature','coverage','packer-signature','packer-marker'):return
   priority={'signature':4,'packer-signature':3,'packer-marker':3,'coverage':2}
   incoming=priority.get(rule,1)
   candidates=[i for i,f in enumerate(report['findings']) if f['id']!=limit_id and priority.get(f['rule'],1)<incoming]
   if candidates:
    removed=report['findings'].pop(candidates[-1]);finding_ids.discard(removed['id'])
   elif len(report['findings'])>=2001:return
  finding_ids.add(item['id']);report['findings'].append(item)
 def command(args,timeout):
  if time.monotonic()-start>240:raise Limit('Analysis time budget reached')
  log=job/'tool.log'
  with log.open('wb') as out:
   proc=subprocess.Popen(args,stdout=out,stderr=subprocess.STDOUT,start_new_session=True)
   deadline=min(time.monotonic()+timeout,start+240)
   try:
    while True:
     try:code=proc.wait(timeout=0.5);break
     except subprocess.TimeoutExpired:
      files=[p for p in job.rglob('*') if p.is_file()]
      if time.monotonic()>deadline or len(files)>6000 or sum(p.stat().st_size for p in files)>400*1024*1024:raise Limit('Analyzer time or disk budget reached')
   except Limit:
    os.killpg(proc.pid,signal.SIGKILL);proc.wait();raise
  # A tool that exits within one polling interval still has to respect bounds.
  files=[p for p in job.rglob('*') if p.is_file() and not p.is_symlink()]
  if len(files)>6000 or sum(p.stat().st_size for p in files)>400*1024*1024:raise Limit('Analyzer file or disk budget reached')
  if log.stat().st_size>256*1024:
   log.unlink(missing_ok=True);raise Limit('Analyzer output limit reached; output is incomplete')
  text=log.read_bytes().decode('utf-8','replace');log.unlink(missing_ok=True)
  return code,text
 def add_text(path,name,kind,origin=None,decompiler=None,origins=None):
  nonlocal total_text,scanned_text
  result={'preview':False,'scanned':False,'status':'omitted'}
  if time.monotonic()-start>240:
   finding('coverage','Analysis time budget reached',name,severity='high');result['status']='analysis-time-limit';return result
  size=path.stat().st_size
  if size>1024*1024 or scanned_text+size>16*1024*1024:
   finding('coverage','Source analysis text limit reached',name,severity='high');result['status']='source-text-limit';return result
  try:
   raw=path.read_bytes();text=raw.decode('utf-16' if raw.startswith((b'\xff\xfe',b'\xfe\xff')) else 'utf-8-sig')
  except (UnicodeError,OSError):finding('coverage','Text file could not be decoded',name);result['status']='decode-failed';return result
  if '\0' in text:finding('coverage','Binary content in text file',name);result['status']='binary-content';return result
  encoded=text.encode();encoded_size=len(encoded)
  if scanned_text+encoded_size>16*1024*1024:
   finding('coverage','Source analysis text limit reached',name,severity='high');result['status']='source-text-limit';return result
  scanned_text+=encoded_size;result['scanned']=True;result['status']='scanned'
  if len(report['files'])<500 and total_text+encoded_size<=8*1024*1024:
   total_text+=encoded_size
   item={'name':name,'text':text,'kind':kind,'language':source_language(path),'byte_size':encoded_size,'line_count':len(text.splitlines()),'sha256':hashlib.sha256(encoded).hexdigest(),'origin':origin or name}
   if decompiler:item['decompiler']=decompiler
   if origins:item['origins']=origins
   report['files'].append(item);result['preview']=True
  else:finding('coverage','Source preview limit reached',name,evidence='Code heuristics still ran; this file is omitted from the preview.',severity='high')
  # Documentation and package metadata are displayed but do not execute behavior.
  if (path.suffix.lower()=='.md' or path.name.lower() in ('manifest.json','addoninfo.txt','license')) and not text.startswith('#!'):return result
  source_findings,observations=context.scan_source(text,name,path.suffix.lower())
  if path.suffix.lower()=='.cs':source_findings=context.contextualize_file_operations(text,source_findings)
  for f in source_findings:
   details={k:v for k,v in f.items() if k not in ('rule','title','file','line','evidence','severity')}
   finding(f['rule'],f['title'],f['file'],f['line'],f['evidence'],f['severity'],**details)
  for o in observations:
   if o['id'] in observation_ids:continue
   if len(report['observations'])>=500:
    finding('coverage','Observation preview limit reached',name,severity='high');break
   observation_ids.add(o['id']);report['observations'].append(o)
  return result
 def reconstruction(input_name,output,prefix,language,tool,args,timeout,inputs=None,dependency_directory=None):
  begun=time.monotonic()
  record={'input':input_name,'scope':'project' if inputs else 'binary','language':language,'tool':tool,'status':'complete','duration_ms':0,'exit_code':None,'generated_files':[],'generated_count':0,'generated_bytes':0,'preview_count':0,'preview_omitted_count':0,'scanned_count':0,'scan_omitted_count':0,'diagnostic':'','limitations':[],
   'dependency_resolution':{'mode':'archive-neighbors-and-installed-runtime' if dependency_directory else 'project-contained-classes','verification':'Tool does not provide a complete dependency-resolution report.','external_downloads':False}}
  if inputs:record['inputs']=inputs
  if dependency_directory:record['dependency_resolution']['supplied_directories']=[dependency_directory]
  report['decompilations'].append(record)
  try:
   code,log=command(args,timeout);record['exit_code']=code
   record['diagnostic']=log.replace(str(work),'[analysis]')[-2000:]
   if code!=0:
    record['status']='incomplete';record['limitations'].append('Decompiler returned a nonzero exit code.')
    finding('coverage','Decompilation failed or unsupported binary',input_name,evidence=f'Analyzer exit {code}: '+record['diagnostic'][-300:],severity='high')
  except Limit as error:
   record['status']='limited';record['diagnostic']=str(error)[:2000];record['limitations'].append(str(error))
   finding('coverage','Decompiler time or output limit reached',input_name,evidence=str(error),severity='high')
  except OSError as error:
   record['status']='unavailable';record['diagnostic']=str(error).replace(str(work),'[analysis]')[-2000:];record['limitations'].append('Decompiler could not be started or accessed.')
   finding('coverage','Decompiler unavailable',input_name,evidence=record['diagnostic'][-300:],severity='high')
  try:
   generated=generated_sources(output)
   if not generated:
    if record['status']=='complete':record['status']='failed'
    record['limitations'].append('No readable source was reconstructed.')
    finding('coverage','No source reconstructed from binary',input_name,severity='high')
   record['generated_count']=len(generated)
   record['generated_files']=[prefix+path.relative_to(output).as_posix() for path in generated]
   for path,name in zip(generated,record['generated_files']):
    try:
     record['generated_bytes']+=path.stat().st_size
     result=add_text(path,name,'decompiled',origin=input_name,decompiler=tool,origins=inputs)
     record['preview_count']+=int(result['preview']);record['scanned_count']+=int(result['scanned'])
     if result['status'] not in ('scanned','omitted') and result['status'] not in record['limitations']:record['limitations'].append(result['status'])
    except OSError as error:
     finding('coverage','Reconstructed source could not be read',name,evidence=str(error),severity='high')
     if 'Reconstructed source could not be read.' not in record['limitations']:record['limitations'].append('Reconstructed source could not be read.')
   record['preview_omitted_count']=len(generated)-record['preview_count'];record['scan_omitted_count']=len(generated)-record['scanned_count']
   if record['preview_omitted_count']:record['limitations'].append('Some reconstructed files were omitted from the source preview.')
   if record['scan_omitted_count']:record['limitations'].append('Some reconstructed files were not inspected by source heuristics.')
   if record['status']=='complete' and record['limitations']:record['status']='incomplete'
  except (Limit,OSError) as error:
   record['status']='failed';record['limitations'].append(str(error)[:2000])
   finding('coverage','Decompiler output could not be inspected',input_name,evidence=str(error),severity='high')
  record['duration_ms']=max(0,round((time.monotonic()-begun)*1000))
  engine=report['engines'].setdefault(tool,{'status':'complete','attempted':0,'reconstructed':0,'failed':0,'incomplete':0})
  engine['attempted']+=1;engine['reconstructed']+=int(record['generated_count']>0)
  if record['status'] in ('failed','unavailable'):engine['failed']+=1;engine['status']='error'
  elif record['status']!='complete':
   engine['incomplete']+=1
   if engine['status']!='error':engine['status']='incomplete'
  return record
 def omitted_binary(name,language,status,reason):
  report['decompilations'].append({'input':name,'scope':'binary','language':language,'tool':None,'status':status,'duration_ms':0,'exit_code':None,'generated_files':[],'generated_count':0,'generated_bytes':0,'preview_count':0,'preview_omitted_count':0,'scanned_count':0,'scan_omitted_count':0,'diagnostic':'','limitations':[reason]})
 def antivirus():
  try:
   code,log=command(['/usr/bin/clamscan','--no-summary','--infected','--max-filesize=32M','--max-scansize=256M','--max-files=2000','--max-recursion=8','--alert-exceeds-max=yes','--alert-encrypted=yes',str(archive)],100)
   report['engines']['clamav']={'status':'complete' if code in (0,1) else 'error','version':subprocess.check_output(['/usr/bin/clamscan','--version'],timeout=5).decode().strip()}
   if code==1:
    reported=False
    for line in log.splitlines():
     if ' FOUND' in line:
      reported=True;evidence=line.rsplit(': ',1)[-1].removesuffix(' FOUND')
      if evidence.startswith(('Heuristics.Limits.Exceeded.','Heuristics.Encrypted.')):
       finding('coverage','Antivirus coverage limit or encrypted content',None,None,evidence,'high');report['engines']['clamav']['status']='incomplete'
      else:finding('signature','ClamAV malware signature match',None,None,evidence,'critical')
    if not reported:finding('coverage','Antivirus result could not be interpreted',severity='high');report['engines']['clamav']['status']='error'
   if code not in (0,1):finding('coverage','Antivirus scan failed',severity='high')
  except (Limit,OSError,subprocess.SubprocessError):report['engines']['clamav']={'status':'error'};finding('coverage','Antivirus scanner unavailable or timed out',severity='high')
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
    inventory={'name':prefix+name,'size':i.file_size,'kind':'archive'};report['inventory'].append(inventory)
    if i.file_size>32*1024*1024:
     inventory['analysis']='entry-size-limit';finding('coverage','File exceeds analysis size limit',prefix+name,severity='high')
     if parts.suffix.lower() in ('.dll','.exe','.jar','.class'):omitted_binary(prefix+name,'Unknown','limited','Entry exceeds the 32 MiB extraction limit; binary magic and source were not inspected.')
     continue
    path=destination.joinpath(*parts.parts);path.parent.mkdir(parents=True,exist_ok=True)
    with z.open(i) as inp,path.open('wb') as out:
     count=0
     while chunk:=inp.read(65536):
      count+=len(chunk)
      if count>i.file_size or count>32*1024*1024:raise Limit('Archive entry size mismatch')
      out.write(chunk)
   return list(destination.rglob('*'))
 # Antivirus runs first so decompiler time limits cannot consume its budget.
 antivirus()
 try:
  paths=extract(archive,work/'archive','archive/')
  original_files={path for path in paths if path.is_file()}
  expanded_total=sum(p.stat().st_size for p in paths if p.is_file());expanded_count=sum(p.is_file() for p in paths)
  inspected_vpks=set()
  cursor=0;depths={p:0 for p in paths}
  while cursor<len(paths):
   packed=paths[cursor];cursor+=1
   if packed.is_file() and packed.suffix.lower()=='.vpk':
    if depths[packed]>=3:raise Limit('Nested VPK depth exceeds analysis limits')
    expanded=unpack_vpk(packed,packed.with_suffix('.vpk-source'),256*1024*1024-expanded_total,2000-expanded_count)
    inspected_vpks.add(packed)
    expanded_total+=sum(p.stat().st_size for p in expanded);expanded_count+=len(expanded)
    if expanded_total>256*1024*1024 or expanded_count>2000:raise Limit('Combined VPK expansion exceeds analysis limits')
    paths.extend(expanded)
    depths.update({p:depths[packed]+1 for p in expanded})
    for p in expanded:report['inventory'].append({'name':'archive/'+p.relative_to(work/'archive').as_posix(),'size':p.stat().st_size,'kind':'vpk-content'})
  binaries=0
  root_java=set()
  for path in original_files:
   if path.suffix.lower()=='.class':
    with path.open('rb') as inp:
     if inp.read(4)==b'\xca\xfe\xba\xbe':root_java.add(path)
  if root_java:
   # Repack only validated class entries. Resource paths from the submitted ZIP
   # must not become decompiler output paths, and expanded VPK classes are not
   # falsely described as classes contained in this project archive.
   jar=work/'project.jar';out=work/'java-project';out.mkdir()
   with zipfile.ZipFile(jar,'w') as project:
    for path in sorted(root_java):project.write(path,path.relative_to(work/'archive').as_posix())
   java_inputs=['archive/'+path.relative_to(work/'archive').as_posix() for path in sorted(root_java)]
   record=reconstruction('archive/',out,'decompiled/java-project/','Java','cfr',['/usr/bin/java','-Xmx384m','-jar','/opt/canna-review/tools/cfr.jar',str(jar),'--outputdir',str(out),'--silent','true'],60,inputs=java_inputs)
   record['mapping_note']='Whole-project reconstruction does not establish an exact source-file-to-class-file mapping.'
  for path in paths:
   if not path.is_file():continue
   name='archive/'+path.relative_to(work/'archive').as_posix();ext=path.suffix.lower()
   if ext=='.vpk' and path not in inspected_vpks:finding('coverage','Nested VPK is not inspected',name,severity='high')
   with path.open('rb') as inp:head=inp.read(16)
   signature=head[:4]
   pe=signature[:2]==b'MZ'
   if pe:
    binary_data=path.read_bytes()
    for rule,title,evidence in packing_evidence(binary_data):finding(rule,title,name,evidence=evidence,severity='high')
    try:
     code,log=command(['/usr/bin/diec','-j','-u','-d',str(path)],25)
     if code!=0:raise Limit('Packer scanner failed')
     detection=json.loads(log)
     if not isinstance(detection,dict) or not isinstance(detection.get('detects'),list):raise Limit('Invalid packer scanner report')
     if any(not isinstance(group,dict) or not isinstance(group.get('values'),list) or any(not isinstance(value,dict) for value in group['values']) for group in detection['detects']):raise Limit('Invalid packer detection entries')
     if any(any(field in value and not isinstance(value[field],str) for field in ('type','string','name')) for group in detection['detects'] for value in group['values']):raise Limit('Invalid packer detection fields')
     engine=report['engines'].setdefault('detect-it-easy',{'status':'complete','version':'3.21','heuristics':True,'scanned':0,'failed':0});engine['scanned']+=1
     for group in detection.get('detects',[]):
      for value in group.get('values',[]):
       detected=die_packing_finding(value,binary_data)
       if detected:
        rule,title,evidence=detected;finding(rule,title,name,evidence=evidence,severity='high')
    except (Limit,OSError,ValueError):
     finding('coverage','Packer signature scanner failed or timed out',name,severity='high')
     engine=report['engines'].setdefault('detect-it-easy',{'status':'complete','version':'3.21','heuristics':True,'scanned':0,'failed':0});engine['status']='error';engine['failed']+=1

   native_magic=signature in (b'\x7fELF',b'\xfe\xed\xfa\xce',b'\xfe\xed\xfa\xcf',b'\xce\xfa\xed\xfe',b'\xcf\xfa\xed\xfe')
   class_magic=signature==b'\xca\xfe\xba\xbe'
   if native_magic:
    finding('coverage','Native executable source is not reconstructed',name,evidence='Executable magic takes precedence over the filename or media extension.',severity='high')
    omitted_binary(name,'Native','not-supported','This pipeline does not reconstruct native executable source.')
   elif not pe and not class_magic and (ext in TEXT or path.name.lower()=='license'):add_text(path,name,'uploaded')
   elif path in root_java:continue
   elif pe or class_magic or ext in ('.dll','.exe','.jar','.class'):
    binaries+=1
    if binaries>16:
     finding('coverage','Decompiler assembly limit reached',name,severity='high');omitted_binary(name,'C#' if pe or ext in ('.dll','.exe') else 'Java','limited','Decompiler assembly limit reached (16 binaries).');continue
    out=work/'decompiled'/str(binaries);out.mkdir(parents=True)
    if pe or ext in ('.dll','.exe'):
     args=['/opt/canna-review/tools/ilspycmd','--disable-updatecheck','--nested-directories','-p','-r',str(path.parent),'-o',str(out),str(path)]
     language,tool,timeout='C#','ilspycmd',60
    else:
     args=['/usr/bin/java','-Xmx384m','-jar','/opt/canna-review/tools/cfr.jar',str(path),'--extraclasspath',str(path.parent),'--outputdir',str(out),'--silent','true']
     language,tool,timeout='Java','cfr',40
    reconstruction(name,out,'decompiled/'+name+'/',language,tool,args,timeout,dependency_directory='archive/'+path.parent.relative_to(work/'archive').as_posix())
   elif ext!='.vpk':
    asset_magic={'.png':head.startswith(b'\x89PNG\r\n\x1a\n'),'.jpg':head.startswith(b'\xff\xd8\xff'),'.jpeg':head.startswith(b'\xff\xd8\xff'),'.gif':head.startswith((b'GIF87a',b'GIF89a')),'.webp':head.startswith(b'RIFF') and head[8:12]==b'WEBP','.ogg':head.startswith(b'OggS'),'.wav':head.startswith(b'RIFF') and head[8:12]==b'WAVE','.mp3':head.startswith(b'ID3') or (len(head)>=2 and head[0]==255 and head[1]&0xe0==0xe0),'.ttf':head.startswith((b'\x00\x01\x00\x00',b'ttcf')),'.otf':head.startswith(b'OTTO')}
    if ext not in asset_magic:finding('coverage','File format not inspected as source',name)
    elif not asset_magic[ext]:finding('coverage','Asset extension does not match file signature',name,evidence='Unrecognized or disguised content is not treated as a harmless asset.',severity='high')

 except PermissionError:
  raise
 except Exception as error:
  finding('coverage','Archive analysis incomplete',evidence=str(error)[:200],severity='high')
 report['findings']=group_coverage_findings(report['findings'])
 report['coverage_complete']=not any(f['rule']=='coverage' for f in report['findings'])
 report['engines']['heuristics']={'status':'complete' if report['coverage_complete'] else 'incomplete','version':VERSION,'scanned_text_bytes':scanned_text}
 report['status']='complete';report['created']=int(time.time())
 shutil.rmtree(work,ignore_errors=True);return report

def once(job):
 try:
  if (job/'input.zip').stat().st_size>128*1024*1024:raise Limit('Archive too large')
  report=analyze(job)
 except Exception as error:report={'status':'failed','error':str(error)[:200],'files':[],'findings':[]}
 tmp=job/'result.tmp';tmp.write_text(json.dumps(report),encoding='utf-8');os.chmod(tmp,0o660);tmp.rename(job/'result.json')
 (job/'input.zip').unlink(missing_ok=True);(job/'ready').unlink(missing_ok=True)

def ready_jobs(root):
 # UUID names are random. Use arrival time so a newer job cannot jump the queue.
 jobs=[]
 for ready in root.glob('*/ready'):
  try:
   if ready.parent.is_symlink() or not re.fullmatch('[0-9a-f-]{36}',ready.parent.name):continue
   jobs.append((ready.stat().st_mtime_ns,str(ready.parent),ready.parent))
  except FileNotFoundError:continue
 return [job for _,_,job in sorted(jobs)]

def main():
 os.umask(0o007)
 workers=max(1,min(2,int(os.environ.get('CANNA_REVIEW_WORKERS','1'))))
 with ProcessPoolExecutor(max_workers=workers) as pool:
  active={}
  while True:
   for job,future in list(active.items()):
    if future.done():
     future.result();del active[job]
   for result in ROOT.glob('*/result.json'):
    if time.time()-result.stat().st_mtime>7200:shutil.rmtree(result.parent,ignore_errors=True)
   for job in ready_jobs(ROOT):
    if len(active)>=workers:break
    if job not in active:active[job]=pool.submit(once,job)
   time.sleep(0.25)
if __name__=='__main__':main()
