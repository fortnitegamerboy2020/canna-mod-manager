// Offline compatibility preparation only. Never load or execute a mod assembly.
using System.Diagnostics;
using System.Web.Script.Serialization;
using BepInEx.Logging;
using Canna.DuctTapePlusPlus;
using Mono.Cecil;

sealed class Request
{
    public string game_root = "", plugins = "", core = "", patchers = "", config = "";
    public string[] declared_dependencies = new string[0];
}
sealed class Payload
{
    public string assembly = "", file = "", sha256 = "", source = "", license = "";
    public string upstream_sha256 = "";
    public string[] normalization = new string[0];
    public string[] dependencies = new string[0], declared_aliases = new string[0];
}
sealed class PayloadIndex
{
    public string protocol = "", distribution_status = "";
    public Payload[] payloads = new Payload[0];
}
sealed class Translation
{
    public string file = "", before_sha256 = "", after_sha256 = "";
    public string[] changes = new string[0];
}
sealed class Replacement
{
    public string file = "", identity = "", sha256 = "", reason = "";
}
sealed class Report
{
    public bool ok, required;
    public string protocol = ManifestContract.Protocol, profile = ManifestContract.Profile;
    public string game_sha256 = "", manifest_sha256 = "", fingerprint = "";
    public string asset_compatibility = "checked", distribution_status = "local-preview-redistribution-unverified";
    public List<Translation> translated = new();
    public List<Replacement> replacements = new();
    public List<string> warnings = new(), errors = new();
}

// Minimal compatibility surface needed by the unchanged upstream Curated code.
static class AutoFix { public static string Sha(byte[] bytes) => ManifestContract.Hash(bytes); }
static class Out
{
    public static Report? Current;
    public static void Warn(string text) { Current?.warnings.Add(text); }
    public static void Note(string text) { Current?.warnings.Add(text); }
}

static class Program
{
    static readonly JavaScriptSerializer Json = new() { MaxJsonLength = 8 * 1024 * 1024, RecursionLimit = 100 };
    static readonly string Home = AppDomain.CurrentDomain.BaseDirectory;
    const long ByteLimit = 768L * 1024 * 1024;

    static int Main(string[] args)
    {
        var report = new Report();
        Out.Current = report;
        string reportPath = "";
        bool reportPathChecked = false;
        try
        {
            if (args.Length == 7 && args[0] == "--prepare-payloads" && args[1] == "--game" && args[3] == "--core" && args[5] == "--report")
            {
                reportPath = Path.GetFullPath(args[6]);
                var gameRoot = Path.GetFullPath(args[2]);
                var core = Path.GetFullPath(args[4]);
                foreach (var path in new[] { Home, reportPath, gameRoot, core }) ManifestContract.RejectLinks(path);
                if (Within(Home, gameRoot) || Within(Home, core) || Within(reportPath, gameRoot) || Within(reportPath, core)
                    || !Within(reportPath, Path.GetFullPath(Path.Combine(Home, "..", ".."))))
                    throw new InvalidDataException("Build-only payload preparation must remain outside game and core references");
                reportPathChecked = true;
                PreparePayloads(gameRoot, core, report);
            }
            else
            {
            if (args.Length != 4 || args[0] != "--request" || args[2] != "--report")
                throw new InvalidDataException("Usage: --request <request.json> --report <report.json>");
            var requestPath = Path.GetFullPath(args[1]);
            reportPath = Path.GetFullPath(args[3]);
            ManifestContract.RejectLinks(requestPath);
            ManifestContract.RejectLinks(reportPath);
            if (new FileInfo(requestPath).Length > 1024 * 1024)
                throw new InvalidDataException("Request exceeds one MiB");
            var request = Json.Deserialize<Request>(File.ReadAllText(requestPath));
            CheckRequest(request, reportPath);
            reportPathChecked = true;
            Run(request, report);
            }
        }
        catch (Exception error)
        {
            report.ok = false;
            report.errors.Add(error.GetType().Name + ": " + error.Message);
        }
        if (!string.IsNullOrEmpty(reportPath) && reportPathChecked)
        {
            try
            {
                Directory.CreateDirectory(Path.GetDirectoryName(reportPath)!);
                File.WriteAllText(reportPath, Json.Serialize(report) + "\n", new System.Text.UTF8Encoding(false));
            }
            catch (Exception error) { Console.Error.WriteLine("Could not write compatibility report: " + error.Message); return 3; }
        }
        if (!report.ok) foreach (var error in report.errors) Console.Error.WriteLine(error);
        return report.ok ? 0 : 2;
    }

    static bool Within(string child, string root)
    {
        child = Path.GetFullPath(child).TrimEnd('\\', '/');
        root = Path.GetFullPath(root).TrimEnd('\\', '/');
        return child.Equals(root, StringComparison.OrdinalIgnoreCase)
            || child.StartsWith(root + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase);
    }

