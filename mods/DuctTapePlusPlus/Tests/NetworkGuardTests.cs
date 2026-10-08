using System;
using System.IO;
using System.Linq;
using Canna.DuctTapePlusPlus;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;

static class NetworkGuardTests
{
    static readonly string Hash = new string('a', 64), Other = new string('b', 64);
    static readonly string Epoch = new string('c', 32), NewEpoch = new string('d', 32);
    static int passed;
    static PeerAdvertisement Ad(int actor, string epoch = null, string protocol = null, string profile = null, string game = null, string digest = null)
    { return new PeerAdvertisement(protocol ?? RoomCompatibilityPolicy.Protocol, profile ?? RoomCompatibilityPolicy.Profile, game ?? Hash, digest ?? Hash, actor, epoch ?? Epoch); }
    static RoomCompatibilityPolicy Ready()
    {
        var policy = new RoomCompatibilityPolicy();
        policy.ConfigureLocal(Ad(1)); policy.BeginRoom(1, Epoch); policy.ReplaceRoster(new[] { 1, 2 });
        policy.ObservePeer(1, Ad(1), policy.Generation); policy.ObservePeer(2, Ad(2), policy.Generation);
        return policy;
    }
    static void Check(bool value, string name)
    { if (!value) throw new Exception(name); Console.WriteLine("PASS " + name); passed++; }
    static void Main()
    {
        Check(!new RoomCompatibilityPolicy().Evaluate().Allowed, "Uninitialized state denies");
        var p = Ready(); Check(p.Evaluate().Allowed, "Every current actor matches");
        p.ObservePeer(2, null, p.Generation); Check(!p.Evaluate().Allowed, "Missing peer advertisement denies");
        p = Ready(); p.ObservePeer(2, Ad(2, protocol: "different/1"), p.Generation); Check(!p.Evaluate().Allowed, "Different protocol denies");
        p = Ready(); p.ObservePeer(2, Ad(2, profile: "different"), p.Generation); Check(!p.Evaluate().Allowed, "Different profile denies");
        p = Ready(); p.ObservePeer(2, Ad(2, game: Other), p.Generation); Check(!p.Evaluate().Allowed, "Different public game hash denies");
        p = Ready(); p.ObservePeer(2, Ad(2, digest: Other), p.Generation); Check(!p.Evaluate().Allowed, "Different content digest denies");
        p = Ready(); p.ObservePeer(2, Ad(2, digest: "malformed"), p.Generation); Check(!p.Evaluate().Allowed, "Malformed digest denies");
        p = Ready(); p.ObservePeer(2, Ad(2, epoch: NewEpoch), p.Generation); Check(!p.Evaluate().Allowed, "Stale room session denies");
        p = Ready(); p.ObservePeer(2, Ad(1), p.Generation); Check(!p.Evaluate().Allowed, "Remote claim of local actor cannot replace local evidence");
        p = Ready(); Check(!p.ObservePeer(3, Ad(2), p.Generation) && p.Evaluate().Allowed, "Unknown sender cannot inject a roster peer");
        p = Ready(); p.ObservePeer(1, Ad(2), p.Generation); Check(!p.Evaluate().Allowed, "Local actor binding must also match");
        p = Ready(); int old = p.Generation; p.BeginRoom(1, NewEpoch); p.ReplaceRoster(new[] { 1, 2 });
        Check(!p.ObservePeer(2, Ad(2), old) && !p.Evaluate().Allowed, "Previous connection callbacks cannot satisfy a reconnect");
        p.ObservePeer(1, Ad(1, epoch: NewEpoch), p.Generation); p.ObservePeer(2, Ad(2, epoch: NewEpoch), p.Generation);
        Check(p.Evaluate().Allowed, "Fresh reconnect advertisements restore readiness");
        p = Ready(); p.ReplaceRoster(new[] { 1, 2, 3 }); Check(!p.Evaluate().Allowed, "Late join immediately denies until advertised");
        p.ObservePeer(3, Ad(3), p.Generation); Check(p.Evaluate().Allowed, "Matching late join restores readiness");
        p.ObservePeer(3, Ad(3, digest: Other), p.Generation); Check(!p.Evaluate().Allowed, "Peer property change revokes readiness");
        p.ReplaceRoster(new[] { 1, 2 }); Check(p.Evaluate().Allowed, "Departed peer no longer participates");
        p.Clear(); Check(!p.Evaluate().Allowed, "Leave and disconnect clear all readiness");
        p = Ready(); p.ConfigureLocal(Ad(1, digest: Other)); Check(!p.Evaluate().Allowed, "Local gameplay configuration change revokes readiness");
        p = Ready(); p.ConfigureLocal(Ad(1), "Immutable mod payload changed."); Check(!p.Evaluate().Allowed, "Invalid local payload denies");
        p = Ready(); p.ReplaceRoster(new[] { 2 }); Check(!p.Evaluate().Allowed, "Membership missing local actor denies");
        p = Ready(); p.ReplaceRoster(new[] { 1, 2, 2 }); Check(!p.Evaluate().Allowed, "Duplicate actor roster denies");
        p = Ready(); p.BeginRoom(1, "bad"); p.ReplaceRoster(new[] { 1, 2 }); Check(!p.Evaluate().Allowed, "Malformed room epoch denies");
        ManifestChecks();
        ConfigChecks();
        JsonChecks();
        var stamps = new ContentStampPolicy();
        Check(stamps.NeedsVerification("first", false), "Initial content stamp requires a full hash");
        stamps.RecordAttempt("first"); Check(!stamps.NeedsVerification("first", false), "Unchanged content stamp skips polling hash");
        Check(stamps.NeedsVerification("different", false), "Changed content stamp requires verification");
        Check(stamps.NeedsVerification("first", true), "Actual gate forces hashing despite identical metadata");
        stamps.Invalidate(); Check(stamps.NeedsVerification("first", false), "Unreadable or linked stamp invalidates cached verification");
        Console.WriteLine("Offline policy checks: " + passed + "; no game or multiplayer session was run.");
    }

