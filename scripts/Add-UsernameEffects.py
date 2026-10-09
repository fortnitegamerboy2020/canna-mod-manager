"""Add original, code-native gradient swatches; preserve existing cosmetic IDs."""
from pathlib import Path
import hashlib,json
ROOT=Path(__file__).resolve().parents[1]
DIR=ROOT/'server/web/cosmetics'
PRESETS={
 'ember':('Ember',['#ffbc6e','#ef6548','#ffe8aa'],'rare',14),
 'ocean':('Ocean',['#74f5dc','#558be8','#c2faff'],'common',30),
 'nebula':('Nebula',['#dc9dff','#865cef','#71d8fa'],'epic',8),
 'candy':('Candy',['#ffa6df','#bfa7ff','#fff1f9'],'common',30),
 'forest':('Forest',['#bdf68e','#4dc896','#e7ffd1'],'common',30),
 'silver':('Silver',['#eeeeff','#9aaabc','#ffffff'],'rare',14),
 'toxic':('Toxic',['#d2ff44','#74d950','#fdffa0'],'rare',14),
 'rose':('Rose Gold',['#ffddd4','#db9a88','#ffe8cc'],'rare',14),
 'lava':('Lava',['#ffbc44','#f44361','#ffddd7'],'epic',8),
 'midnight':('Midnight',['#93a6ff','#a675df','#d3e3ff'],'rare',14),
 'prism':('Prism',['#83ffe6','#ff9dea','#ffefa0','#83affe'],'legendary',3),
 'bliss':('Bliss',['#b0ffb6','#7ac9ff','#efa7ff','#fcffb7'],'legendary',3),
}
def main():
 catalog=json.loads((DIR/'catalog.json').read_text())
 known={item['id'] for item in catalog['items']}
 css=[]
 for style,(name,colors,rarity,weight) in PRESETS.items():
  identity='name-effect-'+style
  stops=''.join(f'<stop offset="{i/(len(colors)-1)*100:.2f}%" stop-color="{color}"/>' for i,color in enumerate(colors))
  data=f'<svg xmlns="http://www.w3.org/2000/svg" width="256" height="64" viewBox="0 0 256 64"><defs><linearGradient id="g">{stops}</linearGradient></defs><rect x="2" y="2" width="252" height="60" rx="14" fill="url(#g)"/></svg>'.encode()
  filename=identity+'.svg';(DIR/filename).write_bytes(data)
  if identity not in known:
   catalog['items'].append(dict(id=identity,name=name+' flow',kind='name_effect',collection='username-effects',style=style,rarity=rarity,weight=weight,animated=True,asset='/api/v1/cosmetics/assets/'+identity,filename=filename,sha256=hashlib.sha256(data).hexdigest(),width=256,height=64,source='Original Canna CSS gradient swatch'))
  css.append(f'[data-username-effect={style}]{{--name-gradient:linear-gradient(100deg,{",".join(colors+[colors[0]])})}}')
 (DIR/'catalog.json').write_text(json.dumps(catalog,indent=2)+'\n')
 path=ROOT/'server/web/forum.css';text=path.read_text();marker='/* Expanded original username presets */'
 if marker in text: text=text[:text.index(marker)]
 path.write_text(text.rstrip()+'\n'+marker+'\n'+'\n'.join(css)+'\n')
 # Rebuild only exact catalog routes, never directory globs.
 mime={'.png':'image/png','.svg':'image/svg+xml','.jpg':'image/jpeg','.webp':'image/webp'}
 routes=[]
 for item in catalog['items']:
  routes.append((item['id'],item['filename']))
  if item.get('poster_filename'): routes.append((item['id']+'-poster',item['poster_filename']))
 rust='// Generated from the checked cosmetic catalog. IDs never become file paths.\npub fn find(id: &str) -> Option<(&\'static str, &\'static [u8])> {\n match id {\n'
 for identity,filename in routes: rust+=f' {json.dumps(identity)} => Some(({json.dumps(mime[Path(filename).suffix])},include_bytes!({json.dumps("../web/cosmetics/"+filename)}))),\n'
 rust+=' _=>None,\n }\n}\n';(ROOT/'server/src/cosmetic_assets.rs').write_text(rust)
 print(json.dumps({'items':len(catalog['items']),'routes':len(routes),'added_presets':len(PRESETS)}))
if __name__=='__main__': main()
