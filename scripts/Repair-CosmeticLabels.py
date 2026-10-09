"""Repair imported display labels without altering IDs, rarity, odds or artwork."""
import hashlib
import json
import re
from pathlib import Path

root = Path(__file__).resolve().parents[1]
path = root/'server/web/cosmetics/catalog.json'
catalog = json.loads(path.read_text(encoding='utf-8'))
original = {item['id']:dict(item) for item in catalog['items']}
reviewed = {'angola':'Top Hat','armenia':'Great Wave','cuba':'Fiery Duel','faroe-islands':'Owl',
            'lesotho':'Sharingan','malawi':'Dragon','morocco':'Fun Fact','saudi-arabia':'Sunset Path','spain':'Web Slinger'}
flags = sorted(item['id'] for item in catalog['items'] if item['id'].startswith('bo2-volkz-flag-'))
changed = []
for item in catalog['items']:
    label = item['name']
    for _ in range(2):
        if not any(c in label for c in ['\u00c2','\u00c3']):
            break
        try:
            repaired = label.encode('latin-1').decode('utf-8')
        except (UnicodeError,ValueError):
            break
        label = repaired
    if item['id'].startswith('bo2-volkz-'):
        label = label.replace('BO2 · Volkz · ', 'BO2 · ')
        if item['id'] in flags:
            country = re.sub(r'-[a-f0-9]{8}$','',item['id'].removeprefix('bo2-volkz-flag-'))
            label = 'BO2 · '+reviewed.get(country,f'Custom Artwork {flags.index(item["id"])+1:03}')
        else:
            for before,after in {'Kakashi&Obito':'Kakashi & Obito','Dayofdead':'Day of the Dead','Dragonfire':'Dragon Fire','Partyrock':'Party Rock','Seasonpass':'Season Pass','Elite1':'Elite 1','Elite2':'Elite 2'}.items():
                label = label.replace(before,after)
    if label != item['name']:
        item.setdefault('imported_display_name',item['name'])
        item['name'] = label
        changed.append(item['id'])
    before = original[item['id']]
    assert all(item[key] == value for key,value in before.items() if key not in {'name','imported_display_name'})
assert len({item['id'] for item in catalog['items']}) == len(original)
assert not any('\u00c2' in item['name'] for item in catalog['items'])
path.write_text(json.dumps(catalog,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
receipt = {'changed_labels':len(changed),'catalog_items':len(original),'ids_assets_odds_unchanged':True,'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'reviewed_custom_labels':len(reviewed),'other_custom_titles':'Neutral numbered labels; filenames were inaccurate country names, not verified game titles'}
(root/'target/review-followup').mkdir(parents=True,exist_ok=True)
(root/'target/review-followup/cosmetic-labels.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf-8')
print(json.dumps(receipt))
