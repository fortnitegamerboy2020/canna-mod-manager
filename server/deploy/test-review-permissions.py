"""Run as the API user to verify the production handoff to the isolated worker."""
import grp, json, os, shutil, stat, time, uuid, zipfile
from pathlib import Path
os.umask(0o077)
root = Path('/var/lib/canna-review/jobs')
job = root / str(uuid.uuid4())
created = False
try:
    job.mkdir()
    created = True
    # Mirror the real API handoff: the spool supplies the directory group,
    # while RestrictSUIDSGID rejects setting the setgid bit on a job.
    os.chmod(job, 0o770)
    gid = job.stat().st_gid
    assert stat.S_IMODE(job.stat().st_mode) == 0o770
    assert gid == grp.getgrnam('canna-review').gr_gid
    with zipfile.ZipFile(job / 'input.zip', 'w') as archive:
        archive.writestr('PermissionFixture.cs', '// Harmless source review fixture\npublic class PermissionFixture { public string Name => "permission-handoff-ok"; }')
    os.chown(job / 'input.zip', -1, gid)
    os.chmod(job / 'input.zip', 0o660)
    assert (job / 'input.zip').stat().st_gid == gid
    (job / 'ready').write_text('ready', encoding='utf-8')
    os.chown(job / 'ready', -1, gid)
    os.chmod(job / 'ready', 0o660)
    assert (job / 'ready').stat().st_gid == gid
    until = time.monotonic() + 90
    while not (job / 'result.json').exists() and time.monotonic() < until:
        time.sleep(1)
    assert (job / 'result.json').stat().st_gid == gid
    assert stat.S_IMODE((job / 'result.json').stat().st_mode) == 0o660
    result = json.loads((job / 'result.json').read_text(encoding='utf-8'))
    assert result['version'] == 'canna-static-6'
    assert result['status'] == 'complete'
    assert any('permission-handoff-ok' in f.get('text', '') for f in result['files']), result['findings']
    assert not any('Permission denied' in f.get('evidence', '') for f in result['findings'])
    print('PASS: v6 API-user archive with explicit review group readable by isolated worker, source preview and result readable by API user.')
finally:
    if created:
        shutil.rmtree(job)
