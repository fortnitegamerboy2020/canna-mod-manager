// Canna DuctTape++ local preview. MIT; upstream components retain their own notices.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;

namespace Canna.DuctTapePlusPlus
{
    [Serializable]
    public sealed class AssemblyRow
    {
        public string identity;
        public string sha256;
    }

    [Serializable]
    public sealed class FileRow
    {
        public string root;
        public string path;
        public string sha256;
    }

    [Serializable]
    public sealed class CompatibilityManifest
    {
        public string protocol;
        public string profile;
        public string game_sha256;
        public string digest;
        public AssemblyRow[] assemblies;
        public FileRow[] files;
    }

    public static class ManifestContract
    {
        public const string Protocol = "canna.ducttape++/1";
        public const string Profile = "rounds-public-1.1.2";
        public const string ManifestRelativePath = "DuctTapePlusPlus/compatibility-manifest.json";

        public static string Hash(byte[] bytes)
        {
            using (var hash = SHA256.Create())
                return BitConverter.ToString(hash.ComputeHash(bytes)).Replace("-", "").ToLowerInvariant();
        }

        public static bool IsHash(string value)
        {
            return value != null && value.Length == 64 && value.All(c =>
                c >= '0' && c <= '9' || c >= 'a' && c <= 'f');
        }

        static bool SafeAtom(string value)
        {
            return !string.IsNullOrEmpty(value) && value.Length <= 2048
                && value.IndexOfAny(new[] { '\r', '\n', '\t', '\0' }) < 0;
        }

        public static bool Validate(CompatibilityManifest manifest, out string error)
        {
            error = null;
            if (manifest == null || manifest.protocol != Protocol || manifest.profile != Profile
                || !IsHash(manifest.game_sha256) || !IsHash(manifest.digest)
                || manifest.assemblies == null || manifest.files == null
                || manifest.assemblies.Length > 4096 || manifest.files.Length > 16384)
            { error = "Invalid compatibility manifest header or limits"; return false; }
            var identities = new HashSet<string>(StringComparer.Ordinal);
            var simpleNames = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (var row in manifest.assemblies)
            {
                if (row == null || !SafeAtom(row.identity) || !IsHash(row.sha256)
                    || !identities.Add(row.identity) || !simpleNames.Add(row.identity.Split(',')[0]))
                { error = "Invalid or duplicate managed assembly identity"; return false; }
            }
            var paths = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (var row in manifest.files)
            {
                if (row == null || !SafeAtom(row.path) || !IsHash(row.sha256)
                    || !(row.root == "plugins" || row.root == "patchers" || row.root == "config")
                    || row.path.StartsWith("/", StringComparison.Ordinal) || row.path.Contains("\\")
                    || row.path.Split('/').Any(p => p == ".." || p == "." || p.Length == 0)
                    || !paths.Add(row.root + "/" + row.path))
                { error = "Invalid or duplicate prepared content key"; return false; }
            }
            if (Fingerprint(manifest) != manifest.digest)
            { error = "Compatibility manifest digest does not match its content"; return false; }
            return true;
        }

        public static string CanonicalText(CompatibilityManifest manifest)
        {
            var text = new StringBuilder();
            text.Append(manifest.protocol).Append('\n').Append(manifest.profile).Append('\n')
                .Append(manifest.game_sha256).Append('\n');
            foreach (var row in manifest.assemblies.OrderBy(r => r.identity, StringComparer.Ordinal))
                text.Append("assembly\t").Append(row.identity).Append('\t').Append(row.sha256).Append('\n');
            foreach (var row in manifest.files.OrderBy(r => r.root + "/" + r.path, StringComparer.Ordinal))
                text.Append("file\t").Append(row.root).Append('/').Append(row.path).Append('\t')
                    .Append(row.sha256).Append('\n');
            return text.ToString();
        }

        public static string Fingerprint(CompatibilityManifest manifest)
        {
            return Hash(Encoding.UTF8.GetBytes(CanonicalText(manifest)));
        }

