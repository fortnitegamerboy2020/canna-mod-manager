"""Context regressions: inert snippets only, never run submitted code."""
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('review_context', Path(__file__).with_name('review_context.py'))
context = importlib.util.module_from_spec(spec)
spec.loader.exec_module(context)


def scan(source, suffix='.cs'):
    return context.scan_source(source, 'archive/Fixture' + suffix, suffix)


def rules(source, suffix='.cs'):
    return {f['rule'] for f in scan(source, suffix)[0]}


DIAGNOSTIC = '''public class Example {
private string output;
public static void TryStart(GameObject root) {
 string[] commandLineArgs = Environment.GetCommandLineArgs();
 for (int i = 0; i + 1 < commandLineArgs.Length; i++) {
  if (commandLineArgs[i] == "--diagnostics") {
   var runner = root.AddComponent<Example>();
   runner.output = commandLineArgs[i + 1];
   runner.StartCoroutine(runner.Run());
  }
 }
}
private IEnumerator Run() {
 Directory.CreateDirectory(output);
 File.WriteAllText(Path.Combine(output, "report.txt"), report.ToString());
 File.WriteAllLines(Path.Combine(output, "native-shaders.txt"), shaders);
}
private void Capture(string name) {
 File.WriteAllBytes(Path.Combine(output, name + ".png"), bytes);
}
}'''


class ContextTests(unittest.TestCase):
    def test_ui_open_does_not_masquerade_as_python_file_access(self):
        self.assertEqual(rules('menu.Open(); popup.open();'), set())
        self.assertIn('filesystem', rules('data = open("fixture.txt", "r").read()', '.py'))

    def test_comments_are_not_behavior_but_trailing_code_is(self):
        source = '/* HttpClient File.Delete("x") */\n// Process.Start("x")\nFile.ReadAllText("x"); // https://docs.invalid\n/* more */ Process.Start("fixture");'
        findings, observations = scan(source)
        self.assertEqual([(f['rule'], f['line']) for f in findings], [('filesystem', 3), ('commands', 4)])
        self.assertFalse(observations)
        self.assertFalse(scan('// trailing comment without newline')[0])

    def test_rust_lifetimes_do_not_hide_following_file_operations(self):
        findings, _ = scan("fn remove<'a>(s: &'a str) { std::fs::remove_file(s); }", '.rs')
        self.assertEqual({f['rule'] for f in findings}, {'filesystem'})

    def test_multiline_and_verbatim_literals_are_not_api_calls(self):
        self.assertEqual(rules('string x = @"First line\nFile.Delete(""fixture"")\nHttpClient";'), set())
        self.assertEqual(rules('x = """Process.Start()\nFile.Delete()"""', '.py'), set())

    def test_interpolation_expressions_are_not_hidden(self):
        self.assertIn('identity', rules('var x = $"{Environment.UserName}";'))
        self.assertIn('commands', rules('x = f"{subprocess.run(command)}"', '.py'))
        self.assertIn('network', rules('const x = `${fetch(url)}`;', '.js'))

    def test_multiline_calls_retain_source_location(self):
        findings, _ = scan('File\n .\n WriteAllText("report.txt", text);\nEnvironment\n .UserName;')
        self.assertEqual([(f['rule'], f['line']) for f in findings], [('identity', 4), ('filesystem', 1)])

    def test_url_references_are_visible_not_network_requests(self):
        findings, observations = scan('string url = "https://media.invalid/movie.mp4";')
        self.assertFalse(findings)
        self.assertEqual(observations[0]['rule'], 'url-reference')
        self.assertIn('elsewhere', observations[0]['context'])
        self.assertIn('network', rules('HttpClient client; client.GetAsync(url);'))

    def test_nonexecutables_in_sensitive_paths_are_still_flagged(self):
        self.assertEqual(rules('File.ReadAllText(@"C:\\Users\\x\\.ssh\\config.txt");'), {'filesystem', 'sensitive-files'})
        self.assertIn('filesystem', rules('File.WriteAllText(Path.Combine(userPath, "report.txt"), text);'))

    def test_scoped_diagnostics_are_summarized_but_still_require_review(self):
        findings, _ = scan(DIAGNOSTIC)
        grouped = context.contextualize_file_operations(DIAGNOSTIC, findings)
        self.assertEqual(len(grouped), 1)
        self.assertEqual(grouped[0]['rule'], 'diagnostic-output')
        self.assertEqual(grouped[0]['severity'], 'review')
        self.assertNotIn('accepted', grouped[0])
        self.assertEqual(len(grouped[0]['locations']), 4)
        self.assertIn('not confined', grouped[0]['context'])

    def test_executable_delete_copy_and_unknown_writes_do_not_get_diagnostic_label(self):
        for extra in ('File.Delete(output);', 'File.Copy(source, output);',
                      'File.WriteAllBytes(Path.Combine(output, "payload.dll"), data);',
                      'File.WriteAllText(destination, text);'):
            source = DIAGNOSTIC + '\n' + extra
            grouped = context.contextualize_file_operations(source, scan(source)[0])
            self.assertFalse(any(f['rule'] == 'diagnostic-output' for f in grouped), extra)
            self.assertIn('filesystem', {f['rule'] for f in grouped})

    def test_variable_names_and_extensions_do_not_prove_guarded_execution(self):
        for source in (DIAGNOSTIC.replace('Environment.GetCommandLineArgs()', 'GetUserPaths()'),
                       DIAGNOSTIC.replace('if (commandLineArgs[i] == "--diagnostics")', 'if (true)')):
            grouped = context.contextualize_file_operations(source, scan(source)[0])
            self.assertTrue(all(f['rule'] == 'filesystem' for f in grouped))

    def test_same_line_harmful_operation_prevents_diagnostic_summary(self):
        source = DIAGNOSTIC.replace('Directory.CreateDirectory(output);',
                                    'Directory.CreateDirectory(output); File.Delete(Path.Combine(output, "other.txt"));')
        grouped = context.contextualize_file_operations(source, scan(source)[0])
        self.assertFalse(any(f['rule'] == 'diagnostic-output' for f in grouped))
        self.assertIn('File.Delete', grouped[0]['evidence'])

    def test_changed_output_evidence_invalidates_group_identity(self):
        initial = context.contextualize_file_operations(DIAGNOSTIC, scan(DIAGNOSTIC)[0])[0]
        source = DIAGNOSTIC.replace('report.txt', 'diagnostic.txt')
        changed = context.contextualize_file_operations(source, scan(source)[0])[0]
        self.assertNotEqual(initial['id'], changed['id'])

    def test_native_dynamic_and_persistence_are_not_cleared_by_context(self):
        self.assertEqual(rules('Assembly.Load(bytes);\nCreateRemoteThread(handle);\nvar p = @"Software\\Microsoft\\Windows\\CurrentVersion\\Run";'), {'dynamic', 'native', 'privileges'})


if __name__ == '__main__':
    unittest.main(verbosity=2)