    static void CheckRequest(Request request, string reportPath)
    {
        if (request == null || string.IsNullOrWhiteSpace(request.game_root)
            || string.IsNullOrWhiteSpace(request.plugins) || string.IsNullOrWhiteSpace(request.core))
            throw new InvalidDataException("Missing game_root, plugins or core");
        request.game_root = Path.GetFullPath(request.game_root);
        request.plugins = Path.GetFullPath(request.plugins).TrimEnd('\\', '/');
        request.core = Path.GetFullPath(request.core);
        foreach (var path in new[] { request.game_root, request.plugins, request.core }) ManifestContract.RejectLinks(path);
        if (!Directory.Exists(request.plugins) || !Directory.Exists(request.core)
            || !File.Exists(Path.Combine(request.core, "BepInEx.dll"))
            || !File.Exists(Path.Combine(request.core, "0Harmony.dll")))
            throw new InvalidDataException("Temporary plugins and complete BepInEx core reference directory are required");
        if (Within(request.plugins, request.game_root) || Within(request.game_root, request.plugins) || Within(request.plugins, request.core)
            || Within(request.core, request.plugins) || Within(reportPath, request.game_root)
            || Within(reportPath, request.core) || Within(reportPath, request.plugins)
            || !Within(reportPath, Path.GetDirectoryName(request.plugins)!))
            throw new InvalidDataException("Preparation may not write into the game, reference core, or plugin report tree");
        var marker = Path.Combine(request.plugins, ".canna-ducttape-staging");
        ManifestContract.RejectLinks(marker);
        if (!File.Exists(marker) || File.ReadAllText(marker).Trim() != ManifestContract.Protocol)
            throw new InvalidDataException("The temporary plugins directory is missing its Canna staging marker");
        foreach (var extra in new[] { request.patchers, request.config }.Where(p => !string.IsNullOrEmpty(p)))
        {
            ManifestContract.RejectLinks(extra);
            if (Within(extra, request.game_root) || Within(extra, request.core) || Within(extra, request.plugins))
                throw new InvalidDataException("Configuration and patchers must be temporary preparation directories");
        }
        if (ManifestContract.ReadFiles(request.patchers).Count > 0)
            throw new InvalidDataException("Uncovered preloaders/patchers are unsupported in the Canna Bliss preview");
        if (request.declared_dependencies == null || request.declared_dependencies.Length > 4096)
            throw new InvalidDataException("Invalid dependency list");
    }