    static void ConfigChecks()
    {
        Func<string, string> hash = s => ManifestContract.ConfigHash(System.Text.Encoding.UTF8.GetBytes(s));
        string first = "# description\r\n[Game]\r\nLives = 3\r\nCards = true\r\n";
        Check(hash(first) == hash("[Game]\nCards=true\n# other comment\nLives=3\n"), "Equivalent config order, comments, line endings and serialization spacing match");
        Check(hash(first) != hash("[Game]\nLives=4\nCards=true\n"), "Different gameplay value remains significant");
        Check(hash(first) != hash("[Other]\nLives=3\nCards=true\n"), "Section identities remain significant");
        string duplicate="[Game]\nLives=3\nLives=4\n";
        Check(hash(duplicate) == ManifestContract.Hash(System.Text.Encoding.UTF8.GetBytes(duplicate)), "Ambiguous duplicate config syntax retains exact-byte comparison");
        var expected = Manifest(Hash, Hash, Hash);
        var changed = Manifest(Hash, Other, Hash);
        Check(ManifestContract.ComponentFingerprint(expected, "mods") == ManifestContract.ComponentFingerprint(changed, "mods")
            && ManifestContract.ComponentFingerprint(expected, "config") != ManifestContract.ComponentFingerprint(changed, "config"), "Component diagnostics isolate configuration without weakening aggregate equality");
        var p = Ready();
        var local=Ad(1);local.ModsDigest=Hash;local.AssetsDigest=Hash;local.ConfigDigest=Hash;p.ConfigureLocal(local);
        var peer=Ad(2,digest:Other);peer.ModsDigest=Hash;peer.AssetsDigest=Hash;peer.ConfigDigest=Other;p.ObservePeer(2,peer,p.Generation);
        Check(!p.Evaluate().Allowed && p.Evaluate().Reason.Contains("active gameplay settings"), "Config mismatch gives actionable denial");
        peer.ModsDigest=Other;p.ObservePeer(2,peer,p.Generation);
        Check(!p.Evaluate().Allowed && p.Evaluate().Reason.Contains("Rebound release"), "Mod or support mismatch identifies prepared DLLs");
        peer.ModsDigest=Hash;peer.AssetsDigest=Other;p.ObservePeer(2,peer,p.Generation);
        Check(!p.Evaluate().Allowed && p.Evaluate().Reason.Contains("assets or patchers"), "Asset mismatch identifies immutable content");
        peer.AssetsDigest=Hash;peer.ConfigDigest=Hash;p.ObservePeer(2,peer,p.Generation);
        Check(!p.Evaluate().Allowed, "Equal component claims cannot override unequal full fingerprint");
    }

    static CompatibilityManifest Manifest(string asset = null, string config = null, string patcher = null)
    {
        var rows = new System.Collections.Generic.List<FileRow>();
        if (asset != null) rows.Add(new FileRow { root = "plugins", path = "Example, Version=1.0.0.0, Culture=neutral, PublicKeyToken=null/art.bundle", sha256 = asset });
        if (config != null) rows.Add(new FileRow { root = "config", path = "example.cfg", sha256 = config });
        if (patcher != null) rows.Add(new FileRow { root = "patchers", path = "approved.dll", sha256 = patcher });
        var result = new CompatibilityManifest { protocol = RoomCompatibilityPolicy.Protocol, profile = RoomCompatibilityPolicy.Profile,
            game_sha256 = Hash, assemblies = new[] { new AssemblyRow { identity = "Example, Version=1.0.0.0, Culture=neutral, PublicKeyToken=null", sha256 = Hash } }, files = rows.ToArray() };
        result.digest = ManifestContract.Fingerprint(result); return result;
    }

