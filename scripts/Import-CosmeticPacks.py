"""Import only image assets from the two user-supplied packs; never run addon code."""
import collections
import hashlib
import io
import json
import re
import zipfile
from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / 'server/web/cosmetics'
CAT = DEST / 'catalog.json'
sha = lambda b: hashlib.sha256(b).hexdigest()
catalog = json.loads(CAT.read_text(encoding='utf-8'))
baseline = {i['id'] for i in catalog['items']}
added = []
sources = []
for archive, prefix, collection in [
    ('mw2-callcards-remastered-main.zip', 'emblem-mw2', 'mw2-emblems'),
    ('Call-of-Duty-Rank-Emblems-main.zip', 'emblem-rank', 'cod-ranks'),
]:
    path = Path.home() / 'Downloads' / archive
    z = zipfile.ZipFile(path)
    source = {'archive': archive, 'sha256': sha(path.read_bytes()), 'images': 0}
    for entry in z.infolist():
        name = Path(entry.filename)
        if name.suffix.lower() != '.png':
            continue
        if collection == 'mw2-emblems' and '/mw2cc/emblems/' not in entry.filename:
            continue
        if collection == 'cod-ranks' and '/png/' not in entry.filename:
            continue
        data = z.read(entry)
        image = Image.open(io.BytesIO(data))
        image.verify()
        image = Image.open(io.BytesIO(data))
        assert image.format == 'PNG' and 0 < image.width <= 1024 and 0 < image.height <= 1024
        source['images'] += 1
        group = entry.filename.split('/')[1] if collection == 'cod-ranks' else 'MW2'
        stem = re.sub(r'[^a-z0-9]+', '-', (group+'-'+name.stem).lower()).strip('-')[:49]
        item_id = prefix+'-'+stem+'-'+sha(data)[:8]
        if item_id in {i['id'] for i in catalog['items']}:
            continue
        filename = item_id+'.png'
        (DEST/filename).write_bytes(data)
        label = re.sub(r'^\d+_preview_', '', name.stem).replace('_', ' ')
        label = re.sub(r'^iw5 prestige ', 'Prestige ', label)
        label = re.sub(r'^master prestige ', 'Master Prestige ', label)
        item = dict(id=item_id, kind='emblem', collection=collection, name=group+' · '+label,
                    rarity='rare' if 'prestige' in label.lower() else 'common', weight=40,
                    filename=filename, asset='/api/v1/cosmetics/assets/'+item_id,
                    sha256=sha(data), width=image.width, height=image.height, animated=False,
                    source_archive=archive, source_member=entry.filename,
                    rights='User-supplied game artwork; original Call of Duty artwork belongs to its respective creators')
        catalog['items'].append(item)
        added.append(item_id)
    sources.append(source)

effects = {
    'aurora': ('Aurora', ['#91ffd2','#90caff','#df9dff','#91ffd2']),
    'canna': ('Emerald Flow', ['#b7ff84','#35d3a5','#e8ffb2','#b7ff84']),
    'sunset': ('Sunset', ['#ffca75','#ff8cac','#c2a1ff','#ffca75']),
    'royal': ('Royal Gold', ['#fff3b0','#f5c766','#ffd5a3','#fff3b0']),
    'ice': ('Glacier', ['#c8ffff','#7fb8ff','#e5f3ff','#c8ffff']),
    'rainbow': ('Prismatic', ['#ff93b5','#ffe17e','#9cf7bf','#95cfff','#d5a4ff','#ff93b5']),
}
for style,(label,colors) in effects.items():
    item_id='name-effect-'+style
    if item_id in {i['id'] for i in catalog['items']}:
        continue
    stops=''.join(f'<stop offset="{i/(len(colors)-1):.4f}" stop-color="{c}"/>' for i,c in enumerate(colors))
    svg=f'<svg xmlns="http://www.w3.org/2000/svg" width="320" height="80" viewBox="0 0 320 80"><defs><linearGradient id="g">{stops}</linearGradient></defs><text x="160" y="53" fill="url(#g)" font-family="sans-serif" font-size="36" font-weight="700" text-anchor="middle">CANNA</text></svg>\n'
    data=svg.encode();filename=item_id+'.svg';(DEST/filename).write_bytes(data)
    catalog['items'].append(dict(id=item_id,kind='name_effect',collection='username-effects',name=label,
        style=style,rarity='epic' if style=='rainbow' else 'rare',weight=30 if style=='rainbow' else 100,
        filename=filename,asset='/api/v1/cosmetics/assets/'+item_id,sha256=sha(data),animated=True,
        rights='Original Canna vector artwork and CSS animation'))
    added.append(item_id)
catalog['cosmetic_pack_sources']=sources
CAT.write_text(json.dumps(catalog,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
routes=[]
for item in catalog['items']:
    routes.append((item['id'],item['filename']))
    if item.get('poster_filename'):routes.append((item['id']+'-poster',item['poster_filename']))
assert len(routes)==len({r[0] for r in routes}) and baseline.issubset({i['id'] for i in catalog['items']})
mime={'.png':'image/png','.jpg':'image/jpeg','.webp':'image/webp','.svg':'image/svg+xml'}
rust="// Generated from the checked cosmetic catalog. IDs never become file paths.\npub fn find(id: &str) -> Option<(&'static str, &'static [u8])> {\n    match id {\n"
for item_id,filename in routes:
    assert re.fullmatch(r'[a-z0-9-]+\.(png|jpg|webp|svg)',filename)
    rust+=f'        {json.dumps(item_id)} => Some(({json.dumps(mime[Path(filename).suffix])}, include_bytes!({json.dumps("../web/cosmetics/"+filename)}))),\n'
rust+='        _ => None,\n    }\n}\n'
(ROOT/'server/src/cosmetic_assets.rs').write_text(rust,encoding='utf-8')
receipt={'added':len(added),'catalog_items':len(catalog['items']),'routes':len(routes),'sources':sources,
    'mw2_banners_still_paused':'mw2' in catalog['paused_collections'],
    'counts':dict(collections.Counter(i['kind'] for i in catalog['items'])),
    'executable_or_addon_code_imported':False}
(ROOT/'target/cosmetic-pack-import-receipt.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf-8')
print(json.dumps(receipt,indent=2))