    static void PreparePayloads(string gameRoot, string core, Report report)
    {
        var managed = Game.ManagedDir(gameRoot) ?? throw new InvalidDataException("Current ROUNDS Managed directory not found");
        report.game_sha256 = ManifestContract.Hash(File.ReadAllBytes(Path.Combine(managed, "Assembly-CSharp.dll")));
        if (report.game_sha256 != "20451cc7090908cd1d125f75f06584645d25e898ec234de0a0c2f154e2900668")
            throw new InvalidDataException("Unsupported game build for payload normalization");
        var bundle = Path.GetFullPath(Path.Combine(Home, ".."));
        var indexPath = Path.Combine(bundle, "payloads", "index.json");
        var index = Json.Deserialize<PayloadIndex>(File.ReadAllText(indexPath));
        if (index.protocol != ManifestContract.Protocol || index.payloads == null || index.payloads.Length > 32)
            throw new InvalidDataException("Invalid build payload index");
        var output = Path.Combine(Path.GetDirectoryName(bundle)!, ".payload-normalize-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(output);
        foreach (var payload in index.payloads)
        {
            if (!ManifestContract.IsHash(payload.sha256) || payload.file != payload.sha256 + ".dll"
                || Path.GetFileName(payload.assembly) != payload.assembly)
                throw new InvalidDataException("Invalid build payload filename");
            var bytes = File.ReadAllBytes(Path.Combine(bundle, "payloads", payload.file));
            if (ManifestContract.Hash(bytes) != payload.sha256) throw new InvalidDataException("Build payload hash mismatch");
            var path = Path.Combine(output, payload.assembly + ".dll");
            if (File.Exists(path)) throw new InvalidDataException("Duplicate build payload assembly");
            File.WriteAllBytes(path, bytes);
            payload.upstream_sha256 = payload.sha256;
        }
        AddRuntime(output, report);
        using (var owner = new ResolverOwner(new Game(gameRoot, managed, core, output)))
        {
            foreach (var payload in index.payloads) owner.Game.Resolver.Set(Path.Combine(output, payload.assembly + ".dll"));
            var scanner = new Scanner(owner.Game);
            foreach (var path in ManifestContract.ReadFiles(output).Where(f => f.EndsWith(".dll", StringComparison.OrdinalIgnoreCase)))
            {
                var beforeHash = ManifestContract.Hash(File.ReadAllBytes(path));
                using var module = ModuleDefinition.ReadModule(path, new ReaderParameters { AssemblyResolver = owner.Game.Resolver, InMemory = true });
                var name = module.Assembly.Name.Name;
                var payload = index.payloads.FirstOrDefault(p => p.assembly == name);
                if (payload != null && Path.GetFileNameWithoutExtension(path) != payload.assembly)
                    throw new InvalidDataException("Build payload identity mismatch");
                if (name == "MMHOOK_Assembly-CSharp") continue; // pinned generated current hook delegates
                var extra = CannaFixRules.Apply(module, owner.Game);
                scanner.Scan(module);
                var fixer = new Fixer(module, owner.Game, scanner);
                fixer.Run();
                if (extra.Count > 0 || fixer.Changed)
                {
                    if (module.Assembly.Name.HasPublicKey) throw new InvalidDataException("Cannot normalize strong-named payload without signing key: " + name);
                    using var stream = new MemoryStream(); module.Write(stream); File.WriteAllBytes(path, stream.ToArray());
                }
                var changes = extra.Concat(fixer.Changes).Distinct().ToArray();
                if (payload != null) payload.normalization = changes;
                var afterHash = ManifestContract.Hash(File.ReadAllBytes(path));
                if (beforeHash != afterHash) report.translated.Add(new Translation {
                    file = ManifestContract.Relative(output, path), before_sha256 = beforeHash, after_sha256 = afterHash, changes = changes });
                foreach (var note in fixer.Notes.Where(n => n.StartsWith("MANUAL", StringComparison.Ordinal))) report.errors.Add(name + ": " + note);
            }
        }
        // A fresh resolver reads the complete final dependency set, never cached pre-rewrite metadata.
        using (var owner = new ResolverOwner(new Game(gameRoot, managed, core, output)))
        {
            foreach (var payload in index.payloads) owner.Game.Resolver.Set(Path.Combine(output, payload.assembly + ".dll"));
            CheckDuplicates(output, owner.Game.Resolver);
            CheckHardDependencies(output, owner.Game.Resolver);
            ValidateModules(output, owner.Game, report);
        }
        if (report.errors.Count > 0) return;
        foreach (var payload in index.payloads)
        {
            var bytes = File.ReadAllBytes(Path.Combine(output, payload.assembly + ".dll"));
            payload.sha256 = ManifestContract.Hash(bytes); payload.file = payload.sha256 + ".dll";
            File.WriteAllBytes(Path.Combine(bundle, "payloads", payload.file), bytes);
        }
        foreach (var filename in new[] { "rounds-port.Runtime.dll", "Canna.DuctTapePlusPlus.NetworkGuard.dll" })
        {
            var bytes = File.ReadAllBytes(Path.Combine(output, "DuctTapePlusPlus", filename));
            File.WriteAllBytes(Path.Combine(bundle, "runtime", filename), bytes);
            var replacement = report.replacements.Single(r => r.file == "DuctTapePlusPlus/" + filename);
            replacement.sha256 = ManifestContract.Hash(bytes);
        }
        File.WriteAllText(indexPath, Json.Serialize(index) + "\n", new System.Text.UTF8Encoding(false));
        report.ok = true;
        report.warnings = report.warnings.Distinct().OrderBy(s => s, StringComparer.Ordinal).ToList();
    }

    static void ValidateModules(string output, Game game, Report report)
    {
        var scanner = new Scanner(game);
        foreach (var file in ManifestContract.ReadFiles(output).Where(f => f.EndsWith(".dll", StringComparison.OrdinalIgnoreCase)))
        {
            using var module = ModuleDefinition.ReadModule(file, new ReaderParameters { AssemblyResolver = game.Resolver, ReadingMode = ReadingMode.Immediate, InMemory = true });
            if (module.Assembly.Name.Name == "MMHOOK_Assembly-CSharp") continue; // exact validated current hook payload
            var issues = scanner.Scan(module);
            var relative = ManifestContract.Relative(output, file);
            foreach (var issue in issues.Where(i => i.Fix != Fix.Review)) report.errors.Add(relative + ": residual " + issue.Fix + " " + issue.What + ": " + issue.Detail);
            foreach (var missing in scanner.Unchecked) report.errors.Add(relative + ": unchecked dependency or asset " + missing);
            foreach (var issue in issues.Where(i => i.Fix == Fix.Review)) report.warnings.Add(relative + ": REVIEW " + issue.What + ": " + issue.Detail);
        }
    }

    static void Run(Request request, Report report)
    {
        var managed = Game.ManagedDir(request.game_root) ?? throw new InvalidDataException("Current ROUNDS Managed directory not found");
        report.game_sha256 = ManifestContract.Hash(File.ReadAllBytes(Path.Combine(managed, "Assembly-CSharp.dll")));
        if (report.game_sha256 != "20451cc7090908cd1d125f75f06584645d25e898ec234de0a0c2f154e2900668")
            throw new InvalidDataException("Unsupported game build: this preview is pinned to the reviewed public ROUNDS assembly");
        var inputFiles = ManifestContract.ReadFiles(request.plugins);
        var inputHashes = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        long size = 0;
        foreach (var file in inputFiles)
        {
            size += new FileInfo(file).Length;
            if (size > ByteLimit) throw new InvalidDataException("Prepared plugin content exceeds 768 MiB");
            inputHashes.Add(ManifestContract.Relative(request.plugins, file), ManifestContract.Hash(File.ReadAllBytes(file)));
        }
        var working = Path.Combine(Path.GetDirectoryName(request.plugins)!, ".ducttape-work-" + Guid.NewGuid().ToString("N"));
        var output = Path.Combine(working, "plugins");
        Directory.CreateDirectory(output);
        foreach (var file in inputFiles)
        {
            var destination = Path.Combine(output, ManifestContract.Relative(request.plugins, file));
            Directory.CreateDirectory(Path.GetDirectoryName(destination)!);
            File.Copy(file, destination);
            if (ManifestContract.Hash(File.ReadAllBytes(destination)) != inputHashes[ManifestContract.Relative(request.plugins, file)])
                throw new InvalidDataException("Input content changed while it was copied");
        }
        var indexPath = Path.GetFullPath(Path.Combine(Home, "..", "payloads", "index.json"));
        var index = Json.Deserialize<PayloadIndex>(File.ReadAllText(indexPath));
        if (index.protocol != ManifestContract.Protocol || index.payloads.Length > 32)
            throw new InvalidDataException("Invalid pinned replacement index");
        var payloads = index.payloads.ToDictionary(p => p.assembly, StringComparer.Ordinal);
        var payloadBytes = new Dictionary<string, byte[]>(StringComparer.Ordinal);
        foreach (var payload in index.payloads)
        {
            if (!ManifestContract.IsHash(payload.sha256) || payload.file != payload.sha256 + ".dll")
                throw new InvalidDataException("Invalid pinned replacement filename or hash");
            var bytes = File.ReadAllBytes(Path.Combine(Path.GetDirectoryName(indexPath)!, payload.file));
            if (ManifestContract.Hash(bytes) != payload.sha256) throw new InvalidDataException("Replacement payload hash mismatch");
            using var module = ModuleDefinition.ReadModule(new MemoryStream(bytes));
            if (module.Assembly?.Name.Name != payload.assembly) throw new InvalidDataException("Replacement assembly identity mismatch");
            payloadBytes.Add(payload.assembly, bytes);
        }
        var needed = new HashSet<string>(StringComparer.Ordinal);
        var oldHashes = ReadOldHashes();
        var inputs = ManifestContract.ReadFiles(output).Where(f => f.EndsWith(".dll", StringComparison.OrdinalIgnoreCase)).ToList();
        foreach (var filename in new[] { "rounds-port.Runtime.dll", "Canna.DuctTapePlusPlus.NetworkGuard.dll" })
        {
            var bundled = File.ReadAllBytes(Path.GetFullPath(Path.Combine(Home, "..", "runtime", filename)));
            using var expected = ModuleDefinition.ReadModule(new MemoryStream(bundled));
            foreach (var file in inputs.ToArray())
            {
                using var candidate = ModuleDefinition.ReadModule(file, new ReaderParameters { InMemory = true });
                if (candidate.Assembly.Name.Name != expected.Assembly.Name.Name) continue;
                if (candidate.Assembly.Name.FullName != expected.Assembly.Name.FullName
                    || ManifestContract.Hash(File.ReadAllBytes(file)) != ManifestContract.Hash(bundled))
                    throw new InvalidDataException("Unknown compatibility runtime bytes cannot be replaced: " + candidate.Assembly.Name.Name);
                File.Delete(file); inputs.Remove(file);
                report.warnings.Add("Restored exact bundled runtime at canonical path: " + expected.Assembly.Name.Name);
            }
        }
        using var bootstrap = new MapResolver { InMemory = true };
        foreach (var directory in new[] { managed, request.core })
            foreach (var file in Directory.GetFiles(directory, "*.dll")) bootstrap.Add(file);
        foreach (var payload in index.payloads) bootstrap.Add(Path.Combine(Path.GetDirectoryName(indexPath)!, payload.file));
        foreach (var file in inputs) bootstrap.Add(file); // user-provided plugin base classes and inherited loader metadata
        ReaderParameters Metadata() => new ReaderParameters { AssemblyResolver = bootstrap, InMemory = true };
        var provided = new HashSet<string>(StringComparer.Ordinal);
        var providedGuids = new HashSet<string>(StringComparer.Ordinal);
        var payloadGuids = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var pair in payloadBytes)
        {
            using var module = ModuleDefinition.ReadModule(new MemoryStream(pair.Value), Metadata());
            foreach (var guid in PluginProviders(module).Select(p => p.Guid)) payloadGuids[guid] = pair.Key;
        }
        foreach (var file in inputs)
        {
            using var module = ModuleDefinition.ReadModule(new MemoryStream(File.ReadAllBytes(file)), Metadata());
            if (module.Assembly == null) throw new InvalidDataException("Multi-module or unmanaged DLL is unsupported: " + ManifestContract.Relative(output, file));
            provided.Add(module.Assembly.Name.Name);
            foreach (var plugin in PluginProviders(module)) providedGuids.Add(plugin.Guid);
        }
        bool BaseLibrary(string name) => name is "UnboundLib" or "MMHOOK_Assembly-CSharp" or "RoundsWithFriends" or "Octokit"
            or "Sirenix.Serialization" or "Sirenix.Serialization.Config" or "Sirenix.Utilities";
        void Need(string name) { if (!provided.Contains(name) || BaseLibrary(name)) needed.Add(name); }
        foreach (var dependency in request.declared_dependencies)
            foreach (var payload in index.payloads)
                if (payload.declared_aliases.Contains(dependency, StringComparer.Ordinal)) Need(payload.assembly);
        foreach (var file in inputs)
        {
            var bytes = File.ReadAllBytes(file);
            using var module = ModuleDefinition.ReadModule(new MemoryStream(bytes), Metadata());
            if (module.Assembly == null) throw new InvalidDataException("Multi-module or unmanaged DLL is unsupported: " + ManifestContract.Relative(output, file));
            foreach (var reference in module.AssemblyReferences)
                if (payloads.ContainsKey(reference.Name)) Need(reference.Name);
            foreach (var dependency in HardDependencies(module))
                if (!providedGuids.Contains(dependency.Guid) && payloadGuids.TryGetValue(dependency.Guid, out var name)) Need(name);
            if (payloads.ContainsKey(module.Assembly.Name.Name) && BaseLibrary(module.Assembly.Name.Name)) Need(module.Assembly.Name.Name);
        }
        while (true)
        {
            int count = needed.Count;
            foreach (var name in needed.ToArray())
            {
                foreach (var dependency in payloads[name].dependencies) Need(dependency);
                using var module = ModuleDefinition.ReadModule(new MemoryStream(payloadBytes[name]), Metadata());
                foreach (var reference in module.AssemblyReferences)
                    if (payloads.ContainsKey(reference.Name)) Need(reference.Name);
                foreach (var dependency in HardDependencies(module))
                    if (!providedGuids.Contains(dependency.Guid) && payloadGuids.TryGetValue(dependency.Guid, out var depName)) Need(depName);
            }
            if (needed.Count == count) break;
        }
        report.required |= needed.Count > 0;
        foreach (var file in inputs.ToArray())
        {
            var bytes = File.ReadAllBytes(file);
            using var module = ModuleDefinition.ReadModule(new MemoryStream(bytes));
            var name = module.Assembly.Name.Name;
            if (!needed.Contains(name) || !BaseLibrary(name)) continue;
            var hash = ManifestContract.Hash(bytes);
            bool known = hash == payloads[name].sha256 || oldHashes.Contains(name + "\t" + hash);
            if (!known) throw new InvalidDataException("Unknown library bytes cannot be replaced by name/version: " + name + " " + hash);
            File.Delete(file); // only the isolated copied preparation tree
            report.required |= hash != payloads[name].sha256;
            report.warnings.Add("Pinned " + name + " replaces known input " + ManifestContract.Relative(output, file) + " (" + hash + ")");
            inputs.Remove(file);
        }
        foreach (var name in needed.OrderBy(n => n, StringComparer.Ordinal))
        {
            var payload = payloads[name];
            var relative = "DuctTapePlusPlus/Libraries/" + name + ".dll";
            WriteReplacement(output, relative, payloadBytes[name], "Pinned modern dependency closure", report);
        }
        AddRuntime(output, report);
        CheckDuplicates(output, bootstrap);
        using var gameResolver = new ResolverOwner(new Game(request.game_root, managed, request.core, output));
        var game = gameResolver.Game;
        foreach (var name in needed) game.Resolver.Set(Path.Combine(output, "DuctTapePlusPlus", "Libraries", name + ".dll"));
        var scanner = new Scanner(game);
        var curated = new Curated(Path.Combine(working, "curated"), new ManualLogSource("Canna Bliss offline"));
        foreach (var original in inputs)
        {
            var relative = ManifestContract.Relative(output, original);
            var beforeBytes = File.ReadAllBytes(original);
            var beforeHash = ManifestContract.Hash(beforeBytes);
            using var first = ModuleDefinition.ReadModule(original, new ReaderParameters { AssemblyResolver = game.Resolver, ReadingMode = ReadingMode.Immediate, InMemory = true });
            var before = scanner.Scan(first);
            var initialUnchecked = scanner.Unchecked.ToArray();
            var curatedBytes = curated.Apply(original, beforeBytes, beforeHash);
            // Read from a real filename so loose asset bundles are checked on every pass.
            if (!ReferenceEquals(curatedBytes, beforeBytes)) File.WriteAllBytes(original, curatedBytes);
            using var module = ModuleDefinition.ReadModule(original, new ReaderParameters { AssemblyResolver = game.Resolver, ReadingMode = ReadingMode.Immediate, InMemory = true });
            var extra = CannaFixRules.Apply(module, game);
            scanner.Scan(module);
            var fixer = new Fixer(module, game, scanner);
            fixer.Run();
            if (fixer.Changed || extra.Count > 0)
            {
                if (module.Assembly.Name.HasPublicKey)
                    throw new InvalidDataException("Strong-named assembly cannot be rewritten without its signing key: " + relative);
                using var stream = new MemoryStream();
                module.Write(stream);
                File.WriteAllBytes(original, stream.ToArray());
            }
            foreach (var note in fixer.Notes.Where(n => n.StartsWith("MANUAL", StringComparison.Ordinal))) report.errors.Add(relative + ": " + note);
            var afterBytes = File.ReadAllBytes(original);
            var afterHash = ManifestContract.Hash(afterBytes);
            if (beforeHash != afterHash)
            {
                report.required = true;
                var changes = extra.Concat(fixer.Changes).ToList();
                if (!ReferenceEquals(curatedBytes, beforeBytes)) changes.Insert(0, "Exact-hash upstream curated patch");
                report.translated.Add(new Translation { file = relative, before_sha256 = beforeHash, after_sha256 = afterHash, changes = changes.ToArray() });
            }
        }
        using (var finalOwner = new ResolverOwner(new Game(request.game_root, managed, request.core, output)))
        {
            foreach (var name in needed) finalOwner.Game.Resolver.Set(Path.Combine(output, "DuctTapePlusPlus", "Libraries", name + ".dll"));
            ValidateModules(output, finalOwner.Game, report);
            CheckDuplicates(output, finalOwner.Game.Resolver);
            CheckHardDependencies(output, finalOwner.Game.Resolver);
        }
        if (report.errors.Count > 0) return;
        if (ManifestContract.Hash(File.ReadAllBytes(Path.Combine(managed, "Assembly-CSharp.dll"))) != report.game_sha256)
            throw new InvalidDataException("Game assembly changed during compatibility preparation");
        var manifest = new CompatibilityManifest {
            protocol = ManifestContract.Protocol, profile = ManifestContract.Profile, game_sha256 = report.game_sha256,
            assemblies = ManifestContract.CollectAssemblies(output), files = ManifestContract.CollectFiles(output, request.patchers, request.config)
        };
        manifest.digest = ManifestContract.Fingerprint(manifest);
        if (!ManifestContract.Validate(manifest, out var error)) throw new InvalidDataException(error);
        var manifestBytes = System.Text.Encoding.UTF8.GetBytes(Json.Serialize(manifest) + "\n");
        File.WriteAllBytes(Path.Combine(output, ManifestContract.ManifestRelativePath), manifestBytes);
        report.fingerprint = manifest.digest;
        report.manifest_sha256 = ManifestContract.Hash(manifestBytes);
        var current = ManifestContract.ReadFiles(request.plugins);
        if (current.Count != inputHashes.Count || current.Any(f => !inputHashes.TryGetValue(ManifestContract.Relative(request.plugins, f), out var hash)
            || ManifestContract.Hash(File.ReadAllBytes(f)) != hash))
            throw new InvalidDataException("Prepared input changed before commit");
        ManifestContract.RejectLinks(request.plugins);
        var backup = Path.Combine(working, "original-plugins");
        Directory.Move(request.plugins, backup);
        try { Directory.Move(output, request.plugins); }
        catch { Directory.Move(backup, request.plugins); throw; }
        report.ok = true;
        report.warnings = report.warnings.Distinct().OrderBy(s => s, StringComparer.Ordinal).ToList();
    }