    static void ManifestChecks()
    {
        string error;
        var expected = Manifest(Hash, Hash, Hash);
        Check(ManifestContract.Validate(expected, out error), "Canonical manifest validates");
        var changed = Manifest(Hash, Other, Hash);
        Check(ManifestContract.VerifyImmutable(expected, changed, out error) && expected.digest != changed.digest, "Live gameplay config changes local digest without changing immutable profile");
        changed = Manifest(Hash, null, Hash);
        Check(ManifestContract.VerifyImmutable(expected, changed, out error), "Runtime config removal also participates in recomputed parity");
        changed = Manifest(Other, Hash, Hash);
        Check(!ManifestContract.VerifyImmutable(expected, changed, out error), "Different asset payload denies despite identical DLL");
        changed = Manifest(Hash, Hash, Other);
        Check(!ManifestContract.VerifyImmutable(expected, changed, out error), "Different patcher payload denies");
        changed = Manifest(Hash, Hash, Hash); changed.assemblies[0].sha256 = Other; changed.digest = ManifestContract.Fingerprint(changed);
        Check(!ManifestContract.VerifyImmutable(expected, changed, out error), "Different managed DLL denies");
        changed = Manifest(Hash, Hash, Hash); changed.game_sha256 = Other; changed.digest = ManifestContract.Fingerprint(changed);
        Check(!ManifestContract.VerifyImmutable(expected, changed, out error), "Game update invalidates prepared payload");
        changed = Manifest(); changed.digest = Other;
        Check(!ManifestContract.Validate(changed, out error), "Forged canonical digest denies");
        changed = Manifest(); changed.assemblies = new[] { expected.assemblies[0], expected.assemblies[0] }; changed.digest = ManifestContract.Fingerprint(changed);
        Check(!ManifestContract.Validate(changed, out error), "Duplicate assembly identities deny");
        changed = Manifest(); changed.assemblies = new[] { expected.assemblies[0], new AssemblyRow { identity = "Example, Version=9.0.0.0, Culture=neutral, PublicKeyToken=null", sha256 = Other } }; changed.digest = ManifestContract.Fingerprint(changed);
        Check(!ManifestContract.Validate(changed, out error), "Different versions with same loader assembly name deny");
        changed = Manifest(Hash); changed.files[0].path = "../bad"; changed.digest = ManifestContract.Fingerprint(changed);
        Check(!ManifestContract.Validate(changed, out error), "Content traversal key denies");
        var first = Manifest(Hash, Other, Hash); Array.Reverse(first.files);
        Check(ManifestContract.Fingerprint(first) == Manifest(Hash, Other, Hash).digest, "Canonical order is independent of JSON row order");
        Check(ManifestContract.CanonicalText(expected).EndsWith("\n", StringComparison.Ordinal) && ManifestContract.CanonicalText(expected).Contains("assembly\tExample"), "Domain separated canonical UTF8 text retains final LF");
    }

