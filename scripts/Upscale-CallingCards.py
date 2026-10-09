"""Stage reviewed 4x calling-card artwork with source receipts; never publish.

Uses the user-approved project-local Real-ESRGAN ncnn runtime. Original artwork,
IDs, inventory and catalog stay untouched. Run --prepare, --infer, --finish or
--all. Every stage is resumable; --samples restricts the initial correction run.
"""
from pathlib import Path
from collections import deque
import argparse, hashlib, json, subprocess, time
import numpy as np
from PIL import Image, ImageDraw

PROJECT = Path(__file__).resolve().parents[1]
TOOLS = PROJECT / 'target/calling-card-upscale-review'
ROOT = PROJECT / 'target/calling-card-upscale-batch'
ART = PROJECT / 'server/web/cosmetics'
CAT = ART / 'catalog.json'
WEED = {'mw2-blunttrauma-a89c363e', 'mw2-highcommand-2d7b0763', 'mw2-jointops-c62f1632'}
SAMPLES = {s['id'] for s in json.loads((TOOLS/'samples.json').read_text(encoding='utf-8'))}
VERSION = 'canna-card-alpha-loop-v2'
ALL_PRO = 'mw2-allpro-title-7f747f6a'

def sha(data): return hashlib.sha256(data).hexdigest()
def write_json(path, data):
    path.write_text(json.dumps(data, indent=2, ensure_ascii=False)+'\n', encoding='utf-8')

def clean_weed(frame):
    """Exact three-card shared-art mask. Never apply gray removal to other art.

    Preserve the dark title plate and colored leaves. Remove neutral JPEG matte
    debris outside that plate, including detached white surround islands.
    """
    if frame.size != (187, 40) and frame.size != (186, 40):
        raise ValueError('Unknown cannabis-card geometry; review mask first')
    a = np.array(frame); rgb = a[:,:,:3].astype(float)
    saturation = rgb.max(2)-rgb.min(2); light = rgb.mean(2)
    plate = Image.new('L', frame.size)
    ImageDraw.Draw(plate).polygon([(45,12),(143,12),(151,26),(143,32),(45,32),(33,27)], fill=255)
    protected = np.array(plate)>0
    remove = (~protected)&(saturation<=29)&(light>60)&(a[:,:,3]>0)
    a[remove,3] = 0
    seen = set(); w,h = frame.size
    for y,x in zip(*np.where(a[:,:,3]>0)):
        if (x,y) in seen: continue
        q = deque([(x,y)]); seen.add((x,y)); component=[]
        while q:
            x,y = q.popleft(); component.append((x,y))
            for xx,yy in ((x-1,y),(x+1,y),(x,y-1),(x,y+1)):
                if 0<=xx<w and 0<=yy<h and a[yy,xx,3]>0 and (xx,yy) not in seen:
                    seen.add((xx,yy)); q.append((xx,yy))
        if len(component)<10 and not any(protected[y,x] for x,y in component):
            for x,y in component: a[y,x,3]=0
    assert np.array_equal(a[:,:,:3], np.array(frame)[:,:,:3])
    return Image.fromarray(a), int(np.sum((np.array(frame)[:,:,3]>0)&(a[:,:,3]==0)))

def padded_rgb(frame):
    """Keep alpha separate; hide no gray/black matte from the neural model.

    Extend neighboring visible color into transparent texels. This changes only
    fully invisible RGB and prevents the neural model learning a gray surround.
    """
    a = np.array(frame); known=a[:,:,3]>0
    if known.all(): return frame.convert('RGB')
    if not known.any(): raise ValueError('Empty artwork cannot be upscaled')
    h,w=known.shape; rgb=a[:,:,:3].copy(); q=deque()
    for y,x in zip(*np.where(known)):
        if any(0<=xx<w and 0<=yy<h and not known[yy,xx] for xx,yy in ((x-1,y),(x+1,y),(x,y-1),(x,y+1))):
            q.append((x,y))
    while q:
        x,y=q.popleft()
        for xx,yy in ((x-1,y),(x+1,y),(x,y-1),(x,y+1)):
            if 0<=xx<w and 0<=yy<h and not known[yy,xx]:
                rgb[yy,xx]=rgb[y,x]; known[yy,xx]=True; q.append((xx,yy))
    return Image.fromarray(rgb)