    sealed class ResolverOwner : IDisposable
    {
        public Game Game;
        public ResolverOwner(Game game) { Game = game; }
        public void Dispose() { Game.Resolver.Dispose(); }
    }

    static HashSet<string> ReadOldHashes()
    {
        var result = new HashSet<string>(StringComparer.Ordinal);
        foreach (var line in File.ReadAllLines(Path.Combine(Home, "old-libraries.tsv")))
        {
            var fields = line.Split('\t');
            if (fields.Length >= 2) result.Add(Path.GetFileNameWithoutExtension(fields[0]) + "\t" + fields[1]);
        }
        using var resource = typeof(Curated).Assembly.GetManifestResourceStream("curated/patches.tsv")!;
        foreach (var line in new StreamReader(resource).ReadToEnd().Split('\n'))
        {
            var fields = line.Split('\t');
            if (fields.Length >= 4 && fields[0] is "UnboundLib.dll" or "MMHOOK_Assembly-CSharp.dll" or "RoundsWithFriends.dll")
            { result.Add(Path.GetFileNameWithoutExtension(fields[0]) + "\t" + fields[1]); result.Add(Path.GetFileNameWithoutExtension(fields[0]) + "\t" + fields[2]); }
        }
        return result;
    }

    static void WriteReplacement(string output, string relative, byte[] bytes, string reason, Report report)
    {
        var file = Path.Combine(output, relative);
        Directory.CreateDirectory(Path.GetDirectoryName(file)!);
        if (File.Exists(file) && ManifestContract.Hash(File.ReadAllBytes(file)) != ManifestContract.Hash(bytes))
            throw new InvalidDataException("Replacement destination already has different bytes: " + relative);
        File.WriteAllBytes(file, bytes);
        using var module = ModuleDefinition.ReadModule(new MemoryStream(bytes));
        report.replacements.Add(new Replacement { file = relative, identity = module.Assembly.Name.FullName,
            sha256 = ManifestContract.Hash(bytes), reason = reason });
    }

