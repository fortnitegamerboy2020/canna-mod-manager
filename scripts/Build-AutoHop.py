"""Package original Canna Auto-Hop source as a self-contained L4D2 VPK."""
from pathlib import Path
import ast, hashlib, json, struct, zipfile, zlib

root = Path(__file__).resolve().parents[1]
source = root / 'mods/CannaAutoHop'
out = root / 'server/staging-autohop'
out.mkdir(exist_ok=True)
# Reuse the tested VPK writer without running the network catalog builder.
tree = ast.parse((root/'scripts/Prepare-SourceCatalog.py').read_text(encoding='utf-8'))
function = next(node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name == 'vpk')
scope = {'Path': Path, 'struct': struct, 'zlib': zlib}
exec(compile(ast.Module(body=[function], type_ignores=[]), '<vpk-writer>', 'exec'), scope)
files = {name: (source/name).read_bytes() for name in ['canna_autohop.nut', 'canna_autohop.cfg', 'README.md']}
license = (root/'LICENSE').read_bytes()
packed = scope['vpk']({
    'scripts/vscripts/canna_autohop.nut': files['canna_autohop.nut'],
    'cfg/canna_autohop.cfg': files['canna_autohop.cfg'],
    'canna-source/LICENSE': license,
    'canna-source/README.md': files['README.md'],
    'addoninfo.txt': b'"AddonInfo" { "addonSteamAppID" "550" "addontitle" "Canna Auto-Hop" "addonauthor" "Canna contributors" "addonversion" "0.1.0-preview" }',
})
archive = out/'Canna-Auto-Hop.zip'
with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as target:
    for name, data in [('addon.vpk',packed),('LICENSE',license),('README.md',files['README.md'])]:
        info=zipfile.ZipInfo(name,date_time=(2026,10,6,0,0,0));info.compress_type=zipfile.ZIP_DEFLATED;target.writestr(info,data)
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
details = {
    'provider':'canna','game':'Left 4 Dead 2','content_type':'mod', 'license':'MIT',
    'authors':'Canna contributors', 'author_links':[{'name':'Canna contributors','url':'https://github.com/fortnitegamerboy2020/canna-mod-manager'}],
    'source_url':'https://github.com/fortnitegamerboy2020/canna-mod-manager/tree/main/mods/CannaAutoHop',
    'dependencies':[], 'preview':True,
    'install_notes':'Launch modded (-insecure), host a local map, then enter sv_cheats 1 and script_execute canna_autohop in the host console. Repeat the script command after map changes. Local hold-to-hop is verified; physical controller and friends multiplayer prediction remain unverified.',
}
manifest=[{'app_id':550,'name':'Canna Auto-Hop','version':'0.1.0-preview','description':files['README.md'].decode(),
           'origin':'canna:autohop:0.1.0-preview:'+digest, 'local_file':archive.name,'sha256':digest,'details':details}]
(out/'catalog-import.json').write_text(json.dumps(manifest,indent=2),encoding='utf-8')
(out/'addon.vpk').write_bytes(packed)
print('Built Canna Auto-Hop preview:',digest)