        public static string Relative(string root, string file)
        {
            root = Path.GetFullPath(root).TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
            file = Path.GetFullPath(file);
            var prefix = root + Path.DirectorySeparatorChar;
            if (!file.StartsWith(prefix, StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException("Path escapes prepared content root");
            return file.Substring(prefix.Length).Replace('\\', '/');
        }

        public static void RejectLinks(string path)
        {
            var current = Path.GetFullPath(path);
            for (;;)
            {
                if ((File.Exists(current) || Directory.Exists(current))
                    && (File.GetAttributes(current) & FileAttributes.ReparsePoint) != 0)
                    throw new InvalidDataException("Linked directories or files are not allowed in compatibility preparation");
                var parent = Path.GetDirectoryName(current);
                if (string.IsNullOrEmpty(parent) || parent == current) return;
                current = parent;
            }
        }

        public static List<string> ReadFiles(string root)
        {
            var files = new List<string>();
            if (string.IsNullOrEmpty(root) || !Directory.Exists(root)) return files;
            RejectLinks(root);
            var pending = new Stack<string>();
            pending.Push(root);
            while (pending.Count > 0)
            {
                var directory = pending.Pop();
                foreach (var file in Directory.GetFiles(directory))
                {
                    RejectLinks(file);
                    files.Add(file);
                    if (files.Count > 16384) throw new InvalidDataException("Prepared file count exceeds 16384");
                }
                foreach (var child in Directory.GetDirectories(directory))
                { RejectLinks(child); pending.Push(child); }
            }
            files.Sort(StringComparer.Ordinal);
            return files;
        }

        public static AssemblyRow[] CollectAssemblies(string plugins)
        {
            var rows = ReadFiles(plugins).Where(f => f.EndsWith(".dll", StringComparison.OrdinalIgnoreCase))
                .Select(f => new AssemblyRow {
                    identity = AssemblyName.GetAssemblyName(f).FullName,
                    sha256 = Hash(File.ReadAllBytes(f))
                }).OrderBy(r => r.identity, StringComparer.Ordinal).ToArray();
            return rows;
        }

        // A package's manager folder is arbitrary. Anchor its assets to the nearest
        // directory with a DLL, and then keep the asset's path relative to that DLL.
        // Both preparation and runtime use this exact mapping.
        public static FileRow[] CollectFiles(string plugins, string patchers, string config)
        {
            var rows = new List<FileRow>();
            var pluginFiles = ReadFiles(plugins);
            var owners = pluginFiles.Where(f => f.EndsWith(".dll", StringComparison.OrdinalIgnoreCase))
                .GroupBy(Path.GetDirectoryName, StringComparer.OrdinalIgnoreCase)
                .ToDictionary(g => g.Key, g => g.Select(f => AssemblyName.GetAssemblyName(f).FullName)
                    .OrderBy(n => n, StringComparer.Ordinal).ToArray(), StringComparer.OrdinalIgnoreCase);
            foreach (var file in pluginFiles)
            {
                var relative = Relative(plugins, file);
                if (file.EndsWith(".dll", StringComparison.OrdinalIgnoreCase)
                    || relative == ManifestRelativePath || relative == ".canna-ducttape-staging") continue;
                var directory = Path.GetDirectoryName(file);
                string owner = null;
                string ownerDirectory = null;
                while (directory != null && (directory.Equals(plugins, StringComparison.OrdinalIgnoreCase)
                    || directory.StartsWith(plugins.TrimEnd('\\', '/') + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase)))
                {
                    string[] names;
                    if (owners.TryGetValue(directory, out names))
                    {
                        owner = names.Length == 1 ? names[0]
                            : "assemblies-" + Hash(Encoding.UTF8.GetBytes(string.Join("\n", names)));
                        ownerDirectory = directory;
                        break;
                    }
                    directory = Path.GetDirectoryName(directory);
                }
                rows.Add(new FileRow { root = "plugins", path = owner == null ? "unowned/" + relative
                    : owner + "/" + Relative(ownerDirectory, file), sha256 = Hash(File.ReadAllBytes(file)) });
            }
            foreach (var entry in new[] { new[] { "patchers", patchers }, new[] { "config", config } })
                foreach (var file in ReadFiles(entry[1]))
                {
                    if (entry[0] == "config" && Relative(entry[1], file).Equals("BepInEx.cfg", StringComparison.OrdinalIgnoreCase)) continue;
                    rows.Add(new FileRow { root = entry[0], path = Relative(entry[1], file), sha256 = Hash(File.ReadAllBytes(file)) });
                }
            return rows.OrderBy(r => r.root + "/" + r.path, StringComparer.Ordinal).ToArray();
        }

        public static bool VerifyImmutable(CompatibilityManifest expected, CompatibilityManifest actual, out string error)
        {
            if (!Validate(expected, out error) || !Validate(actual, out error)) return false;
            if (expected.game_sha256 != actual.game_sha256)
            { error = "The game assembly changed since translation"; return false; }
            Func<CompatibilityManifest, string[]> assemblies = m => m.assemblies
                .OrderBy(r => r.identity, StringComparer.Ordinal).Select(r => r.identity + "\t" + r.sha256).ToArray();
            Func<CompatibilityManifest, string[]> immutableFiles = m => m.files.Where(r => r.root != "config")
                .OrderBy(r => r.root + "/" + r.path, StringComparer.Ordinal)
                .Select(r => r.root + "/" + r.path + "\t" + r.sha256).ToArray();
            if (!assemblies(expected).SequenceEqual(assemblies(actual))
                || !immutableFiles(expected).SequenceEqual(immutableFiles(actual)))
            { error = "Installed managed assemblies, assets or patchers differ from the prepared profile"; return false; }
            error = null;
            return true;
        }

        public static CompatibilityManifest CreateRuntimeManifest(CompatibilityManifest expected, string plugins, string patchers, string config)
        {
            var actual = new CompatibilityManifest {
                protocol = Protocol, profile = Profile, game_sha256 = expected.game_sha256,
                assemblies = CollectAssemblies(plugins), files = CollectFiles(plugins, patchers, config)
            };
            actual.digest = Fingerprint(actual);
            return actual;
        }
    }
}
