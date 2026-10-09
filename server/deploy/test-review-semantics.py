"""Inert positive/negative source corpus, never executed."""
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('context', Path(__file__).with_name('review_context.py'))
context = importlib.util.module_from_spec(spec)
spec.loader.exec_module(context)

def analyze(*sources, incomplete=False):
    report = {'files':[], 'findings':[], 'observations':[]}
    for i, text in enumerate(sources):
        name = f'archive/{i}.cs'
        findings, observations = context.scan_source(text,name,'.cs')
        report['files'].append({'name':name,'text':text,'language':'csharp'})
        report['findings'] += findings
        report['observations'] += observations
    if incomplete:
        report['findings'].append(context.entry('coverage','Incomplete',None,None,'Fixture','high'))
    context.semantics.classify(report,context)
    return report

OWNER = '''using BepInEx; using System.IO;
public class Mod { public void Awake() { new Fix(new Fix.Settings(Path.Combine(Paths.CachePath,"repair"),Paths.ManagedPath)); } }'''
FIX = '''using System.IO;
internal class Fix(Settings settings) {
 public record Settings(string Cache,string Managed);
 private readonly string originals = Path.Combine(settings.Cache,"originals");
 public void Run() { Directory.CreateDirectory(settings.Cache); Directory.CreateDirectory(originals); File.ReadAllBytes(Path.Combine(settings.Managed,"Assembly-CSharp.dll")); }
}'''

