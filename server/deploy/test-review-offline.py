"""Run the revised scanner outside the live worker/queue against static inputs.

Uses existing Linux analysis tools. Does not launch any submitted binary, write
the production job spool, approve mods, install tools or restart a service.
Detailed results stay in the explicitly supplied scratch output directory.
"""
import argparse
from collections import Counter
import hashlib
import importlib.util
import json
import shutil
import subprocess
import tempfile
from pathlib import Path
import zipfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--fixture', action='store_true')
parser.add_argument('archives', nargs='*', type=Path)
args = parser.parse_args()
spec = importlib.util.spec_from_file_location('review_worker', Path(__file__).with_name('review-worker.py'))
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)
args.output.mkdir(parents=True, exist_ok=True)
summaries = []


def review(archive, label):
    with tempfile.TemporaryDirectory(prefix='canna-offline-analysis-', dir=args.output) as tmp:
        job = Path(tmp)
        shutil.copyfile(archive, job / 'input.zip')
        result = worker.analyze(job)
    result['sha256'] = hashlib.sha256(archive.read_bytes()).hexdigest()
    summary = {'label': label, 'sha256': result['sha256'], 'version': result['version'],
               'status': result['status'], 'coverage_complete': result['coverage_complete'],
               'source_files': len(result['files']),
               'findings': dict(Counter(f['rule'] for f in result['findings'])),
               'observations': dict(Counter(f['rule'] for f in result['observations'])),
               'engines': result['engines']}
    (args.output / (label + '.json')).write_text(json.dumps(result), encoding='utf-8')
    summaries.append(summary)
    print(json.dumps(summary), flush=True)
    return result


if args.fixture:
    with tempfile.TemporaryDirectory(prefix='canna-inert-fixture-', dir=args.output) as tmp:
        root = Path(tmp)
        project = root / 'fixture'
        project.mkdir()
        (project / 'fixture.csproj').write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>', encoding='utf-8')
        (project / 'Example.cs').write_text('''namespace ReviewFixture;
public class Example {
 public string SourceUrl() { return "https://example.invalid/project"; }
 public void NeverRun() { System.Diagnostics.Process.Start("fixture-only-never-run"); }
 public void UnknownDirectory(string path) { System.IO.File.WriteAllText(System.IO.Path.Combine(path,"report.txt"),"fixture"); }
}''', encoding='utf-8')
        subprocess.run(['dotnet', 'build', str(project), '-c', 'Release', '--nologo', '-v', 'quiet'], check=True, stdout=subprocess.DEVNULL)
        fixture = root / 'managed-fixture.zip'
        with zipfile.ZipFile(fixture, 'w') as archive:
            archive.write(project / 'bin/Release/net8.0/fixture.dll', 'fixture.dll')
            # The standard harmless antivirus test string is never executable.
            archive.writestr('antivirus-test.txt', b'X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*')
        result = review(fixture, 'managed-eicar-fixture')
        assert any(f['kind'] == 'decompiled' and 'Example' in f['name'] for f in result['files'])
        assert any(f['rule'] == 'commands' for f in result['findings'])
        assert any(f['rule'] == 'filesystem' for f in result['findings'])
        assert any(f['rule'] == 'signature' for f in result['findings'])
        assert any(o['rule'] == 'url-reference' for o in result['observations'])
        assert result['engines']['clamav']['status'] == 'complete'
        assert result['engines']['detect-it-easy']['status'] == 'complete'
        print('PASS: real decompilation/DiE/ClamAV fixture, unknown path retained, URL reference separated, no fixture binary executed.', flush=True)

for archive in args.archives:
    review(archive, archive.stem)
(args.output / 'summary.json').write_text(json.dumps(summaries, indent=2), encoding='utf-8')