def clean_all_pro(frame):
    # Reviewed outline for this exact 175x38 skateboard sprite. The grayscale
    # wheels, trucks and lettering are foreground and must survive cleanup.
    if frame.size != (175,38): raise ValueError('Unknown All Pro geometry')
    mask=Image.new('L',frame.size);d=ImageDraw.Draw(mask)
    d.polygon([(18,9),(161,0),(167,0),(173,2),(174,6),(174,11),(170,16),(160,21),(145,24),(125,24),(105,25),(65,25),(41,23),(29,19),(22,14)],fill=255)
    d.polygon([(40,13),(48,11),(57,13),(61,19),(59,24),(56,33),(50,36),(44,33),(42,24)],fill=255)
    d.polygon([(121,10),(130,9),(137,12),(140,18),(138,26),(136,34),(130,37),(124,35),(122,27)],fill=255)
    d.rectangle((63,12,114,27),fill=255)
    a=np.array(frame);before=a[:,:,3].copy();allowed=np.array(mask)>0
    a[~allowed,3]=0
    # Neutral JPEG fringe below the deck, away from the title and both trucks.
    yy,xx=np.indices(before.shape);rgb=a[:,:,:3].astype(float)
    neutral=(rgb.max(2)-rgb.min(2)<=32)&(rgb.mean(2)>55)
    trucks=((xx>=40)&(xx<=61))|((xx>=121)&(xx<=140))
    below_deck=(yy>=24)&~trucks
    gray_matte=neutral&(rgb.mean(2)<185)
    a[gray_matte&below_deck,3]=0
    # The deck/trucks remain connected. Any remaining isolated neutral dots
    # inside the approximate outline are compression debris, not artwork.
    seen=set();w,h=frame.size
    for y,x in zip(*np.where(a[:,:,3]>0)):
        if (x,y) in seen: continue
        q=deque([(x,y)]);seen.add((x,y));component=[]
        while q:
            x,y=q.popleft();component.append((x,y))
            for xx1,yy1 in ((x-1,y),(x+1,y),(x,y-1),(x,y+1)):
                if 0<=xx1<w and 0<=yy1<h and a[yy1,xx1,3]>0 and (xx1,yy1) not in seen:
                    seen.add((xx1,yy1));q.append((xx1,yy1))
        if len(component)<8 and all(max(a[y,x,:3])-min(a[y,x,:3])<=32 for x,y in component):
            for x,y in component:a[y,x,3]=0
    assert np.array_equal(a[:,:,:3],np.array(frame)[:,:,:3])
    return Image.fromarray(a),int(np.sum((before>0)&(a[:,:,3]==0)))

def visible(frame):
    a=np.array(frame).astype(float)
    return a[:,:,:3]*a[:,:,3:4]/255

def trim_padding(frames):
    # Only a trailing, near-black run of at least two atlas slots is padding.
    # Relative brightness also detects leftover DDS block noise. Legitimate
    # dark scenes (Mad Panda, Sunset) remain because they are not a tiny fraction
    # of their own animation's brightness.
    levels=[float(visible(f).mean()) for f in frames]
    threshold=min(8.0, float(np.median(levels[:min(12,len(levels))]))*.055)
    end=len(frames)
    while end>2 and levels[end-1]<=threshold: end-=1
    if len(frames)-end<2: end=len(frames)
    return frames[:end], {'source_slots':len(frames),'kept_slots':end,'removed_trailing_padding':len(frames)-end,'blank_threshold':threshold,'slot_brightness':levels}

def seam_bridge(frames):
    """Bridge only a large wrap jump, using premultiplied RGBA interpolation.

    Original moving frames remain intact. Four additional 100ms bridge frames
    avoid a sudden wrap; near-natural loops need none.
    """
    arrays=[visible(f) for f in frames]
    steps=[float(np.abs(b-a).mean()) for a,b in zip(arrays,arrays[1:])]
    jump=float(np.abs(arrays[-1]-arrays[0]).mean())
    typical=float(np.median(steps))
    count=4 if jump>max(5.0,typical*2.5) else 0
    if count:
        a=np.array(frames[-1]).astype(float)/255; b=np.array(frames[0]).astype(float)/255
        for j in range(1,count+1):
            t=j/(count+1); alpha=a[:,:,3]*(1-t)+b[:,:,3]*t
            premul=a[:,:,:3]*a[:,:,3:4]*(1-t)+b[:,:,:3]*b[:,:,3:4]*t
            rgb=np.divide(premul,alpha[:,:,None],out=np.zeros_like(premul),where=alpha[:,:,None]>0)
            frame=np.dstack([rgb,alpha]); frames.append(Image.fromarray(np.round(frame*255).clip(0,255).astype(np.uint8)))
    return frames, {'original_wrap_difference':jump,'median_frame_difference':typical,'bridge_frames':count}