    static void AddRuntime(string output, Report report)
    {
        foreach (var filename in new[] { "rounds-port.Runtime.dll", "Canna.DuctTapePlusPlus.NetworkGuard.dll" })
        {
            var bytes = File.ReadAllBytes(Path.GetFullPath(Path.Combine(Home, "..", "runtime", filename)));
            WriteReplacement(output, "DuctTapePlusPlus/" + filename, bytes, "Pinned compatibility runtime", report);
        }
    }

    static void CheckDuplicates(string output, IAssemblyResolver resolver)
    {
        var assemblies = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var guids = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var file in ManifestContract.ReadFiles(output).Where(f => f.EndsWith(".dll", StringComparison.OrdinalIgnoreCase)))
        {
            using var module = ModuleDefinition.ReadModule(file, new ReaderParameters { AssemblyResolver = resolver, ReadingMode = ReadingMode.Deferred, InMemory = true });
            if (!assemblies.Add(module.Assembly.Name.Name)) throw new InvalidDataException("Duplicate assembly simple identity: " + module.Assembly.Name.Name);
            foreach (var attribute in Scanner.AllTypes(module).SelectMany(t => t.CustomAttributes)
                .Where(a => a.AttributeType.FullName == "BepInEx.BepInPlugin"))
            {
                var guid = attribute.ConstructorArguments.FirstOrDefault().Value as string;
                if (string.IsNullOrEmpty(guid) || !guids.Add(guid)) throw new InvalidDataException("Missing or duplicate plugin GUID: " + guid);
            }
        }
    }

    sealed record PluginProvider(string Guid, Version Version, TypeDefinition Type);
    sealed record Dependency(string Guid, Version? Minimum);

    static IEnumerable<PluginProvider> PluginProviders(ModuleDefinition module)
    {
        foreach (var type in Scanner.AllTypes(module))
        foreach (var attribute in type.CustomAttributes.Where(a => a.AttributeType.FullName == "BepInEx.BepInPlugin"))
        {
            if (attribute.AttributeType.Scope is not AssemblyNameReference scope || scope.Name != "BepInEx")
                throw new InvalidDataException("Plugin attribute does not reference the actual BepInEx framework");
            if (type.IsAbstract || type.HasGenericParameters || !type.IsClass || !DerivesPlugin(type))
                throw new InvalidDataException("Attributed plugin provider must be a concrete actual BaseUnityPlugin: " + type.FullName);
            if (attribute.ConstructorArguments.Count != 3 || attribute.ConstructorArguments[0].Value is not string guid
                || string.IsNullOrWhiteSpace(guid) || guid.Length > 2048 || guid.Any(c => !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z'
                    || c >= '0' && c <= '9' || c == '.' || c == '_' || c == '-')) || attribute.ConstructorArguments[1].Value is not string
                || attribute.ConstructorArguments[2].Value is not string version
                || !Version.TryParse(version, out var parsed)) throw new InvalidDataException("Malformed BepInPlugin identity/version");
            CheckProcessFilters(type);
            yield return new PluginProvider(guid, parsed, type);
        }
    }

    static IEnumerable<CustomAttribute> InheritedAttributes(TypeDefinition type, string fullName)
    {
        var seen = new HashSet<string>(StringComparer.Ordinal);
        TypeDefinition? current = type;
        for (int count = 0; current != null; count++)
        {
            if (count >= 128 || !seen.Add(current.Module.Assembly.Name.FullName + "\t" + current.FullName))
                throw new InvalidDataException("Cyclic or excessive plugin metadata inheritance");
            foreach (var attribute in current.CustomAttributes.Where(a => a.AttributeType.FullName == fullName))
            {
                // BepInEx5.4.11's Cecil metadata reader matches FULLNAME without
                // assembly scope. Foreign lookalikes can therefore alter loading;
                // neither silently trust nor ignore these ambiguous declarations.
                if (attribute.AttributeType.Scope is not AssemblyNameReference scope || scope.Name != "BepInEx")
                    throw new InvalidDataException("Unsupported foreign-scope loader metadata: " + fullName);
                yield return attribute;
            }
            current = current.BaseType?.Resolve();
        }
    }

    static void CheckProcessFilters(TypeDefinition type)
    {
        var filters = InheritedAttributes(type, "BepInEx.BepInProcess").ToArray();
        if (filters.Length == 0) return;
        bool matches = false;
        foreach (var filter in filters)
        {
            if (filter.ConstructorArguments.Count != 1 || filter.ConstructorArguments[0].Value is not string process
                || string.IsNullOrWhiteSpace(process) || process.Length > 2048)
                throw new InvalidDataException("Malformed BepInProcess filter: " + type.FullName);
            // Verified installed Chainloader: case-sensitive .exe removal,
            // invariant case-insensitive comparison; ANY match permits loading.
            // Limit this profile to ordinary reviewed ROUNDS process spellings.
            bool ordinary = string.Equals(process, "ROUNDS", StringComparison.InvariantCultureIgnoreCase)
                || process.EndsWith(".exe", StringComparison.Ordinal)
                    && string.Equals(process.Substring(0, process.Length - 4), "ROUNDS", StringComparison.InvariantCultureIgnoreCase);
            matches |= ordinary && string.Equals(process.Replace(".exe", ""), "ROUNDS", StringComparison.InvariantCultureIgnoreCase);
        }
        if (!matches) throw new InvalidDataException("Plugin process filters do not permit the reviewed ROUNDS.exe process: " + type.FullName);
    }

    static IEnumerable<string> Incompatibilities(TypeDefinition type)
    {
        foreach (var attribute in InheritedAttributes(type, "BepInEx.BepInIncompatibility"))
        {
            if (attribute.ConstructorArguments.Count != 1 || attribute.ConstructorArguments[0].Value is not string guid
                || string.IsNullOrWhiteSpace(guid)) throw new InvalidDataException("Malformed BepInIncompatibility GUID");
            yield return guid;
        }
    }

    static bool DerivesPlugin(TypeDefinition type)
    {
        var seen = new HashSet<string>(StringComparer.Ordinal);
        TypeReference? current = type.BaseType;
        for (int count = 0; current != null && count < 128; count++)
        {
            if (current.FullName == "BepInEx.BaseUnityPlugin" && current.Scope is AssemblyNameReference scope && scope.Name == "BepInEx")
            {
                var actual = current.Resolve();
                return actual != null && actual.Module.Assembly.Name.Name == "BepInEx" && actual.FullName == "BepInEx.BaseUnityPlugin";
            }
            var key = current.Scope + "\t" + current.FullName;
            if (!seen.Add(key)) throw new InvalidDataException("Cyclic plugin provider inheritance");
            current = current.Resolve()?.BaseType;
        }
        return false;
    }

    static IEnumerable<Dependency> HardDependencies(ModuleDefinition module)
    {
        foreach (var plugin in PluginProviders(module))
        foreach (var attribute in InheritedAttributes(plugin.Type, "BepInEx.BepInDependency"))
        {
            if (attribute.AttributeType.Scope is not AssemblyNameReference scope || scope.Name != "BepInEx")
                throw new InvalidDataException("Dependency attribute does not reference the actual BepInEx framework");
            if (attribute.ConstructorArguments.Count < 1 || attribute.ConstructorArguments[0].Value is not string guid
                || string.IsNullOrWhiteSpace(guid)) throw new InvalidDataException("Malformed BepInDependency GUID");
            Version? minimum = null;
            int flags = 1;
            if (attribute.ConstructorArguments.Count > 1)
            {
                var value = attribute.ConstructorArguments[1].Value;
                if (value is string version)
                {
                    if (!Version.TryParse(version, out minimum)) throw new InvalidDataException("Invalid minimum plugin version: " + guid);
                }
                else flags = Convert.ToInt32(value);
            }
            foreach (var property in attribute.Properties.Concat(attribute.Fields))
                if (property.Name is "Flags" or "DependencyFlags") flags = Convert.ToInt32(property.Argument.Value);
            if (flags == 2) continue;
            if (flags != 1) throw new InvalidDataException("Unknown BepInDependency flags: " + guid);
            yield return new Dependency(guid, minimum);
        }
    }

    static void CheckHardDependencies(string output, IAssemblyResolver resolver)
    {
        var files = ManifestContract.ReadFiles(output).Where(f => f.EndsWith(".dll", StringComparison.OrdinalIgnoreCase)).ToList();
        var providers = new Dictionary<string, Version>(StringComparer.Ordinal);
        var incompatible = new List<(string Guid, string Other)>();
        foreach (var file in files)
        {
            using var module = ModuleDefinition.ReadModule(file, new ReaderParameters { AssemblyResolver = resolver, InMemory = true });
            foreach (var plugin in PluginProviders(module))
            {
                providers.Add(plugin.Guid, plugin.Version);
                foreach (var other in Incompatibilities(plugin.Type)) incompatible.Add((plugin.Guid, other));
            }
        }
        foreach (var pair in incompatible)
            if (providers.ContainsKey(pair.Other))
                throw new InvalidDataException("Incompatible actual plugin providers cannot be activated together: " + pair.Guid + " and " + pair.Other);
        foreach (var file in files)
        {
            using var module = ModuleDefinition.ReadModule(file, new ReaderParameters { AssemblyResolver = resolver, InMemory = true });
            foreach (var dependency in HardDependencies(module))
                if (!providers.TryGetValue(dependency.Guid, out var version)
                    || dependency.Minimum != null && version < dependency.Minimum)
                    throw new InvalidDataException("Unresolved hard plugin dependency: " + ManifestContract.Relative(output, file)
                        + " requires " + dependency.Guid + (dependency.Minimum == null ? "" : " >= " + dependency.Minimum));
        }
    }
}
