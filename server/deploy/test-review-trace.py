"""Inert source fixtures; submitted code is never executed."""
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('context', Path(__file__).with_name('review_context.py'))
context = importlib.util.module_from_spec(spec); spec.loader.exec_module(context)

def traced(source):
    findings, _ = context.scan_source(source, 'fixture.cs', '.cs')
    return context.trace_operations(source, findings)

class TraceTests(unittest.TestCase):
    def test_network_field_is_observation_but_client_request_remains_review(self):
        source = '''public class Mod {
private readonly HttpClient client;
public void Send(string destination) {
 client.GetAsync(destination);
}
}'''
        findings, observations = context.scan_source(source, 'fixture.cs', '.cs')
        self.assertEqual([(f['rule'],f['line']) for f in findings],[('network',4)])
        self.assertEqual(observations[0]['line'],2)
        self.assertEqual(observations[0]['rule'],'declaration-reference')
        self.assertEqual(traced(source)[0]['trace']['method'],'Send')
        same_line, _ = context.scan_source('public static string FromBase64String(string value) { return Convert.FromBase64String(value); }','fixture.cs','.cs')
        self.assertEqual(len(same_line),1,'A declaration must not hide a real call on the same line')
    def test_helper_argument_and_destination_assignments(self):
        source = '''public class Mod {
public void Awake() {
 string root = "plugins";
 string destination = Path.Combine(root, "replacement.dll");
 Replace(destination, bytes);
}
private void Replace(string path, byte[] data) {
 string temporary = path + ".rptmp";
 File.WriteAllBytes(temporary, data);
}
}'''
        finding = traced(source)[0]
        self.assertEqual(finding['trace']['method'], 'Replace')
        self.assertEqual(finding['severity'], 'high')
        self.assertEqual(finding['trace']['callers'][0]['method'], 'Awake')
        self.assertTrue(all(caller['method'] != 'unresolved' for caller in finding['trace']['callers']), 'Method declarations are not callers')
        assignments = finding['trace']['paths'][0]['assignments']
        self.assertTrue(any('replacement.dll' in a['expression'] for a in assignments))
        self.assertFalse(finding['trace']['paths'][0]['resolved'])
        self.assertNotIn('accepted', finding)

    def test_primary_constructor_is_not_enclosing_method_for_fields(self):
        source = '''internal sealed class Mod(string root) {
private readonly string output = Path.Combine(root, "plugin.dll");
private void Write(byte[] bytes) {
 File.WriteAllBytes(output, bytes);
}
}'''
        finding = traced(source)[0]
        self.assertEqual(finding['trace']['method'],'Write')
        self.assertEqual(finding['severity'],'high')

    def test_directory_caller_and_unknown_path_not_accepted(self):
        finding = traced('''public class Mod {
public void Awake() { Setup(userPath); }
private void Setup(string folder) { Directory.CreateDirectory(folder); }
}''')[0]
        self.assertEqual(finding['trace']['effect'], 'directory-creation')
        self.assertEqual(finding['trace']['method'], 'Setup')
        self.assertEqual(finding['trace']['paths'][0]['assignments'][0]['expression'], 'userPath')
        self.assertNotIn('accepted', finding)

    def test_repeated_operations_group_without_omitting_evidence(self):
        findings = traced('''public class Mod {
private void Read() {
 File.ReadAllText(first);
 File.ReadAllText(second);
}
}''')
        self.assertEqual(len(findings), 1)
        self.assertEqual(len(findings[0]['locations']), 2)
        changed = traced('''public class Mod {
private void Read() {
 File.ReadAllText(first);
 File.ReadAllText(third);
}
}''')[0]
        self.assertNotEqual(findings[0]['id'], changed['id'])

    def test_comments_literals_ambiguity_sensitive_and_dynamic_survive(self):
        source = '''public class Mod {
private void Setup(string folder) { Directory.CreateDirectory(folder); }
private void Setup(int folder) { File.Delete(GetPath(folder)); }
public void Awake() { Setup(unknown); Assembly.Load(payload); File.ReadAllText("Login Data"); }
}'''
        findings = traced(source)
        self.assertTrue(any(f['rule']=='sensitive-files' for f in findings))
        self.assertTrue(any(f['rule']=='dynamic' for f in findings))
        self.assertTrue(any(c['ambiguous'] for f in findings for c in f.get('trace',{}).get('callers',[])))
        self.assertEqual(traced('// File.Delete(x);\nvar message="Directory.CreateDirectory(x)";'), [])

    def test_readonly_dll_stream_is_not_executable_write(self):
        finding = traced('''public class Mod {
private void Read() { var s = new FileStream("plugin.dll", FileMode.Open, FileAccess.Read); }
}''')[0]
        self.assertEqual(finding['severity'], 'review')
        self.assertIn('File read', finding['title'])

    def test_method_name_declaration_is_not_a_decode_call(self):
        source = '''public static string FromBase64String(this string encoded) {
 return Encoding.UTF8.GetString(Convert.FromBase64String(encoded));
}'''
        findings, observations = context.scan_source(source, 'fixture.cs', '.cs')
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0]['line'], 2)
        self.assertEqual(observations[0]['rule'], 'declaration-reference')

if __name__ == '__main__': unittest.main(verbosity=2)