def prepare(samples_only):
    ROOT.mkdir(exist_ok=True)
    for name in ['inputs','alpha','raw','assets','posters','sheets']: (ROOT/name).mkdir(exist_ok=True)
    cat=json.loads(CAT.read_text(encoding='utf-8'))
    if cat.get('artwork_upscale'):raise ValueError('Already integrated upscaled artwork: use the preserved original catalog to prepare a new trial, never upscale the 4x outputs again')
    rar=json.loads((PROJECT/'target/callingcard-asset-audit/rar-audit.json').read_text(encoding='utf-8'))
    items=[]; inputs=set()
    for item in cat['items']:
        if item.get('kind')!='banner' or item.get('collection') not in ['mw2','bo2']: continue
        if samples_only and item['id'] not in SAMPLES: continue
        source=ART/item['filename']; assert sha(source.read_bytes())==item['sha256']
        animated=bool(item.get('animated')); metadata={}; source_frames=[]
        if animated:
            sheet_path=Path(rar['extracted'])/item['source_entry']
            assert sha(sheet_path.read_bytes())==item['source_sha256']
            sheet=Image.open(sheet_path).convert('RGBA')
            assert sheet.width==256 and sheet.height in [1024,2048]
            source_frames=[sheet.crop((0,y,256,y+64)) for y in range(0,sheet.height,64)]
            source_frames,metadata['padding']=trim_padding(source_frames)
            source_frames,metadata['seam']=seam_bridge(source_frames)
        else:
            source_frames=[Image.open(source).convert('RGBA')]
            if item['id'] in WEED:
                source_frames[0],removed=clean_weed(source_frames[0]); metadata['weed_removed_pixels']=removed
            if item['id']==ALL_PRO:
                source_frames[0],removed=clean_all_pro(source_frames[0]); metadata['skateboard_removed_pixels']=removed
        keys=[]; alphas=[]
        for n,frame in enumerate(source_frames):
            rgb=padded_rgb(frame); key=sha(rgb.tobytes()+str(rgb.size).encode())
            input_path=ROOT/'inputs'/(key+'.png')
            if not input_path.exists(): rgb.save(input_path)
            alpha_name=f"{item['id']}-{n:03}.png"; frame.getchannel('A').save(ROOT/'alpha'/alpha_name)
            keys.append(key); alphas.append(alpha_name); inputs.add(key)
        items.append({'id':item['id'],'name':item['name'],'collection':item['collection'],'source_filename':item['filename'],'source_sha256':item['sha256'],'native_size':list(source_frames[0].size),'animated':animated,'input_keys':keys,'alpha_files':alphas,'frame_duration_ms':100 if animated else None,'corrections':metadata})
    write_json(ROOT/'plan.json',{'version':VERSION,'samples_only':samples_only,'model':'realesrgan-x4plus','scale':4,'items':items,'unique_inputs':len(inputs),'frames':sum(len(i['input_keys']) for i in items),'catalog_sha256':sha(CAT.read_bytes()),'shipped_hashes':{i['filename']:i['sha256'] for i in cat['items']}})
    print(f'Prepared {len(items)} cards / {len(inputs)} unique neural inputs',flush=True)

def infer():
    plan=json.loads((ROOT/'plan.json').read_text(encoding='utf-8'))
    pending=ROOT/'pending'; pending.mkdir(exist_ok=True)
    keys=set(k for i in plan['items'] for k in i['input_keys'])
    for k in sorted(keys):
        src=ROOT/'inputs'/(k+'.png'); dst=pending/(k+'.png')
        if not (ROOT/'raw'/(k+'.png')).exists():
            if not dst.exists(): dst.write_bytes(src.read_bytes())
    # Restrict inference to current plan; stale pending files are not run.
    todo=sorted(k for k in keys if not (ROOT/'raw'/(k+'.png')).exists())
    status={'phase':'inference','cards':len(plan['items']),'unique_inputs':len(keys),'pending':len(todo),'started_at':time.time(),'published':False}
    write_json(ROOT/'status.json',status)
    exe=TOOLS/'runtime/realesrgan-ncnn-vulkan-v0.2.0-windows/realesrgan-ncnn-vulkan.exe'
    # Small batches provide durable progress and resume points, with low GPU
    # memory use. All inputs are trusted decoded images, never addon code.
    for start in range(0,len(todo),40):
        batch=ROOT/f'infer-{start//40:03}'; batch.mkdir(exist_ok=True)
        for k in todo[start:start+40]: (batch/(k+'.png')).write_bytes((ROOT/'inputs'/(k+'.png')).read_bytes())
        args=[str(exe),'-i',str(batch),'-o',str(ROOT/'raw'),'-m',str(TOOLS/'models'),'-n','realesrgan-x4plus','-s','4','-t','128','-g','0','-j','1:1:1','-f','png']
        with (ROOT/'inference.log').open('a',encoding='utf-8') as log:
            subprocess.run(args,stdout=log,stderr=log,check=True)
        for k in todo[start:start+40]:
            with Image.open(ROOT/'raw'/(k+'.png')) as im,Image.open(ROOT/'inputs'/(k+'.png')) as original:
                assert im.size==(original.width*4,original.height*4)
        status['pending']=len(todo)-min(start+40,len(todo)); status['completed_at']=time.time()
        write_json(ROOT/'status.json',status)
        print(f"Upscaled {len(keys)-status['pending']}/{len(keys)} unique inputs",flush=True)

