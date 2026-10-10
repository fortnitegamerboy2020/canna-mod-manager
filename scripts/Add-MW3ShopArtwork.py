"""Fetch credited MW3 title artwork and run the approved local 4x pipeline.

Resumable staging; --apply integrates only checked images and their routes.
No deployment or inventory mutation. Separate alpha is preserved exactly.
"""
import argparse, hashlib, json, re, subprocess, urllib.request, importlib.util
from collections import deque
import numpy as np
from html.parser import HTMLParser
from pathlib import Path
from PIL import Image
ROOT = Path(__file__).resolve().parents[1]
STAGE = ROOT / 'target/overnight-growth/mw3-art'
ART = ROOT / 'server/web/cosmetics'
TOOLS = ROOT / 'target/calling-card-upscale-review'
SOURCE = 'https://mw3titles.com/'
def sha(data): return hashlib.sha256(data).hexdigest()
def save(p, data): p.write_text(json.dumps(data, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
class Cards(HTMLParser):
    def __init__(self): super().__init__(); self.cards = {}
    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == 'img' and re.fullmatch(r'titles/[a-zA-Z0-9_-]+\.png', a.get('src', '')) and a.get('alt'):
            self.cards.setdefault(a['src'], a['alt'].strip())
def fetch(url):
    request = urllib.request.Request(url, headers={'User-Agent':'Canna-Artwork-Import/1.0'})
    with urllib.request.urlopen(request, timeout=30) as response:
        data = response.read(4*1024*1024+1)
    assert len(data) <= 4*1024*1024
    return data
def matte(frame):
    a=np.array(frame);rgb=a[:,:,:3].astype(int);w,h=frame.size
    background=(rgb.min(2)>=245)&(rgb.max(2)-rgb.min(2)<=8)&(a[:,:,3]>0)
    seen=set();q=deque((x,y) for x,y in [(x,0) for x in range(w)]+[(x,h-1) for x in range(w)]+[(0,y) for y in range(h)]+[(w-1,y) for y in range(h)] if background[y,x])
    while q:
        x,y=q.popleft()
        if (x,y) in seen: continue
        seen.add((x,y));a[y,x,3]=0
        for xx,yy in [(x-1,y),(x+1,y),(x,y-1),(x,y+1)]:
            if 0<=xx<w and 0<=yy<h and background[yy,xx] and (xx,yy) not in seen:q.append((xx,yy))
    result=Image.fromarray(a);box=result.getbbox();assert box
    return result.crop(box),len(seen)
def main(apply):
    spec=importlib.util.spec_from_file_location('approved_upscale',ROOT/'scripts/Upscale-CallingCards.py');pipeline=importlib.util.module_from_spec(spec);spec.loader.exec_module(pipeline)
    for folder in ['originals','inputs','raw','assets','corrected']: (STAGE/folder).mkdir(parents=True, exist_ok=True)
    page = STAGE/'page.html'
    if not page.exists(): page.write_bytes(fetch(SOURCE))
    parser = Cards(); parser.feed(page.read_text(encoding='utf-8'))
    entries = []
    for n, (src, title) in enumerate(parser.cards.items()):
        original = STAGE/'originals'/Path(src).name
        if not original.exists(): original.write_bytes(fetch(SOURCE+src))
        with Image.open(original) as im:
            assert im.format == 'PNG' and im.width <= 1024 and im.height <= 256
            rgba,removed=matte(im.convert('RGBA')); w,h = rgba.size
            assert w/h >= 2 and rgba.getbbox()
            key = sha(original.read_bytes())
            rgba.save(STAGE/'corrected'/(key+'.png'))
            color=pipeline.padded_rgb(rgba);input_key=sha(color.tobytes()+str(color.size).encode())
            color.save(STAGE/'inputs'/(input_key+'.png'))
        identity = 'mw3-'+re.sub('[^a-z0-9]+','-',Path(src).stem.removeprefix('mw3_').lower()).strip('-')+'-'+key[:8]
        entries.append(dict(id=identity, name='MW3 · '+title, source_url=SOURCE+src,
                            source_sha256=key, input_key=input_key, matte_pixels_removed=removed,width=w*4,height=h*4, shop_only=n<100))
    assert len(entries) >= 120
    exe = TOOLS/'runtime/realesrgan-ncnn-vulkan-v0.2.0-windows/realesrgan-ncnn-vulkan.exe'
    todo = [x for x in entries if not (STAGE/'raw'/(x['input_key']+'.png')).exists()]
    for start in range(0,len(todo),20):
        batch = STAGE/('matte-v2-batch-'+str(start)); batch.mkdir(exist_ok=True)
        for item in todo[start:start+20]:
            key = item['input_key']; (batch/(key+'.png')).write_bytes((STAGE/'inputs'/(key+'.png')).read_bytes())
        with (STAGE/'inference.log').open('a',encoding='utf-8') as log:
            subprocess.run([str(exe),'-i',str(batch),'-o',str(STAGE/'raw'),'-m',str(TOOLS/'models'),'-n','realesrgan-x4plus','-s','4','-t','128','-g','0','-j','1:1:1','-f','png'],stdout=log,stderr=log,check=True)
        print('MW3 inputs reconstructed:',min(start+20,len(todo)),'/',len(todo),flush=True)
    items=[]
    for n,entry in enumerate(entries):
        original = STAGE/'corrected'/(entry['source_sha256']+'.png')
        with Image.open(original) as im, Image.open(STAGE/'raw'/(entry['input_key']+'.png')) as color:
            assert color.size == (entry['width'],entry['height'])
            rgba=color.convert('RGBA'); alpha=im.convert('RGBA').getchannel('A').resize(rgba.size,Image.Resampling.LANCZOS); rgba.putalpha(alpha)
            dest=STAGE/'assets'/(entry['id']+'.webp'); rgba.save(dest,format='WEBP',lossless=True,method=1,exact=True)
            with Image.open(dest) as encoded: assert encoded.convert('RGBA').tobytes()==rgba.tobytes()
        item=dict(entry,kind='banner',collection='mw3',rarity=['common','uncommon','rare','epic','legendary'][n%5],weight=40,
                  filename=dest.name,asset='/api/v1/cosmetics/assets/'+entry['id'],sha256=sha(dest.read_bytes()),animated=False,
                  rights='Original Call of Duty artwork belongs to its respective creators; gallery credited to mw3titles.com',
                  conversion='4x Real-ESRGAN color reconstruction; border-connected white matte removed, transparent margins cropped, separate alpha resized; lossless WebP, not native HD artwork')
        items.append(item)
    assert len({i['id'] for i in items})==len(items) and sum(i['shop_only'] for i in items)==100
    save(STAGE/'items.json',items)
    if apply:
        catpath=ART/'catalog.json'; catalog=json.loads(catpath.read_text(encoding='utf-8'))
        backup=STAGE/'catalog-before.json'
        if not backup.exists(): backup.write_bytes(catpath.read_bytes())
        byid={i['id']:i for i in catalog['items']}
        for item in items:
            if item['id'] in byid:
                assert byid[item['id']]['source_sha256']==item['source_sha256']
                catalog['items'][catalog['items'].index(byid[item['id']])]=item
            else: catalog['items'].append(item)
            (ART/item['filename']).write_bytes((STAGE/'assets'/item['filename']).read_bytes())
        save(catpath,catalog)
        mime={'.svg':'image/svg+xml','.png':'image/png','.jpg':'image/jpeg','.webp':'image/webp'}
        routes=[]
        for item in catalog['items']:
            routes.append((item['id'],item['filename']))
            if item.get('poster_filename'): routes.append((item['id']+'-poster',item['poster_filename']))
        rust='// Generated checked asset routes; identities never become file paths.\npub fn find(id: &str) -> Option<(&\'static str, &\'static [u8])> {\n    match id {\n'
        for identity,filename in routes:
            assert re.fullmatch('[a-z0-9-]+\\.(png|webp|jpg|svg)',filename)
            rust+=f'        {json.dumps(identity)} => Some(({json.dumps(mime[Path(filename).suffix])}, include_bytes!({json.dumps("../web/cosmetics/"+filename)}))),\n'
        rust+='        _ => None,\n    }\n}\n'
        (ROOT/'server/src/cosmetic_assets.rs').write_text(rust,encoding='utf-8')
    save(STAGE/'receipt.json',dict(items=len(items),shop_reserved=100,crate_items=len(items)-100,scale=4,alpha_preserved=True,source=SOURCE,integrated=apply,published=False))
    print('MW3 assets checked:',len(items),'(100 shop-only)',flush=True)
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--apply',action='store_true');main(p.parse_args().apply)
