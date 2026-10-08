// Native body + actual Unbound Prefix emitted against fixture APIs; no real UI/game.
using System.Reflection;
using System.Reflection.Emit;
using HarmonyLib;
using Mono.Cecil;
using Mono.Cecil.Cil;
using EmitOpCodes = System.Reflection.Emit.OpCodes;
using Bindings = RoundsPort.Runtime.Canna_CardBarSlotBindings;

static class CardBarBindingFixtures
{
    delegate void AddBody(CardBarHandler handler, int id, CardInfo card);
    public static void Run(ModuleDefinition native, Game game, string unboundPath, string moddingPath, Action<bool, string> check)
    {
        using var unbound = ModuleDefinition.ReadModule(unboundPath);
        VerifyRebuild(unbound, game, unboundPath, check);
        var nativeMethod = native.GetType("CardBarHandler").Methods.Single(m => m.Name == "AddCard");
        var prefixMethod = unbound.GetType("UnboundLib.Patches.CardBarHandler_Patch_AddCard").Methods.Single(m => m.Name == "Prefix");
        var beforePrefix = string.Join("\n", prefixMethod.Body.Instructions.Select(i => i.ToString()));
        var prefix = CompilePrefix(prefixMethod);
        var body = CompileBody(nativeMethod, check);
        check(string.Join("\n", prefixMethod.Body.Instructions.Select(i => i.ToString())) == beforePrefix,
            "Actual Unbound CardData Prefix keeps original identity argument unchanged");
        foreach (var (ids, actualId) in new[] { (new[] { 0, 1 }, 1), (new[] { 1, 2 }, 2), (new[] { 13, 7 }, 7) })
        {
            var players = ids.Select(Player).ToList(); var bars = ids.Select(_ => new CardBar()).ToArray();
            PlayerManager.instance = new PlayerManager { players = players };
            var handler = new CardBarHandler { FixtureBars = bars };
            RoundsPort.Runtime.Canna_CardBarRebuildBindings_Fix.Postfix(handler);
            var card = Card("BoundCard" + actualId); UnboundLib.Cards.CardData.Cards.Clear();
            prefix(actualId, card); body(handler, actualId, card);
            int slot = Array.IndexOf(ids, actualId);
            check(bars[slot].Added.Single() == card && bars.Where((_, i) => i != slot).All(bar => bar.Added.Count == 0),
                $"Native body maps actual ID {actualId} to bound card bar in [{string.Join(',', ids)}]");
            check(UnboundLib.Cards.CardData.Cards.Count == 1 && UnboundLib.Cards.CardData.Cards[actualId].Single() == card.CardName,
                "Actual Unbound Prefix records CardData under original PlayerID rather than display slot");
        }
        var first = Player(7); var second = Player(13); var stableBars = new[] { new CardBar(), new CardBar() };
        PlayerManager.instance = new PlayerManager { players = new() { first, second } }; Bindings.Bind(stableBars, PlayerManager.instance.players);
        PlayerManager.instance.players.Reverse();
        check(ReferenceEquals(Bindings.Resolve(stableBars, 7), stableBars[0]) && ReferenceEquals(Bindings.Resolve(stableBars, 13), stableBars[1]),
            "Roster reorder after Rebuild preserves original player-object/bar ownership");
        first.PlayerID = 8; Reject(() => Bindings.Resolve(stableBars, 8), "Changed bound player ID is refused", check); first.PlayerID = 7;
        PlayerManager.instance.players[1] = Player(7); Reject(() => Bindings.Resolve(stableBars, 7), "Replaced player object with same ID is refused", check);
        PlayerManager.instance.players = new() { first, second }; stableBars[0] = new CardBar();
        Reject(() => Bindings.Resolve(stableBars, 7), "Replaced bound bar object is refused", check);
        var freshBars = new[] { new CardBar(), new CardBar() }; Bindings.Bind(freshBars, PlayerManager.instance.players);
        (freshBars[0], freshBars[1]) = (freshBars[1], freshBars[0]); Reject(() => Bindings.Resolve(freshBars, 7), "In-place card bar reorder is refused", check);
        PlayerManager.instance.players = new() { first, second };
        foreach (int absent in new[] { -1, 99 }) { var bars = new[] { new CardBar(), new CardBar() }; Bindings.Bind(bars, PlayerManager.instance.players); Reject(() => Bindings.Resolve(bars, absent), "Absent bound picker ID " + absent + " is refused", check); }
        Reject(() => Bindings.Bind(new[] { new CardBar(), new CardBar() }, new[] { Player(7), Player(7) }), "Duplicate roster IDs are refused", check);
        var repeatedBar = new CardBar(); Reject(() => Bindings.Bind(new[] { repeatedBar, repeatedBar }, new[] { Player(7), Player(13) }), "Duplicate bar objects are refused", check);
        Reject(() => Bindings.Bind(new CardBar[] { null }, new[] { Player(7) }), "Null bar is refused", check);
        Reject(() => Bindings.Bind(new[] { new CardBar() }, new[] { Player(7), Player(13) }), "Mismatched bar count is refused", check);
        PlayerManager.instance.players = new() { Player(0), Player(2) }; var menuSlots = new[] { new CardBar(), new CardBar(), new CardBar(), new CardBar() };
        check(ReferenceEquals(Bindings.Resolve(menuSlots, 2), menuSlots[2]), "Unbound four-slot menu preserves original positional slot API despite unrelated roster");
        PlayerManager.instance.players = new() { Player(1), Player(0) };
        check(ReferenceEquals(Bindings.Resolve(menuSlots, 1), menuSlots[1]), "Unbound slot API preserves position without claiming player identity ownership");
        PlayerManager.instance = null;
        check(ReferenceEquals(Bindings.Resolve(menuSlots, 3), menuSlots[3]), "Unbound native menu slots work before a PlayerManager exists");
        Reject(() => Bindings.Resolve(menuSlots, -1), "Unbound negative slot is refused", check);
        Reject(() => Bindings.Resolve(menuSlots, 4), "Unbound out-of-range slot is refused", check);
        Reject(() => Bindings.Resolve(new CardBar[] { null }, 0), "Unbound null selected bar is refused", check);
        PlayerManager.instance = new PlayerManager();
        var empty = Array.Empty<CardBar>(); PlayerManager.instance.players.Clear(); Bindings.Bind(empty, PlayerManager.instance.players);
        check(true, "Empty menu/reset rebuild is accepted"); Reject(() => Bindings.Resolve(empty, 0), "Empty roster denies every picker", check);
        CheckBodyRefusals(nativeMethod, check);
        CheckActualBoundsGuard(moddingPath, game, body, prefix, check);
    }
    static void CheckActualBoundsGuard(string path, Game game, AddBody body, Action<int, CardInfo> prefix, Action<bool, string> check)
    {
        const string name = "ModdingUtils.AIMinion.Patches.CardBarHandlerPatchAddCard";
        using var source = ModuleDefinition.ReadModule(path);
        var method = source.GetType(name).Methods.Single(m => m.Name == "Prefix");
        var before = string.Join("\n", method.Body.Instructions.Select(i => i.ToString()));
        CannaFixRules.Apply(source, game);
        check(string.Join("\n", method.Body.Instructions.Select(i => i.ToString())) == before,
            "Pinned ModdingUtils bounds guard is validated without changing its IL");
        var reflected = CompileGuard(method);
        var guard = (Func<CardBarHandler, int, bool>)reflected.CreateDelegate(typeof(Func<CardBarHandler, int, bool>));
        check(RoundsPort.Runtime.Canna_ModdingUtilsCardBarBounds_Fix.KnownBoundsBody(reflected),
            "Runtime recognizes actual bounds-only guard IL with tokens bound to harmless APIs");
        var players = new[] { Player(0), Player(2) }; var bars = new[] { new CardBar(), new CardBar() };
        PlayerManager.instance = new PlayerManager { players = players.ToList() };
        var handler = new CardBarHandler { FixtureBars = bars }; Bindings.Bind(bars, PlayerManager.instance.players);
        bool result = guard(handler, 2);
        check(!result, "Actual ModdingUtils guard reproduces false result for valid sparse player ID2 and two bars");
        RoundsPort.Runtime.Canna_ModdingUtilsCardBarBounds_Fix.Postfix(handler, 2, true, reflected, ref result);
        check(result, "Guard repair admits only the live player2 with a verified bound bar");
        var card = Card("GuardedSparseCard"); UnboundLib.Cards.CardData.Cards.Clear();
        prefix(2, card); if (result) body(handler, 2, card);
        check(bars[1].Added.Single() == card && bars[0].Added.Count == 0 && UnboundLib.Cards.CardData.Cards[2].Single() == card.CardName,
            "Actual guard plus repaired native body adds exactly one button to player2 bar and keeps CardData ID2");
        foreach (int absent in new[] { 3, 99 })
        {
            result = guard(handler, absent);
            RoundsPort.Runtime.Canna_ModdingUtilsCardBarBounds_Fix.Postfix(handler, absent, true, reflected, ref result);
            check(!result, "Absent bound player ID" + absent + " stays rejected by the original guard");
        }
        result = false;
        RoundsPort.Runtime.Canna_ModdingUtilsCardBarBounds_Fix.Postfix(handler, 2, false, reflected, ref result);
        check(!result, "A skipped original guard retains another Prefix veto");
        result = false;
        RoundsPort.Runtime.Canna_ModdingUtilsCardBarBounds_Fix.Postfix(handler, 0, true, reflected, ref result);
        check(!result, "False result outside original ID-bounds condition is not overridden");
        result = true;
        RoundsPort.Runtime.Canna_ModdingUtilsCardBarBounds_Fix.Postfix(handler, 99, true, reflected, ref result);
        check(result, "Already true result is preserved without broad validation or rewriting");
        foreach (var kind in new[] { "Prefix", "Transpiler", "Finalizer", "Postfix" })
        {
            var patches = new Patches(); var patch = new Patch { PatchMethod = typeof(CardBarBindingFixtures).GetMethod(nameof(OtherCondition), BindingFlags.NonPublic | BindingFlags.Static) };
            (kind == "Prefix" ? patches.Prefixes : kind == "Transpiler" ? patches.Transpilers : kind == "Finalizer" ? patches.Finalizers : patches.Postfixes).Add(patch);
            Harmony.FixturePatches[reflected] = patches; result = false;
            RoundsPort.Runtime.Canna_ModdingUtilsCardBarBounds_Fix.Postfix(handler, 2, true, reflected, ref result);
            check(!result, "Guard repair preserves competing Harmony " + kind + " conditions");
        }
        Harmony.FixturePatches.Clear();
        var unbound = new[] { new CardBar(), new CardBar(), new CardBar(), new CardBar() }; var menu = new CardBarHandler { FixtureBars = unbound };
        foreach (int id in new[] { 2, 4, 99 })
        {
            bool original = guard(menu, id); result = original;
            RoundsPort.Runtime.Canna_ModdingUtilsCardBarBounds_Fix.Postfix(menu, id, true, reflected, ref result);
            check(result == original, "Unbound four-slot menu/AI guard keeps original result for ID" + id);
        }
        players[1].PlayerID = 3;
        Reject(() => { bool value = false; RoundsPort.Runtime.Canna_ModdingUtilsCardBarBounds_Fix.Postfix(handler, 2, true, reflected, ref value); },
            "Guard repair rejects changed bound player ownership", check);
        players[1].PlayerID = 2; bars[1] = new CardBar();
        Reject(() => { bool value = false; RoundsPort.Runtime.Canna_ModdingUtilsCardBarBounds_Fix.Postfix(handler, 2, true, reflected, ref value); },
            "Guard repair rejects changed bound bar ownership", check);
        using var changed = ModuleDefinition.ReadModule(path);
        changed.GetType(name).Methods.Single(m => m.Name == "Prefix").Body.Instructions.Single(i => i.OpCode == Mono.Cecil.Cil.OpCodes.Clt).OpCode = Mono.Cecil.Cil.OpCodes.Cgt;
        bool refused = false; try { CannaFixRules.Apply(changed, game); } catch (InvalidDataException) { refused = true; }
        check(refused, "Unreviewed ModdingUtils guard condition is refused during payload normalization");
        check(!RoundsPort.Runtime.Canna_ModdingUtilsCardBarBounds_Fix.KnownBoundsBody(CompileGuard(changed.GetType(name).Methods.Single(m => m.Name == "Prefix"))),
            "Runtime independently rejects a changed guard condition");
        using var ambiguous = ModuleDefinition.ReadModule(path);
        ambiguous.GetType(name).Methods.Add(new MethodDefinition("Prefix", Mono.Cecil.MethodAttributes.Private | Mono.Cecil.MethodAttributes.Static, ambiguous.TypeSystem.Boolean));
        refused = false; try { CannaFixRules.Apply(ambiguous, game); } catch (InvalidDataException) { refused = true; }
        check(refused, "Ambiguous ModdingUtils guard overload is refused");
    }
    static bool OtherCondition() => false;
    static MethodInfo CompileGuard(MethodDefinition source)
    {
        var assembly = AssemblyBuilder.DefineDynamicAssembly(new AssemblyName("Canna.Fixture.Bounds." + Guid.NewGuid().ToString("N")), AssemblyBuilderAccess.Run);
        var type = assembly.DefineDynamicModule("Fixture").DefineType("FixtureBounds", System.Reflection.TypeAttributes.Public | System.Reflection.TypeAttributes.Abstract | System.Reflection.TypeAttributes.Sealed);
        var method = type.DefineMethod("Prefix", System.Reflection.MethodAttributes.Private | System.Reflection.MethodAttributes.Static,
            typeof(bool), new[] { typeof(CardBarHandler), typeof(int) });
        var il = method.GetILGenerator();
        foreach (var instruction in NativeCardPickerFixtures.Convert(source, il))
        { foreach (var label in instruction.labels) il.MarkLabel(label); NativeCardPickerFixtures.Emit(il, instruction); }
        return type.CreateType().GetMethod("Prefix", BindingFlags.NonPublic | BindingFlags.Static);
    }
    static void VerifyRebuild(ModuleDefinition unbound, Game game, string path, Action<bool, string> check)
    {
        const string name = "UnboundLib.Extensions.CardBarHandlerExtensions";
        var type = unbound.GetType(name); var method = type.Methods.Single(m => m.Name == "Rebuild");
        var before = method.Body.Instructions.Select(i => i.OpCode + " " + i.Operand).ToArray();
        var predicate = type.NestedTypes.SelectMany(t => t.Methods).Single(m => m.Name.Contains("<Rebuild>b__"));
        string beforePredicate = string.Join("\n", predicate.Body.Instructions.Select(i => i.ToString()));
        string[] prefixBefore = unbound.GetType("UnboundLib.Patches.CardBarHandler_Patch_AddCard").Methods.Single(m => m.Name == "Prefix").Body.Instructions.Select(i => i.ToString()).ToArray();
        var changes = CannaFixRules.Apply(unbound, game);
        check(changes.Any(c => c.Contains("color owners use roster slots", StringComparison.Ordinal)), "Actual pinned Unbound Rebuild color-owner lookup is normalized");
        var after = method.Body.Instructions.Select(i => i.OpCode + " " + i.Operand).ToArray();
        check(before.Length == after.Length && before.Zip(after).Count(pair => pair.First != pair.Second) == 3,
            "Unbound Rebuild normalization changes exactly three selector instructions");
        check(string.Join("\n", predicate.Body.Instructions.Select(i => i.ToString())) == beforePredicate,
            "Reviewed Rebuild closure predicate remains unchanged");
        check(unbound.GetType("UnboundLib.Patches.CardBarHandler_Patch_AddCard").Methods.Single(m => m.Name == "Prefix").Body.Instructions.Select(i => i.ToString()).SequenceEqual(prefixBefore),
            "Rebuild normalization preserves actual Unbound AddCard/CardData identity Prefix");
        check(CannaFixRules.Apply(unbound, game).Count == 0, "Unbound color-owner normalization is idempotent");
        using var ambiguous = ModuleDefinition.ReadModule(path);
        var originalType = ambiguous.GetType(name);
        originalType.Methods.Add(new MethodDefinition("Rebuild", Mono.Cecil.MethodAttributes.Public | Mono.Cecil.MethodAttributes.Static, ambiguous.TypeSystem.Void));
        bool refused = false; try { CannaFixRules.Apply(ambiguous, game); } catch (InvalidDataException) { refused = true; }
        check(refused, "Ambiguous Unbound Rebuild method is refused");
        using var unknown = ModuleDefinition.ReadModule(path);
        var capture = unknown.GetType(name).NestedTypes.SelectMany(t => t.Methods).Single(m => m.Name.Contains("<Rebuild>b__"));
        capture.Body.Instructions.Single(i => i.OpCode == Mono.Cecil.Cil.OpCodes.Ceq).OpCode = Mono.Cecil.Cil.OpCodes.Add;
        refused = false; try { CannaFixRules.Apply(unknown, game); } catch (InvalidDataException) { refused = true; }
        check(refused, "Unreviewed Unbound closure predicate is refused");
    }
    static AddBody CompileBody(MethodDefinition method, Action<bool, string> check)
    {
        var dynamic = new DynamicMethod("FixtureNativeCardBarAdd", typeof(void), new[] { typeof(CardBarHandler), typeof(int), typeof(CardInfo) }, typeof(CardBarBindingFixtures).Module, true);
        var il = dynamic.GetILGenerator(); var original = NativeCardPickerFixtures.Convert(method, il); string before = NativeCardPickerFixtures.Snapshot(original);
        var source = typeof(CardBarHandler).GetMethod(nameof(CardBarHandler.AddCard));
        var code = RoundsPort.Runtime.Canna_CardBarAddCard_Fix.Transpiler(original, source).ToList();
        check(NativeCardPickerFixtures.Snapshot(original) == before, "Card bar transpiler preserves original input");
        check(code.Zip(original).Count(pair => pair.First.opcode != pair.Second.opcode || !Equals(pair.First.operand, pair.Second.operand)) == 1,
            "Native AddCard changes only array-element lookup to verified binding");
        check(NativeCardPickerFixtures.Snapshot(RoundsPort.Runtime.Canna_CardBarAddCard_Fix.Transpiler(code, source)) == NativeCardPickerFixtures.Snapshot(code), "Native card-bar transpiler is idempotent");
        foreach (var instruction in code) { foreach (var label in instruction.labels) il.MarkLabel(label); NativeCardPickerFixtures.Emit(il, instruction); }
        return (AddBody)dynamic.CreateDelegate(typeof(AddBody));
    }
    static Action<int, CardInfo> CompilePrefix(MethodDefinition prefix)
    {
        var dynamic = new DynamicMethod("FixtureActualUnboundAddPrefix", typeof(void), new[] { typeof(int), typeof(CardInfo) }, typeof(CardBarBindingFixtures).Module, true);
        var il = dynamic.GetILGenerator(); foreach (var instruction in NativeCardPickerFixtures.Convert(prefix, il)) NativeCardPickerFixtures.Emit(il, instruction);
        return (Action<int, CardInfo>)dynamic.CreateDelegate(typeof(Action<int, CardInfo>));
    }
    static void CheckBodyRefusals(MethodDefinition method, Action<bool, string> check)
    {
        var dynamic = new DynamicMethod("FixtureCardBarRefusals", typeof(void), System.Type.EmptyTypes); var il = dynamic.GetILGenerator();
        var source = typeof(CardBarHandler).GetMethod(nameof(CardBarHandler.AddCard));
        var ambiguous = NativeCardPickerFixtures.Convert(method, il); ambiguous.Add(new CodeInstruction(EmitOpCodes.Nop));
        Reject(() => RoundsPort.Runtime.Canna_CardBarAddCard_Fix.Transpiler(ambiguous, source).ToArray(), "Unexpected native AddCard instruction count is refused", check);
        var wrong = NativeCardPickerFixtures.Convert(method, il); wrong[2].opcode = EmitOpCodes.Ldarg_2;
        Reject(() => RoundsPort.Runtime.Canna_CardBarAddCard_Fix.Transpiler(wrong, source).ToArray(), "Unexpected native AddCard selector argument is refused", check);
        var labeled = NativeCardPickerFixtures.Convert(method, il); labeled[3].labels.Add(il.DefineLabel());
        Reject(() => RoundsPort.Runtime.Canna_CardBarAddCard_Fix.Transpiler(labeled, source).ToArray(), "Branch into native card-bar replacement is refused", check);
    }
    static void Reject(Action action, string description, Action<bool, string> check)
    { bool denied = false; try { action(); } catch (InvalidOperationException) { denied = true; } check(denied, description); }
    static Player Player(int id) { var obj = new UnityEngine.GameObject(); var player = obj.Attach(new Player { PlayerID = id }); player.data = obj.Attach(new CharacterData()); return player; }
    static CardInfo Card(string name) { var obj = new UnityEngine.GameObject { name = name }; return obj.Attach(new CardInfo()); }
}