    static void RejectJson(string text, string name)
    {
        bool rejected = false;
        try { ManifestJson.Parse(text); }
        catch (ManifestDecodeException) { rejected = true; }
        Check(rejected, name);
    }
    static void JsonChecks()
    {
        string error;
        // Literal output from the real helper which Unity parsed with both arrays null.
        string actual = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Fixtures", "helper-runtime-manifest.json"));
        var decoded = ManifestJson.Parse(actual);
        Check(decoded.assemblies.Length == 7 && decoded.files.Length == 0 && ManifestContract.Validate(decoded, out error),
            "Actual seven-assembly helper JSON decodes with explicit empty files array");
        string json = JsonConvert.SerializeObject(Manifest(Hash, Other, Hash));
        decoded = ManifestJson.Parse(json);
        Check(decoded.files.Length == 3 && ManifestContract.Validate(decoded, out error), "Nonempty asset, patcher and configuration arrays decode exactly");
        decoded = ManifestJson.Parse(actual.Replace("+", "\\u002B"));
        Check(ManifestContract.Validate(decoded, out error), "Helper Unicode escapes retain canonical decoded digest");
        var reordered = JObject.Parse(json);
        reordered["files"] = new JArray(((JArray)reordered["files"]).Reverse());
        Check(ManifestContract.Validate(ManifestJson.Parse(reordered.ToString(Formatting.None)), out error), "JSON row order does not affect decoded canonical content");
        RejectJson("{\"protocol\":\"duplicate\"," + json.Substring(1), "Duplicate root properties are rejected before schema validation");
        RejectJson(json.Replace("\"identity\":", "\"identity\":\"duplicate\",\"identity\":"), "Duplicate row properties cannot overwrite hashed assembly identity");
        RejectJson(json.Replace("\"protocol\":", "\"p\\u0072otocol\":\"duplicate\",\"protocol\":"), "Escaped duplicate property aliases are rejected");
        var altered = JObject.Parse(json); altered["unexpected"] = "value";
        RejectJson(altered.ToString(Formatting.None), "Unknown root properties are rejected");
        altered = JObject.Parse(json); ((JObject)altered["assemblies"][0])["unexpected"] = "value";
        RejectJson(altered.ToString(Formatting.None), "Unknown managed assembly properties are rejected");
        altered = JObject.Parse(json); altered.Remove("files");
        RejectJson(altered.ToString(Formatting.None), "Missing file array is not silently converted to empty");
        altered = JObject.Parse(json); altered["files"] = JValue.CreateNull();
        RejectJson(altered.ToString(Formatting.None), "Null file array is rejected");
        altered = JObject.Parse(json); altered["assemblies"] = new JObject();
        RejectJson(altered.ToString(Formatting.None), "Wrong array type is rejected");
        altered = JObject.Parse(json); altered["digest"] = 1;
        RejectJson(altered.ToString(Formatting.None), "Numeric manifest fields cannot coerce to strings");
        altered = JObject.Parse(json); altered["assemblies"][0] = "not-an-object";
        RejectJson(altered.ToString(Formatting.None), "Rows must be schema objects");
        RejectJson(json + "{}", "Trailing JSON content is rejected");
        RejectJson(json.Substring(0, json.Length - 1), "Truncated JSON is rejected");
        RejectJson("{\"assemblies\":[[[[[[]]]]]]}", "Deeply nested JSON is bounded before object construction");
        altered = JObject.Parse(json); altered["assemblies"] = new JArray(Enumerable.Range(0, 4097).Select(_ => new JObject()));
        RejectJson(altered.ToString(Formatting.None), "Managed assembly array count is bounded");
        altered = JObject.Parse(json); altered["files"] = new JArray(Enumerable.Range(0, 16385).Select(_ => new JObject()));
        RejectJson(altered.ToString(Formatting.None), "Prepared file array count is bounded");
        altered = JObject.Parse(json); altered["profile"] = new string('x', 2049);
        RejectJson(altered.ToString(Formatting.None), "Decoded JSON string length is bounded");
        RejectJson(new string(' ', ManifestJson.MaximumTextLength + 1), "Total JSON text size is bounded");
        RejectJson("{\"files\":[" + string.Join(",", Enumerable.Repeat("{}", 100001)) + "]}", "JSON token budget bounds oversized object allocation");
        RejectJson("/* comment */" + json, "JavaScript comments are rejected");
        RejectJson(json.Replace("\"protocol\"", "'protocol'"), "Single-quoted JavaScript properties are rejected");
        RejectJson(json.Replace("\"protocol\"", "protocol"), "Unquoted JavaScript properties are rejected");
        RejectJson(json.Substring(0, json.Length - 1) + ",}", "Trailing object commas are rejected");
        RejectJson(actual.Replace("\"files\":[]", "\"files\":[{} ,]"), "Trailing array commas are rejected");
        RejectJson("\u00a0" + json, "Non-JSON whitespace is rejected");
        RejectJson(json.Replace("Example,", "Example\\u0000,"), "Escaped control characters are rejected");
        RejectJson(json.Replace("Example,", "Example\\ud800,"), "Unpaired Unicode surrogate escapes are rejected");
        RejectJson(json.Replace("Example,", "Example\\udc00,"), "Orphan low Unicode surrogate escapes are rejected");
        RejectJson(json.Replace("Example,", "Example\\uZZZZ,"), "Malformed Unicode escape digits are rejected");
        var unicode = Manifest(config: Hash);
        unicode.files[0].path = "emoji-\ud83d\ude00.cfg"; unicode.digest = ManifestContract.Fingerprint(unicode);
        string unicodeJson = JsonConvert.SerializeObject(unicode, new JsonSerializerSettings { StringEscapeHandling = StringEscapeHandling.EscapeNonAscii });
        Check(ManifestContract.Validate(ManifestJson.Parse(unicodeJson), out error), "Paired Unicode surrogate escapes preserve valid asset keys and digest");
        Check(ManifestJson.Parse(json.Replace("rounds-public-1.1.2", @"literal-\\ud800")).profile == @"literal-\ud800",
            "Escaped literal backslash text is not mistaken for a Unicode escape");
    }
}