def visible_hash(frame):
    return tuple(sha(Image.alpha_composite(Image.new('RGBA',frame.size,bg),frame).tobytes()) for bg in ['black','white'])
def timeline(frames,durations):
    out=[]
    for f,d in zip(frames,durations):
        h=visible_hash(f)
        if out and out[-1][0]==h: out[-1][1]+=d
        else: out.append([h,d])
    return out

def finish():
    plan=json.loads((ROOT/'plan.json').read_text(encoding='utf-8')); outputs=[]
    previous={x['id']:x for x in json.loads((ROOT/'receipt.json').read_text(encoding='utf-8'))['outputs']} if (ROOT/'receipt.json').exists() else {}
    for idx,item in enumerate(plan['items']):
        cached=previous.get(item['id']);cached_file=ROOT/'assets'/cached['filename'] if cached else None
        if cached and all(cached.get(k)==v for k,v in item.items()) and cached_file.exists() and sha(cached_file.read_bytes())==cached['sha256'] and sha((ROOT/'posters'/(item['id']+'.png')).read_bytes())==cached['poster_sha256']:
            outputs.append(cached);continue
        frames=[]
        for k,alpha_name in zip(item['input_keys'],item['alpha_files']):
            frame=Image.open(ROOT/'raw'/(k+'.png')).convert('RGBA')
            alpha=Image.open(ROOT/'alpha'/alpha_name).resize(frame.size,Image.Resampling.LANCZOS)
            frame.putalpha(alpha); frames.append(frame)
        filename=item['id']+('.webp' if item['animated'] else '.png'); dest=ROOT/'assets'/filename
        encoded_count=1
        if item['animated']:
            frames[0].save(dest,format='WEBP',save_all=True,append_images=frames[1:],duration=100,loop=0,lossless=True,quality=80,method=1,exact=True)
            check=Image.open(dest); decoded=[]; durations=[]
            for n in range(check.n_frames):
                check.seek(n); decoded.append(check.convert('RGBA')); durations.append(check.info['duration'])
            assert timeline(decoded,durations)==timeline(frames,[100]*len(frames))
            assert check.info['loop']==0
            encoded_count=check.n_frames
        else: frames[0].save(dest)
        poster=ROOT/'posters'/(item['id']+'.png'); frames[0].save(poster)
        outputs.append({**item,'filename':filename,'sha256':sha(dest.read_bytes()),'width':frames[0].width,'height':frames[0].height,'encoded_frames':encoded_count,'duration_ms':len(frames)*100 if item['animated'] else None,'poster_sha256':sha(poster.read_bytes())})
        if (idx+1)%40==0: print(f'Validated {idx+1}/{len(plan["items"])} output cards',flush=True)
    assert sha(CAT.read_bytes())==plan['catalog_sha256'], 'Catalog changed during batch; rebase before integrating'
    for filename,digest in plan['shipped_hashes'].items(): assert sha((ART/filename).read_bytes())==digest
    write_json(ROOT/'receipt.json',{'version':VERSION,'status':'complete local artwork candidates; not integrated or published','cards':len(outputs),'unique_neural_inputs':plan['unique_inputs'],'scale':4,'model':'realesrgan-x4plus','all_published_artwork_unchanged':True,'animation_timeline_validated':True,'published':False,'outputs':outputs})
    write_json(ROOT/'status.json',{'phase':'complete','cards':len(outputs),'published':False,'completed_at':time.time()})
    print(f'Finished and validated {len(outputs)} local artwork candidates; production untouched',flush=True)

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--samples',action='store_true')
    for stage in ['prepare','infer','finish','all']: parser.add_argument('--'+stage,action='store_true')
    args=parser.parse_args()
    if args.prepare or args.all: prepare(args.samples)
    if args.infer or args.all: infer()
    if args.finish or args.all: finish()
