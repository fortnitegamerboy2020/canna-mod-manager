"""Prepare attributed Source recommendations and licensed, self-contained VPKs.

No Workshop addon binaries are mirrored. Their official listings handle subscriptions.
Licensed GitHub source is pinned and preserved inside its VPK alongside the license.
"""
import base64, hashlib, html, io, json, re, struct, urllib.request, zipfile, zlib
from pathlib import Path
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'server/staging-source'
OUT.mkdir(exist_ok=True)

def fetch(url, data=None):
    return urllib.request.urlopen(urllib.request.Request(url, data=data, headers={'User-Agent':'Canna-Catalog/1.0'}), timeout=40).read()

def vpk(files):
    tree, payload = bytearray(), bytearray()
    groups = {}
    for name, data in sorted(files.items()):
        p = Path(name)
        groups.setdefault(p.suffix[1:] or ' ', {}).setdefault(str(p.parent).replace('\\','/') if str(p.parent) != '.' else ' ', []).append((p.stem,data))
    for ext, dirs in groups.items():
        tree += ext.encode()+b'\0'
        for directory, items in dirs.items():
            tree += directory.encode()+b'\0'
            for name, data in items:
                tree += name.encode()+b'\0'+struct.pack('<IHHIIH', zlib.crc32(data),0,0x7fff,len(payload),len(data),0xffff)
                payload += data
            tree += b'\0'
        tree += b'\0'
    tree += b'\0'
    return struct.pack('<III',0x55aa1234,1,len(tree))+tree+payload

recommendations=[]
workshop=json.loads((ROOT/'target/source-workshop.json').read_text(encoding='utf-8-sig'))
for item in workshop:
    if item['result'] != 1: continue
    profile='https://steamcommunity.com/profiles/'+item['creator']
    author=ET.fromstring(fetch(profile+'/?xml=1')).findtext('steamID')
    assert author
    ident='workshop-'+item['publishedfileid']
    description=re.sub(r'\[/?(?:url(?:=[^\]]*)?|b|i|u|h[1-6]|list|\*)\]','',item['description'])
    page=fetch('https://steamcommunity.com/sharedfiles/filedetails/?id='+item['publishedfileid']).decode('utf-8')
    authors=[{'name':html.unescape(re.sub('<[^>]+>','',name)).strip(),'url':url} for url,name in re.findall(r'class="friendBlockLinkOverlay" href="([^"]+)".*?class="friendBlockContent">\s*(.*?)<br',page,re.S)]
    assert authors
    author=', '.join(a['name'] for a in authors)
    dependencies=[html.unescape(re.sub('<[^>]+>','',name)).strip() for name in re.findall(r'class="requiredItem">(.*?)</div>',page,re.S)]
    recommendations.append({'id':ident,'app_id':550,'name':item['title'],'version':'Workshop','description':description,'author':author,'review_status':'external','details':{'provider':'steam-workshop','game':'Left 4 Dead 2','authors':author,'author_links':[{'name':author,'url':profile}],'source_url':'https://steamcommunity.com/sharedfiles/filedetails/?id='+item['publishedfileid'],'icon_url':item['preview_url'],'external_only':True,'dependencies':dependencies,'content_type':'mod'}})
    recommendations[-1]['details']['author_links']=authors
    art=fetch(item['preview_url'])
    assert len(art)<4*1024*1024
    recommendations[-1]['details']['icon_data']=base64.b64encode(art).decode()

manifest=[]
for repo, commit, appid, mode in [
    ('originalgrego/L4D2-Practice-Script','ad238202a6367848b538fa181171f1c03d6021e1',550,'practice'),
    ('jpobzy/L4dAutoConfig','d49009405ea4b536ba858a8dc3e7fe319a70d95b',500,'tree'),
    ('jpobzy/L4dRemovedMainMenuMusic',None,500,'tree')]:
    info=json.loads(fetch('https://api.github.com/repos/'+repo))
    if not commit:
        commit=json.loads(fetch('https://api.github.com/repos/'+repo+'/commits/'+info['default_branch']))['sha']
    assert info['license']['spdx_id'] in ['MIT','GPL-2.0']
    archive=zipfile.ZipFile(io.BytesIO(fetch('https://codeload.github.com/'+repo+'/zip/'+commit)))
    source={p.filename.split('/',1)[1]:archive.read(p) for p in archive.infolist() if not p.is_dir()}
    files={'canna-source/'+name:data for name,data in source.items()}
    if mode=='practice':
        files['cfg/l4d2_practice.cfg']=source['l4d2_practice.cfg']
        files['addoninfo.txt']=b'"AddonInfo" { "addonSteamAppID" "550" "addontitle" "L4D2 Practice Script" "addonauthor" "originalgrego" "addonversion" "source" }'
    else:
        for name,data in source.items():
            if not name.startswith('.') and name not in ['README.md','LICENSE']:
                files[name]=data
    original_vpk=source.get('removedmainlobbymusic.vpk')
    packed=original_vpk or vpk(files)
    target=OUT/(repo.split('/')[1]+'.zip')
    with zipfile.ZipFile(target,'w',zipfile.ZIP_DEFLATED) as result:
        result.writestr('addon.vpk',packed)
        if original_vpk:
            for name,data in source.items():result.writestr('canna-source/'+name,data)
        result.writestr('LICENSE',source['LICENSE'])
        result.writestr('README.md',source['README.md']+f'\n\nCanna packaging: unchanged source from https://github.com/{repo}/tree/{commit}. All source files are also inside addon.vpk at canna-source/.\n'.encode())
    details={'provider':'github','source_url':'https://github.com/'+repo,'authors':repo.split('/')[0],'author_links':[{'name':repo.split('/')[0],'url':'https://github.com/'+repo.split('/')[0]}],'game':'Left 4 Dead 2' if appid==550 else 'Left 4 Dead','license':info['license']['spdx_id'],'commit':commit,'content_type':'mod','dependencies':[],'install_notes':'Start a local versus map and enter exec l4d2_practice in the console. This changes practice keybinds.' if mode=='practice' else 'Read the original README for configuration and compatibility. Configuration addons may change keybinds and interface settings.'}
    manifest.append({'app_id':appid,'name':info['name'],'version':commit[:12]+('.2' if original_vpk else ''),'description':info['description'],'origin':'github:'+repo+':'+commit+(':vpk2' if original_vpk else ''),'local_file':target.name,'sha256':hashlib.sha256(target.read_bytes()).hexdigest(),'details':details})
(ROOT/'server/web/source-recommendations.json').write_text(json.dumps(recommendations,ensure_ascii=False,indent=2),encoding='utf-8')
(OUT/'catalog-import.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2),encoding='utf-8')
(OUT/'assets-import.json').write_text('[]',encoding='utf-8')
print(f'Prepared {len(manifest)} licensed archives and {len(recommendations)} official Workshop listings.')
