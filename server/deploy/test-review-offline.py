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
import struct
import subprocess
import tempfile
from pathlib import Path
import zipfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--fixture', action='store_true')
parser.add_argument('--java-fixture', action='store_true')
parser.add_argument('--mixed-fixture', action='store_true')
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


def inert_java_class():
    """Valid Java 8 class: constructor and a constant-return method; never run.

    Writing a small class fixture directly avoids installing a Java compiler.
    The class contains no file/network/process access or initialization hook.
    """
    def utf8(value):
        raw = value.encode('utf-8')
        return b'\x01' + struct.pack('>H', len(raw)) + raw

    constants = [utf8('ReviewJavaFixture'), b'\x07\x00\x01',
                 utf8('java/lang/Object'), b'\x07\x00\x03',
                 utf8('<init>'), utf8('()V'), utf8('Code'),
                 b'\x0c\x00\x05\x00\x06', b'\x0a\x00\x04\x00\x08',
                 utf8('Label'), utf8('()Ljava/lang/String;'),
                 utf8('fixture-only-never-run'), b'\x08\x00\x0c']
    header = b'\xca\xfe\xba\xbe' + struct.pack('>HHH', 0, 52, len(constants) + 1)

    def method(flags, name, descriptor, max_locals, bytecode):
        code = struct.pack('>HHI', 1, max_locals, len(bytecode)) + bytecode + struct.pack('>HH', 0, 0)
        return (struct.pack('>HHHH', flags, name, descriptor, 1) +
                struct.pack('>HI', 7, len(code)) + code)

    return (header + b''.join(constants) + struct.pack('>HHHHHH', 0x21, 2, 4, 0, 0, 2) +
            method(1, 5, 6, 1, b'\x2a\xb7\x00\x09\xb1') +
            method(9, 10, 11, 0, b'\x12\x0d\xb0') + struct.pack('>H', 0))


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
        reconstruction = next(item for item in result['decompilations']
                              if item['input'] == 'archive/fixture.dll')
        assert reconstruction['tool'] == 'ilspycmd'
        assert reconstruction['status'] == 'complete', reconstruction
        assert reconstruction['generated_count'] >= 1
        assert reconstruction['generated_count'] == reconstruction['preview_count']
        assert all(item['origin'] == 'archive/fixture.dll'
                   for item in result['files'] if item['kind'] == 'decompiled')
        assert all(item['name'] in reconstruction['generated_files']
                   for item in result['files'] if item['kind'] == 'decompiled')
        print('PASS: real decompilation/DiE/ClamAV fixture, unknown path retained, URL reference separated, no fixture binary executed.', flush=True)

if args.java_fixture:
    with tempfile.TemporaryDirectory(prefix='canna-inert-java-fixture-', dir=args.output) as tmp:
        root = Path(tmp)
        fixture = root / 'java-project-fixture.zip'
        class_bytes = inert_java_class()
        with zipfile.ZipFile(fixture, 'w') as archive:
            archive.writestr('ReviewJavaFixture.class', class_bytes)
        result = review(fixture, 'java-project-fixture')
        reconstructed = next(item for item in result['files']
                             if 'fixture-only-never-run' in item['text'])
        record = result['decompilations'][0]
        assert record['status'] == 'complete', record
        assert record['scope'] == 'project'
        assert record['inputs'] == ['archive/ReviewJavaFixture.class']
        assert reconstructed['origins'] == record['inputs']
        assert 'does not establish' in record['mapping_note']
        jar = root / 'fixture.jar'
        with zipfile.ZipFile(jar, 'w') as archive:
            archive.writestr('ReviewJavaFixture.class', class_bytes)
        fixture = root / 'java-jar-fixture.zip'
        with zipfile.ZipFile(fixture, 'w') as archive:
            archive.write(jar, 'plugins/fixture.jar')
        result = review(fixture, 'java-jar-fixture')
        record = result['decompilations'][0]
        assert record['status'] == 'complete', record
        assert record['scope'] == 'binary'
        assert record['input'] == 'archive/plugins/fixture.jar'
        assert any('fixture-only-never-run' in item['text'] for item in result['files'])
        assert record['dependency_resolution']['supplied_directories'] == ['archive/plugins']
        print('PASS: real CFR project and JAR reconstruction/provenance; no Java class executed or compiler installed.', flush=True)

if args.mixed_fixture:
    with tempfile.TemporaryDirectory(prefix='canna-inert-mixed-fixture-', dir=args.output) as tmp:
        root = Path(tmp)
        project = root / 'fixture'
        project.mkdir()
        (project / 'fixture.csproj').write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>', encoding='utf-8')
        (project / 'Example.cs').write_text('namespace ReviewFixture; public class Example { public string Label() { return "managed-fixture-only-never-run"; } }', encoding='utf-8')
        nuget_config = root / 'NuGet.Config'
        nuget_config.write_text('<configuration><packageSources><clear /></packageSources></configuration>', encoding='utf-8')
        subprocess.run(['dotnet', 'restore', str(project), '--configfile', str(nuget_config), '--nologo', '-v', 'quiet'], check=True, stdout=subprocess.DEVNULL)
        subprocess.run(['dotnet', 'build', str(project), '--no-restore', '-c', 'Release', '--nologo', '-v', 'quiet'], check=True, stdout=subprocess.DEVNULL)
        fixture = root / 'mixed-java-managed-fixture.zip'
        with zipfile.ZipFile(fixture, 'w') as archive:
            archive.writestr('ReviewJavaFixture.class', inert_java_class())
            archive.write(project / 'bin/Release/net8.0/fixture.dll', 'ManagedDisguised.class')
        result = review(fixture, 'mixed-java-managed-fixture')
        managed = next(item for item in result['decompilations']
                       if item['input'] == 'archive/ManagedDisguised.class')
        assert managed['tool'] == 'ilspycmd', managed
        assert managed['status'] == 'complete', managed
        assert managed['generated_count'] >= 1
        assert any(item['origin'] == 'archive/ManagedDisguised.class' and
                   'managed-fixture-only-never-run' in item['text'] for item in result['files'])
        java = next(item for item in result['decompilations'] if item['scope'] == 'project')
        assert java['inputs'] == ['archive/ReviewJavaFixture.class'], java
        assert java['status'] == 'complete', java
        assert any('fixture-only-never-run' in item['text'] for item in result['files'])
        print('PASS: real ILSpy reconstructs PE renamed .class alongside real CFR Java project; no fixture code executed or package source used.', flush=True)

for archive in args.archives:
    review(archive, archive.stem)
(args.output / 'summary.json').write_text(json.dumps(summaries, indent=2), encoding='utf-8')
