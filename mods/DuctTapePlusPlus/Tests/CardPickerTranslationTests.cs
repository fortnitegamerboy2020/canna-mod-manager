// Executes only reviewed Prefix/condition IL in fixture APIs; no game process or files.
using System.Reflection;
using System.Security.Cryptography;
using System.Text.Json;
using Mono.Cecil;
using Mono.Cecil.Cil;

static class CardPickerTranslationTests
{
    const string PrefixType = "CardChoiceSpawnUniqueCardPatch.CardChoicePatchSpawnUniqueCard";
    const string ExpectedSourceHash = "1b9e8ef4c1d691d3217671096192b1b5d582edf895ac2043f49443552f8a4590";
    static readonly List<string> checks = new();
    static void Check(bool condition, string description)
    { if (!condition) throw new Exception(description); checks.Add(description); Console.WriteLine("PASS " + description); }
    static string Hash(string path) => Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(path))).ToLowerInvariant();
    static int Main(string[] args)
    {
        if (args.Length != 2) throw new ArgumentException("Expected repository root and receipt path.");
        string root = Path.GetFullPath(args[0]), receipt = Path.GetFullPath(args[1]);
        string relativeReceipt = Path.GetRelativePath(Path.Combine(root, "target"), receipt);
        if (Path.IsPathRooted(relativeReceipt) || relativeReceipt == ".." || relativeReceipt.StartsWith(".." + Path.DirectorySeparatorChar, StringComparison.Ordinal)
            || !receipt.EndsWith(".json", StringComparison.OrdinalIgnoreCase)) throw new ArgumentException("Receipt must be a JSON file inside this repository's target directory.");
        string cached = Path.Combine(root, "target/ducttape-plus-plus/live-game");
        string dependency = Path.Combine(cached, "BepInEx/plugins/Canna/DuctTapePlusPlus/Libraries/CardChoiceSpawnUniqueCardPatch.dll");
        Check(Hash(dependency) == ExpectedSourceHash, "Actual cached dependency has the pinned audited bytes");
        using var native = ModuleDefinition.ReadModule(Path.Combine(cached, "ROUNDS_Data/Managed/Assembly-CSharp.dll"));
        using var harmony = ModuleDefinition.ReadModule(Path.Combine(cached, "BepInEx/core/0Harmony.dll"));
        var game = new Game(native, harmony);
        NativeCardPickerFixtures.Run(native, Check);
        CardBarBindingFixtures.Run(native, game, Path.Combine(cached, "BepInEx/plugins/Canna/DuctTapePlusPlus/Libraries/UnboundLib.dll"),
            Path.Combine(cached, "BepInEx/plugins/Canna/DuctTapePlusPlus/Libraries/ModdingUtils.dll"), Check);
        using var source = ModuleDefinition.ReadModule(dependency);
        Check(source.Mvid.ToString("N") == "841c054f09a24ec6a9377cfc2e2475d0", "Actual Prefix module matches crashing log MVID");
        string teamBefore = TeamBranch(source);
        string conditionBefore = ConditionBodies(source);
        var changes = CannaFixRules.Apply(source, game);
        Check(changes.Any(c => c.Contains("public-API identity lookup", StringComparison.Ordinal)), "Actual Canna rule rewrites the player selector");
        Check(TeamBranch(source) == teamBefore, "Team selector instructions remain unchanged");
        Check(ConditionBodies(source) == conditionBefore, "Actual card condition, duplicate and eligibility bodies remain unchanged");
        Check(CannaFixRules.Apply(source, game).Count == 0, "Player selector translation is idempotent");
        Check(Prefix(source).Body.Instructions.Count(i => i.Operand is MethodReference m && m.Name == "Canna_GetPickerByPlayerID") == 1,
            "Translated actual Prefix has one local public-API identity resolver");
        Check(!source.GetType(PrefixType).Methods.Where(m => m.HasBody).SelectMany(m => m.Body.Instructions)
            .Any(i => i.Operand is MethodReference m && m.DeclaringType.FullName == "PlayerManager" && m.Name == "GetPlayerWithID"),
            "Normalized dependency never calls the native internal identity method");
        Check(!Prefix(source).Body.Instructions.Any(i => i.Operand is MethodReference m && m.Name == "get_Item"
            && m.DeclaringType.FullName == "System.Collections.Generic.List`1<Player>"), "Translated Prefix has no Player positional index");
        VerifyAmbiguityRefused(dependency, game);
        VerifyProjectileSelectorMetadata(Path.Combine(cached, "BepInEx/plugins/Canna/DuctTapePlusPlus/Libraries/ModdingUtils.dll"), game);
        var translated = BuildIsolatedFixture(source);
        var loaded = Assembly.Load(translated);
        var prefix = loaded.GetType(PrefixType).GetMethod("Prefix", BindingFlags.NonPublic | BindingFlags.Public | BindingFlags.Static);
        Check(prefix != null, "Actual translated Prefix loads against harmless fixture APIs");
        var nullCard = MakeCard("NullCard");
        loaded.GetType("CardChoiceSpawnUniqueCardPatch.CardChoiceSpawnUniqueCardPatch").GetField("NullCard",
            BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static).SetValue(null, nullCard);

        foreach (var (ids, picker) in new[] { (new[] { 0, 1 }, 0), (new[] { 0, 1 }, 1), (new[] { 1, 2 }, 2),
                     (new[] { 7, 13 }, 13), (new[] { 2, 0, 1 }, 0) })
        {
            var roster = ids.Select(id => MakePlayer(id, id + 20)).ToList();
            Reset(roster, picker, PickerType.Player);
            var chosen = MakeCard("Allowed"); ModdingUtils.Utils.Cards.instance.Candidates.Add(chosen);
            var spawned = InvokePrefix(prefix);
            Check(ReferenceEquals(ModdingUtils.Utils.Cards.instance.LastPlayer, roster.Single(p => p.PlayerID == picker)),
                $"Actual Prefix sends correct picker ID {picker} from roster [{string.Join(',', ids)}] to filtering");
            Check(spawned.GetComponent<CardInfo>().sourceCard == chosen, "Actual Prefix preserves chosen card source");
            Check(PlayerManager.instance.IdentityCalls == 0 && PlayerManager.instance.TeamCalls == 0,
                "PLAYER prefix resolves via public roster/identity APIs without calling internal method");
        }
        var teamRoster = new List<Player> { MakePlayer(12, 4), MakePlayer(25, 4), MakePlayer(4, 9) };
        Reset(teamRoster, 4, PickerType.Team);
        ModdingUtils.Utils.Cards.instance.Candidates.Add(MakeCard("TeamCard")); InvokePrefix(prefix);
        Check(ReferenceEquals(ModdingUtils.Utils.Cards.instance.LastPlayer, teamRoster[0])
            && PlayerManager.instance.TeamCalls == 1 && PlayerManager.instance.IdentityCalls == 0,
            "TEAM prefix preserves first member of selected team and bypasses player identity lookup");

        foreach (int missing in new[] { -1, 99 })
        {
            Reset(new List<Player> { MakePlayer(1, 1), MakePlayer(2, 2) }, missing, PickerType.Player);
            var helper = loaded.GetType(PrefixType).GetMethod("Canna_GetPickerByPlayerID", BindingFlags.NonPublic | BindingFlags.Static);
            Check(helper.Invoke(null, new object[] { PlayerManager.instance, missing }) is null,
                $"Actual injected identity helper resolves missing ID {missing} as null without guessing");
            ModdingUtils.Utils.Cards.instance.Candidates.Add(MakeCard("Unrelated"));
            bool rejected = false;
            try { InvokePrefix(prefix); } catch (TargetInvocationException ex) when (ex.InnerException is InvalidOperationException) { rejected = true; }
            Check(rejected && ModdingUtils.Utils.Cards.instance.LastPlayer is null,
                $"Missing picker ID {missing} never selects an unrelated player");
        }
        CheckCardRestrictions(prefix);
        Check(!AppDomain.CurrentDomain.GetAssemblies().Any(a => a.GetName().Name == "Assembly-CSharp" || a.GetName().Name == "ModdingUtils"
            || a.GetName().Name.StartsWith("UnityEngine", StringComparison.Ordinal)), "No real game, Unity or ModdingUtils assembly was loaded");
        Directory.CreateDirectory(Path.GetDirectoryName(receipt));
        File.WriteAllText(receipt, JsonSerializer.Serialize(new {
            passed = true, checks = checks.ToArray(), actual_dependency_sha256 = ExpectedSourceHash,
            rules_source_sha256 = Hash(Path.Combine(root, "mods/DuctTapePlusPlus/CannaFixRules.cs")),
            native_runtime_source_sha256 = Hash(Path.Combine(root, "mods/DuctTapePlusPlus/LocalCardPickerFixes.cs")),
            fixture_source_sha256 = Hash(Path.Combine(root, "mods/DuctTapePlusPlus/Tests/CardPickerTranslationTests.cs")),
            native_fixture_source_sha256 = Hash(Path.Combine(root, "mods/DuctTapePlusPlus/Tests/NativeCardPickerFixtures.cs")),
            fixture_api_source_sha256 = Hash(Path.Combine(root, "mods/DuctTapePlusPlus/Tests/CardPickerFixtureApis.cs")),
            actual_moddingutils_sha256 = Hash(Path.Combine(cached, "BepInEx/plugins/Canna/DuctTapePlusPlus/Libraries/ModdingUtils.dll")),
            actual_unboundlib_sha256 = Hash(Path.Combine(cached, "BepInEx/plugins/Canna/DuctTapePlusPlus/Libraries/UnboundLib.dll")),
            cardbar_fixture_source_sha256 = Hash(Path.Combine(root, "mods/DuctTapePlusPlus/Tests/CardBarBindingFixtures.cs")),
            native_metadata_sha256 = Hash(Path.Combine(cached, "ROUNDS_Data/Managed/Assembly-CSharp.dll")),
            game_files_changed = false, game_process_changed = false, real_game_or_mod_runtime_loaded = false,
            isolated_actual_prefix_and_conditions_executed = true, live_card_pick_verified = false,
            harmony_api_stubbed = true, real_harmony_registrations_verified = false,
            cardbar_downstream_button_ui_stubbed = true, unbound_carddata_storage_stubbed = true,
            native_offline_pick_downstream_stubbed = true,
            scope = "Actual pinned Prefix/condition IL normalized by linked rules; exact native Pick IL and production transpiler bound only to harmless API stubs"
        }, new JsonSerializerOptions { WriteIndented = true }) + "\n");
        Console.WriteLine("Card picker fixture checks: " + checks.Count); return 0;
    }

    static MethodDefinition Prefix(ModuleDefinition module) => module.GetType(PrefixType).Methods.Single(m => m.Name == "Prefix");
    static string TeamBranch(ModuleDefinition module) => string.Join("\n", Prefix(module).Body.Instructions
        .SkipWhile(i => i.Offset < 0x22).TakeWhile(i => i.Offset < 0x39).Select(i => i.OpCode + " " + i.Operand));
    static string ConditionBodies(ModuleDefinition module) => string.Join("\n", Scanner.AllTypes(module)
        .Where(t => t.FullName.StartsWith(PrefixType, StringComparison.Ordinal)).SelectMany(t => t.Methods)
        .Where(m => m.HasBody && m.Name != "Prefix" && m.Name != "Canna_GetPickerByPlayerID").Select(m => m.FullName + "\n" + string.Join("\n", m.Body.Instructions.Select(i => i.OpCode + " " + i.Operand))));
    static void VerifyAmbiguityRefused(string dependency, Game game)
    {
        using var ambiguous = ModuleDefinition.ReadModule(dependency);
        var type = ambiguous.GetType(PrefixType);
        var duplicate = new MethodDefinition("Prefix", Mono.Cecil.MethodAttributes.Private | Mono.Cecil.MethodAttributes.Static, ambiguous.TypeSystem.Boolean);
        duplicate.Body.Instructions.Add(Instruction.Create(OpCodes.Ldc_I4_0)); duplicate.Body.Instructions.Add(Instruction.Create(OpCodes.Ret)); type.Methods.Add(duplicate);
        bool denied = false;
        try { CannaFixRules.Apply(ambiguous, game); } catch (InvalidDataException) { denied = true; }
        Check(denied, "Ambiguous duplicate Prefix is refused rather than broadly rewritten");
        using var wrong = ModuleDefinition.ReadModule(dependency);
        var selector = Prefix(wrong).Body.Instructions.Single(i => i.Operand is FieldReference f && f.Name == "pickrID"
            && i.Previous?.Previous?.Operand is FieldReference previous && previous.Name == "players");
        selector.Operand = new FieldReference("unknownPicker", wrong.TypeSystem.Int32, selector.Operand is FieldReference current ? current.DeclaringType : null);
        denied = false;
        try { CannaFixRules.Apply(wrong, game); } catch (InvalidDataException) { denied = true; }
        Check(denied, "Unknown player selector pattern is refused");
        using var tampered = ModuleDefinition.ReadModule(dependency);
        CannaFixRules.Apply(tampered, game);
        var helper = tampered.GetType(PrefixType).Methods.Single(m => m.Name == "Canna_GetPickerByPlayerID");
        helper.Body.Instructions.First(i => i.OpCode == OpCodes.Ldc_I4_0).OpCode = OpCodes.Ldc_I4_1;
        denied = false;
        try { CannaFixRules.Apply(tampered, game); } catch (InvalidDataException) { denied = true; }
        Check(denied, "Tampered existing identity helper is refused");
    }
    static string MetadataBody(TypeDefinition type) => string.Join("\n", All(type).SelectMany(t => t.Methods)
        .Select(m => m.FullName + "|" + string.Join(";", m.CustomAttributes.Select(a => a.AttributeType.FullName))
            + "\n" + (m.HasBody ? string.Join("\n", m.Body.Instructions.Select(i => i.OpCode + " " + i.Operand)) : "")));
    static IEnumerable<TypeDefinition> All(TypeDefinition type)
    { yield return type; foreach (var nested in type.NestedTypes) foreach (var item in All(nested)) yield return item; }
    static void VerifyProjectileSelectorMetadata(string path, Game game)
    {
        using var source = ModuleDefinition.ReadModule(path);
        var names = new[] { "ModdingUtils.AIMinion.Patches.ProjectileInit_PatchGetCorrectPlayerOffline", "ModdingUtils.AIMinion.Patches.ProjectileInit_PatchGetCorrectPlayerRPCs" };
        var before = names.ToDictionary(n => n, n => MetadataBody(source.GetType(n)));
        string otherAnnotations = UnrelatedClassAnnotations(source, names);
        var selectorNames = names.ToDictionary(n => n, n => All(source.GetType(n)).SelectMany(t => t.Methods).Where(m => m.HasBody)
            .SelectMany(m => m.Body.Instructions).Where(i => i.OpCode == OpCodes.Ldstr && i.Operand is string text
                && (text.StartsWith("OFFLINE_", StringComparison.Ordinal) || text.StartsWith("RPCA_", StringComparison.Ordinal)))
            .Select(i => (string)i.Operand).ToArray());
        Check(selectorNames.Values.Sum(s => s.Length) == 6, "Actual ModdingUtils selectors retain six explicit projectile targets");
        var changes = CannaFixRules.Apply(source, game);
        Check(changes.Count(c => c.Contains("dynamic ProjectileInit targets", StringComparison.Ordinal)) == 2,
            "Only the two reviewed projectile class-target conflicts are normalized");
        Check(UnrelatedClassAnnotations(source, names) == otherAnnotations, "Unrelated ModdingUtils class annotations remain unchanged");
        foreach (var name in names)
        {
            var type = source.GetType(name);
            var marker = type.CustomAttributes.Single(a => a.AttributeType.FullName == "HarmonyLib.HarmonyPatch");
            Check(marker.ConstructorArguments.Count == 0 && marker.Fields.Count == 0 && marker.Properties.Count == 0,
                name + " retains only empty class Harmony marker");
            Check(MetadataBody(type) == before[name], name + " preserves selector, iterator and transpiler bodies/attributes");
            Check(type.Methods.Single(m => m.Name == "RPCMethods").CustomAttributes.Any(a => a.AttributeType.FullName == "HarmonyLib.HarmonyTargetMethods"),
                name + " preserves target-method discovery attribute");
        }
        Check(CannaFixRules.Apply(source, game).Count == 0, "Projectile selector marker normalization is idempotent");
        using var ambiguous = ModuleDefinition.ReadModule(path);
        var target = ambiguous.GetType(names[0]);
        var original = target.CustomAttributes.Single(a => a.AttributeType.FullName == "HarmonyLib.HarmonyPatch");
        var duplicate = new CustomAttribute(original.Constructor);
        foreach (var arg in original.ConstructorArguments) duplicate.ConstructorArguments.Add(arg);
        target.CustomAttributes.Add(duplicate);
        bool denied = false;
        try { CannaFixRules.Apply(ambiguous, game); } catch (InvalidDataException) { denied = true; }
        Check(denied, "Ambiguous projectile class Harmony annotations are refused");
        using var changed = ModuleDefinition.ReadModule(path);
        var targetName = All(changed.GetType(names[0])).SelectMany(t => t.Methods).Where(m => m.HasBody).SelectMany(m => m.Body.Instructions)
            .First(i => i.OpCode == OpCodes.Ldstr && (string)i.Operand == "OFFLINE_Init");
        targetName.Operand = "OFFLINE_UnknownInit";
        denied = false;
        try { CannaFixRules.Apply(changed, game); } catch (InvalidDataException) { denied = true; }
        Check(denied, "Unreviewed projectile target names are refused");
    }
    static string UnrelatedClassAnnotations(ModuleDefinition source, string[] excluded) => string.Join("\n", Scanner.AllTypes(source)
        .Where(t => !excluded.Contains(t.FullName)).Select(t => t.FullName + "|" + string.Join(";", t.CustomAttributes.Select(a => a.Constructor.FullName
            + "|" + Convert.ToHexString(a.GetBlob())))));
    static byte[] BuildIsolatedFixture(ModuleDefinition source)
    {
        var keep = source.GetType(PrefixType);
        var holder = source.GetType("CardChoiceSpawnUniqueCardPatch.CardChoiceSpawnUniqueCardPatch");
        foreach (var type in source.Types.ToArray()) if (type != keep && type != holder && type.Name != "<Module>") source.Types.Remove(type);
        holder.BaseType = source.TypeSystem.Object;
        holder.Methods.Clear(); holder.Properties.Clear(); holder.Events.Clear(); holder.Interfaces.Clear();
        foreach (var field in holder.Fields.ToArray()) if (field.Name != "NullCard") holder.Fields.Remove(field);
        source.CustomAttributes.Clear(); source.Assembly.CustomAttributes.Clear();
        foreach (var type in Scanner.AllTypes(source))
        {
            type.CustomAttributes.Clear();
            foreach (var method in type.Methods)
            {
                method.CustomAttributes.Clear();
                foreach (var parameter in method.Parameters) parameter.CustomAttributes.Clear();
                if (!method.HasBody) continue;
                foreach (var instruction in method.Body.Instructions)
                    if (instruction.Operand is MethodReference called && called.DeclaringType.FullName == "CardChoiceSpawnUniqueCardPatch.CustomCategories.CustomCardCategories")
                        instruction.Operand = source.ImportReference(typeof(CardChoiceSpawnUniqueCardPatch.CustomCategories.CustomCardCategories).GetProperty("CanDrawMultipleCategory").GetMethod);
            }
        }
        var fixtureScope = new AssemblyNameReference(typeof(CardChoice).Assembly.GetName().Name, typeof(CardChoice).Assembly.GetName().Version);
        source.AssemblyReferences.Add(fixtureScope);
        foreach (var reference in source.GetTypeReferences())
            if (reference.Scope is AssemblyNameReference external && external.Name != "mscorlib" && external.Name != "netstandard"
                && !external.Name.StartsWith("System", StringComparison.Ordinal)) reference.Scope = fixtureScope;
        // Also rebind references introduced by imported native member signatures,
        // which Cecil can retain through generic/signature objects outside its
        // enumerated TypeRef table until the module is written.
        foreach (var reference in source.AssemblyReferences)
            if (reference.Name != "mscorlib" && reference.Name != "netstandard"
                && !reference.Name.StartsWith("System", StringComparison.Ordinal))
            { reference.Name = fixtureScope.Name; reference.Version = fixtureScope.Version; reference.PublicKeyToken = null; }
        source.Assembly.Name.Name = "Isolated.Canna.CardPickerActualPrefix";
        source.Name = source.Assembly.Name.Name + ".dll";
        using var bytes = new MemoryStream(); source.Write(bytes); return bytes.ToArray();
    }
    static Player MakePlayer(int id, int team)
    {
        var gameObject = new UnityEngine.GameObject { name = "FixturePlayer" + id };
        var player = gameObject.Attach(new Player { PlayerID = id, TeamID = team });
        player.data = gameObject.Attach(new CharacterData()); gameObject.Attach(new Holding()); return player;
    }
    static CardInfo MakeCard(string name)
    { var gameObject = new UnityEngine.GameObject { name = name }; return gameObject.Attach(new CardInfo()); }
    static void Reset(List<Player> roster, int picker, PickerType kind)
    {
        PlayerManager.instance = new PlayerManager { players = roster };
        CardChoice.instance = new CardChoice { pickrID = picker, pickerType = kind, cards = new[] { MakeCard("CatalogMarker") } };
        ModdingUtils.Utils.Cards.instance = new ModdingUtils.Utils.Cards();
    }
    static UnityEngine.GameObject InvokePrefix(MethodInfo prefix)
    {
        object[] args = { null, CardChoice.instance, new UnityEngine.Vector3(), new UnityEngine.Quaternion() };
        Check((bool)prefix.Invoke(null, args) == false, "Actual unique-card Prefix still owns spawning");
        return (UnityEngine.GameObject)args[0];
    }
    static void CheckCardRestrictions(MethodInfo prefix)
    {
        var picker = MakePlayer(7, 1);
        var other = MakePlayer(13, 2);
        Reset(new List<Player> { other, picker }, 7, PickerType.Player);
        var blocked = new CardCategory { name = "Blocked" };
        var bannedByOwned = new CardCategory { name = "ExistingCardExclusion" };
        picker.BlockedCategories.Add(blocked);
        var owned = MakeCard("Owned"); owned.blacklistedCategories = new[] { bannedByOwned }; picker.data.currentCards.Add(owned);
        var first = MakeCard("BlockedCandidate"); first.categories = new[] { blocked };
        var second = MakeCard("ExcludedByOwned"); second.categories = new[] { bannedByOwned };
        var duplicate = MakeCard("Duplicate"); CardChoice.instance.spawnedCards.Add(new UnityEngine.GameObject { name = "Duplicate(Clone)" });
        var accepted = MakeCard("AllowedAfterRestrictions");
        ModdingUtils.Utils.Cards.instance.Candidates.AddRange(new[] { first, second, duplicate, accepted });
        var result = InvokePrefix(prefix);
        Check(result.GetComponent<CardInfo>().sourceCard == accepted && ModdingUtils.Utils.Cards.instance.EligibilityCalls == 4,
            "Actual condition preserves picker eligibility, category exclusions and duplicate-card rejection");
        Reset(new List<Player> { picker }, 7, PickerType.Player);
        var repeat = MakeCard("Duplicate"); repeat.categories = new[] { CardChoiceSpawnUniqueCardPatch.CustomCategories.CustomCardCategories.CanDrawMultipleCategory };
        CardChoice.instance.spawnedCards.Add(new UnityEngine.GameObject { name = "Duplicate(Clone)" });
        ModdingUtils.Utils.Cards.instance.Candidates.Add(repeat);
        Check(InvokePrefix(prefix).GetComponent<CardInfo>().sourceCard == repeat, "CanDrawMultiple category continues to allow duplicates");
    }
}
