"""Run as the API user to verify the production handoff to the isolated worker."""
import grp, json, os, shutil, time, uuid, zipfile
from pathlib import Path
os.umask(0o077)
root = Path('/var/lib/canna-review/jobs')
job = root / str(uuid.uuid4())
job.mkdir()
os.chmod(job, 0o2770)
try:
    with zipfile.ZipFile(job / 'input.zip', 'w') as archive:
        archive.writestr('PermissionFixture.cs', '// Harmless source review fixture\npublic class PermissionFixture { public string Name => "permission-handoff-ok"; }')
    os.chmod(job / 'input.zip', 0o660)
    assert (job / 'input.zip').stat().st_gid == grp.getgrnam('canna-review').gr_gid
    (job / 'ready').write_text('ready')
    os.chmod(job / 'ready', 0o660)
    until = time.monotonic() + 90
    while not (job / 'result.json').exists() and time.monotonic() < until:
        time.sleep(1)
    result = json.loads((job / 'result.json').read_text())
    assert result['status'] == 'complete'
    assert any('permission-handoff-ok' in f.get('text', '') for f in result['files']), result['findings']
    assert not any('Permission denied' in f.get('evidence', '') for f in result['findings'])
    print('PASS: API-user archive readable by isolated worker, source reconstructed, result readable by API user.')
finally:
    shutil.rmtree(job)
