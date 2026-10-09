"""Replay saved source previews locally; does not run binary/antivirus tools.

Output status stays preview and cannot be ingested as a completed worker scan.
Existing binary/coverage findings are retained without staff decision fields.
"""
import argparse
from collections import Counter
import importlib.util
import json
from pathlib import Path

root = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('review_context', root / 'server/deploy/review_context.py')
context = importlib.util.module_from_spec(spec)
spec.loader.exec_module(context)
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('report', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
original = json.loads(args.report.read_text(encoding='utf-8-sig'))
findings, observations = [], []
preview_names = {file['name'] for file in original.get('files', [])}
for finding in original.get('findings', []):
    if finding['rule'] == 'coverage' or not finding.get('line') or finding.get('file') not in preview_names:
        findings.append({k: v for k, v in finding.items() if k not in {'accepted', 'reason', 'reviewed', 'reviewer'}})
for file in original.get('files', []):
    suffix = Path(file['name']).suffix.lower()
    if suffix == '.md' and not context.markdown_has_code(file['text']):
        continue
    found, observed = context.scan_source(file['text'], file['name'], suffix)
    if suffix == '.cs':
        found = context.contextualize_file_operations(file['text'], found)
        found = context.trace_operations(file['text'], found)
    findings.extend(found)
    observations.extend(observed)
summary = {'mod': original.get('mod_name'), 'sha256': original.get('sha256'),
           'source_files_replayed': len(original.get('files', [])),
           'previous_findings': dict(Counter(f['rule'] for f in original['findings'])),
           'preview_findings': dict(Counter(f['rule'] for f in findings)),
           'observations': dict(Counter(f['rule'] for f in observations)),
           'verification': 'Saved source previews only; binary tools and antivirus were not rerun.'}
preview = {'status': 'preview', 'version': 'canna-static-9-preview', 'mod_name': original.get('mod_name'),
           'sha256': original.get('sha256'), 'files': original.get('files', []),
           'findings': findings, 'observations': observations, 'summary': summary}
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(preview, indent=2), encoding='utf-8')
print(json.dumps(summary, indent=2))
