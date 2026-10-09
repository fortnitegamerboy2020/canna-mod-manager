"""Validate, compact and integrate approved artwork into local server sources.

No build, deployment, account, inventory or publication operations. Original
files and catalog/routes are backed up under target before local replacement.
"""
from pathlib import Path
from PIL import Image
import hashlib,json,re,shutil,argparse

ROOT=Path(__file__).resolve().parents[1];ART=ROOT/'server/web/cosmetics'
BATCH=ROOT/'target/calling-card-upscale-batch';STAGE=ROOT/'target/calling-card-upscale-integration'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def visible_hash(im):return tuple(hashlib.sha256(Image.alpha_composite(Image.new('RGBA',im.size,c),im).tobytes()).hexdigest() for c in ['black','white'])
def write(path,obj):path.write_text(json.dumps(obj,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
def main(apply):
 STAGE.mkdir(exist_ok=True);(STAGE/'assets').mkdir(exist_ok=True);backup=STAGE/'originals';backup.mkdir(exist_ok=True)
 catpath=ART/'catalog.json';catalog=json.loads(catpath.read_text(encoding='utf-8'))
 batch=json.loads((BATCH/'receipt.json').read_text(encoding='utf-8'))
 assert batch['cards']==635 and batch['animation_timeline_validated'] and not batch['published']
 existing={x['id']:x for x in catalog['items']};updated=[];original_files=set()
 for n,out in enumerate(batch['outputs']):
  item=existing[out['id']];src=ART/item['filename'];assert sha(src)==out['source_sha256'],item['id']
  expected=BATCH/'assets'/out['filename'];assert sha(expected)==out['sha256']
  original_files.add(item['filename']);saved=backup/item['filename']
  if not saved.exists():shutil.copy2(src,saved)
  assert sha(saved)==out['source_sha256']
  original=Image.open(expected).convert('RGBA');filename=item['id']+'.webp';dest=STAGE/'assets'/filename
  if out['animated']:shutil.copy2(expected,dest)
  else:
   original.save(dest,format='WEBP',lossless=True,quality=80,method=1,exact=True)
   with Image.open(dest) as encoded:assert visible_hash(encoded.convert('RGBA'))==visible_hash(original)
  item['prior_artwork']={k:item[k] for k in ['filename','sha256','width','height','conversion','frame_count','duration_ms','poster_sha256'] if k in item}
  item.update(filename=filename,sha256=sha(dest),width=out['width'],height=out['height'],frame_count=out['encoded_frames'],
   conversion='4x Real-ESRGAN general color reconstruction; separate alpha; lossless WebP; local approved artwork preview',
   upscale={'scale':4,'model':'realesrgan-x4plus','pipeline':batch['version'],'input_sha256':out['source_sha256'],'corrections':out['corrections']})
  if out['animated']:
   poster=BATCH/'posters'/(out['id']+'.png');assert sha(poster)==out['poster_sha256']
   savedposter=backup/item['poster_filename']
   if not savedposter.exists():shutil.copy2(ART/item['poster_filename'],savedposter)
   assert sha(savedposter)==item['prior_artwork']['poster_sha256']
   shutil.copy2(poster,STAGE/'assets'/item['poster_filename'])
   item.update(duration_ms=out['duration_ms'],poster_sha256=out['poster_sha256'],animation_timing='100 ms per retained native frame; unused black atlas slots trimmed; four premultiplied bridge frames on large wrap jumps')
  updated.append(item['id'])
  if (n+1)%100==0:print(f'Compacted and checked {n+1}/635 cards',flush=True)
 catalog['paused_collections']=[x for x in catalog.get('paused_collections',[]) if x!='mw2']
 catalog['artwork_upscale']={'status':'local approved preview; unpublished','cards':635,'mw2':398,'bo2':237,'animated':13,'scale':4,'model':'realesrgan-x4plus','source':'AI reconstruction of credited source artwork; not native HD originals','pipeline':batch['version']}
 write(STAGE/'catalog.json',catalog)
 routes=[]
 for item in catalog['items']:
  routes.append((item['id'],item['filename']))
  if item.get('poster_filename'):routes.append((item['id']+'-poster',item['poster_filename']))
 assert len(routes)==1001 and len({i for i,_ in routes})==1001
 mime={'.svg':'image/svg+xml','.png':'image/png','.jpg':'image/jpeg','.webp':'image/webp'}
 rust="// Generated from the checked cosmetic catalog. IDs never become file paths.\npub fn find(id: &str) -> Option<(&'static str, &'static [u8])> {\n    match id {\n"
 for identity,filename in routes:
  assert re.fullmatch(r'[a-z0-9-]+\.(png|jpg|webp|svg)',filename)
  rust+=f'        {json.dumps(identity)} => Some(({json.dumps(mime[Path(filename).suffix])}, include_bytes!({json.dumps("../web/cosmetics/"+filename)}))),\n'
 rust+='        _ => None,\n    }\n}\n';(STAGE/'cosmetic_assets.rs').write_text(rust,encoding='utf-8')
 original_catalog=backup/'catalog.json';routepath=ROOT/'server/src/cosmetic_assets.rs'
 if not original_catalog.exists():shutil.copy2(catpath,original_catalog)
 if not (backup/'cosmetic_assets.rs').exists():shutil.copy2(routepath,backup/'cosmetic_assets.rs')
 before=json.loads(original_catalog.read_text(encoding='utf-8'));before_by_id={i['id']:i for i in before['items']}
 assert len(catalog['items'])==len(before['items'])==988
 for item in catalog['items']:
  previous=before_by_id[item['id']]
  for key in ['id','kind','name','rarity','weight','collection','asset']:assert item[key]==previous[key]
  if item['id'] not in updated:assert item==previous
 active_files={f for _,f in routes}
 receipt={'status':'staged local integration' if not apply else 'integrated local sources; unpublished','cards':635,'routes':1001,'catalog_items':988,'animated_posters':13,'inventory_ids_and_weights_preserved':True,'unrelated_items_unchanged':True,'originals_backup':str(backup),'lossless_static_visible_pixels_verified':True,'candidate_bytes':sum(p.stat().st_size for p in (STAGE/'assets').iterdir()),'published':False}
 if apply:
  # Every exact prior file was verified above. Only mutate resolved files
  # inside the named artwork root; no recursive deletion or game-directory use.
  for p in (STAGE/'assets').iterdir():shutil.copy2(p,ART/p.name)
  shutil.copy2(STAGE/'catalog.json',catpath);shutil.copy2(STAGE/'cosmetic_assets.rs',routepath)
  for filename in original_files-active_files:
   p=(ART/filename).resolve();assert p.parent==ART.resolve();assert sha(p)==before_by_id[next(i['id'] for i in before['items'] if i['filename']==filename)]['sha256'];p.unlink()
  for item in catalog['items']:
   assert sha(ART/item['filename'])==item['sha256']
   if item.get('poster_filename'):assert sha(ART/item['poster_filename'])==item['poster_sha256']
 write(STAGE/'receipt.json',receipt);print(json.dumps(receipt),flush=True)

if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--apply',action='store_true');main(p.parse_args().apply)