class SemanticsTests(unittest.TestCase):
    def test_loader_reads_and_cache_constructor_are_observations(self):
        r=analyze(OWNER,FIX)
        self.assertFalse(r['findings'])
        self.assertEqual(len(r['observations']),3)
        self.assertTrue(all(f['severity']=='info' for f in r['observations']))
        self.assertTrue(any('constructor' in b['expression'] for f in r['observations'] for p in f['path_classification']['destinations'] for b in p['resolved']['bindings']))
    def test_unknown_primary_constructor_call_blocks_lowering(self):
        r=analyze(OWNER+'\npublic class Evil { void Go() { new Fix(userSettings); } }',FIX)
        self.assertEqual(sum(len(f.get('locations',[f])) for f in r['findings']),3)
    def test_mutable_member_or_field_blocks_lowering(self):
        for bad in [FIX.replace('private readonly','private'),FIX.replace('public void Run() {','public void Run() { settings.Cache = unknown;')]:
            self.assertTrue(analyze(OWNER,bad)['findings'])
    def test_future_local_assignment_does_not_explain_earlier_read(self):
        r=analyze('using BepInEx; public class Mod { void Go(string path) { File.ReadAllBytes(path); path = Paths.ManagedPath; } }')
        self.assertTrue(r['findings'])
    def test_method_and_lambda_parameters_cannot_borrow_field_root(self):
        for body in ['void Run(string path) { File.ReadAllBytes(path); }',
                     'void Run() { string path; File.ReadAllBytes(path); }',
                     'void Run() { Func<string, byte[]> read = path => File.ReadAllBytes(path); }']:
            r=analyze('using BepInEx; class Mod { readonly string path = Paths.ManagedPath; '+body+' }')
            self.assertTrue(r['findings'])
    def test_custom_record_getter_is_not_a_generated_constructor_binding(self):
        bad=FIX.replace('public record Settings(string Cache,string Managed);','public record Settings(string Cache,string Managed) { public string Cache { get { return Evil(); } } }')
        self.assertTrue(analyze(OWNER,bad)['findings'])
    def test_symlink_traversal_absolute_dynamic_and_sensitive_paths_stay_review(self):
        for segment in ['".."','"../outside"','"C:/Startup"','"\\\\server\\\\share"','input','GetPath()','"Login Data"']:
            r=analyze('using BepInEx; public class Mod { public void Go() { File.ReadAllBytes(Path.Combine(Paths.CachePath,'+segment+')); } }')
            self.assertTrue(r['findings'],segment)
    def test_shadows_aliases_and_incomplete_coverage_block_lowering(self):
        for extra in ['public class Paths {}','using Paths = Evil.Paths;','public class Extra { object Paths; }']:
            self.assertTrue(analyze(OWNER+'\n'+extra,FIX)['findings'])
        self.assertTrue(analyze(OWNER,FIX,incomplete=True)['findings'])
    def test_write_delete_copy_replace_and_readwrite_stream_not_cleared(self):
        for op in ['File.WriteAllBytes(Path.Combine(Paths.CachePath,"code.dll"),bytes)',
                   'File.AppendAllText(Path.Combine(Paths.CachePath,"index.tsv"),text)',
                   'File.Delete(Path.Combine(Paths.CachePath,"data.json"))',
                   'File.Replace(old,newPath,backup)',
                   'File.Copy(source,Path.Combine(Paths.CachePath,"new.dll"))',
                   'new FileStream(Path.Combine(Paths.CachePath,"data"),FileMode.Open,FileAccess.ReadWrite)']:
            self.assertTrue(analyze('using BepInEx; class Mod { public void Run() { '+op+'; } }')['findings'],op)
    def test_every_operation_on_same_line_is_retained(self):
        r=analyze('using BepInEx; class Mod { public void Go() { File.ReadAllBytes(Path.Combine(Paths.ManagedPath,"Assembly-CSharp.dll")); File.WriteAllBytes(unknown,bytes); File.Move(temp,dest); } }')
        self.assertEqual(len(r['observations']),1)
        self.assertEqual(len(r['findings']),2)
        self.assertEqual({f['operation'] for f in r['findings']},{'File.WriteAllBytes','File.Move'})
    def test_read_mode_is_exact_not_any_read_token_in_arguments(self):
        read='new FileStream(Path.Combine(Paths.ManagedPath,"Assembly-CSharp.dll"),FileMode.Open,FileAccess.Read)'
        self.assertFalse(analyze('using BepInEx; class Mod { public void Run() { '+read+'; } }')['findings'])
        for args in ['FileMode.Create,FileAccess.Read','mode,FileAccess.Read','FileMode.Open,FileAccess.ReadWrite','FileMode.OpenOrCreate,FileAccess.Read']:
            self.assertTrue(analyze('using BepInEx; class Mod { public void Run() { new FileStream(Path.Combine(Paths.CachePath,"code.dll"),'+args+'); } }')['findings'])
        self.assertTrue(analyze('using BepInEx; class Mod { public void Run() { new FileStream(Paths.ManagedPath,FileMode.Open,FileAccess.Read,FileShare.Read,4096,FileOptions.DeleteOnClose); } }')['findings'])
        self.assertTrue(analyze('using BepInEx; class Mod { object FileAccess; public void Run() { new FileStream(Paths.ManagedPath,FileMode.Open,FileAccess.Read); } }')['findings'])
    def test_no_mod_name_or_cache_word_allowlist(self):
        r=analyze('public class DuctTape { public void Go() { Directory.CreateDirectory(cache); File.ReadAllBytes(assemblyCSharp); } }')
        self.assertEqual(len(r['findings']),2)
    def test_ordinary_info_does_not_remove_process_network_load_findings(self):
        r=analyze('using BepInEx; class Mod { public void Go() { Directory.CreateDirectory(Path.Combine(Paths.CachePath,"repair")); Process.Start(exe); Assembly.Load(payload); new HttpClient(); } }')
        self.assertEqual({f['rule'] for f in r['findings']},{'commands','dynamic','network'})
    def test_capability_group_keeps_sites_and_changes_identity_on_any_change(self):
        a=context.entry('dynamic','Reflection-based instance construction','decompiled/archive/library.dll/One.cs',3,'Activator.CreateInstance(first)')
        b=context.entry('dynamic','Reflection-based instance construction','decompiled/archive/library.dll/Two.cs',4,'Activator.CreateInstance(second)')
        grouped=context.semantics.group_capabilities([a,b])
        self.assertEqual(len(grouped),1)
        self.assertEqual(len(grouped[0]['locations']),2)
        self.assertNotIn('accepted',grouped[0])
        changed=context.entry('dynamic','Reflection-based instance construction','decompiled/archive/library.dll/Two.cs',4,'Activator.CreateInstance(other)')
        self.assertNotEqual(grouped[0]['id'],context.semantics.group_capabilities([a,changed])[0]['id'])

if __name__=='__main__':unittest.main(verbosity=2)
