// Exact native Pick IL + harmless Harmony API surfaces + production transpiler.
// Dynamic method binds every API reference to harmless fixtures; no game DLL executes.
using System.Reflection;
using System.Reflection.Emit;
using HarmonyLib;
using Mono.Cecil;
using Mono.Cecil.Cil;
using EmitOpCodes = System.Reflection.Emit.OpCodes;

static class NativeCardPickerFixtures
{
    static readonly Dictionary<string, System.Reflection.Emit.OpCode> opcodes = typeof(EmitOpCodes).GetFields(BindingFlags.Public | BindingFlags.Static)
        .Where(f => f.FieldType == typeof(System.Reflection.Emit.OpCode)).Select(f => (System.Reflection.Emit.OpCode)f.GetValue(null)).ToDictionary(o => o.Name);
    static readonly MethodInfo signature = typeof(ApplyCardStats).GetMethod(nameof(ApplyCardStats.Pick));
    delegate void PickBody(ApplyCardStats card, int id, bool force, PickerType kind);
    public static void Run(ModuleDefinition native, Action<bool, string> check)
    {
        var source = native.GetType("ApplyCardStats").Methods.Single(m => m.Name == "Pick" && m.Parameters.Count == 3);
        foreach (var (ids, picker) in new[] { (new[] { 0, 1 }, 1), (new[] { 1, 2 }, 2), (new[] { 13, 7 }, 7) })
        {
            PlayerManager.instance = new PlayerManager { players = ids.Select(id => Player(id, id + 20)).ToList() };
            var target = PlayerManager.instance.players.Single(p => p.PlayerID == picker);
            var card = Card();
            Compile(source, check)(card, picker, false, PickerType.Player);
            check(card.Applied.Length == 1 && ReferenceEquals(card.Applied[0], target),
                $"Transpiled actual native Pick applies only actual ID {picker} in [{string.Join(',', ids)}]");
            check(target.data.currentCards.Count == 1 && PlayerManager.instance.players.Where(p => p != target).All(p => p.data.currentCards.Count == 0),
                "Native offline dispatch keeps the selected player's card ownership");
        }
        PlayerManager.instance = new PlayerManager { players = new() { Player(12, 4), Player(25, 4), Player(4, 9) } };
        var teamCard = Card(); Compile(source, check)(teamCard, 4, false, PickerType.Team);
        check(teamCard.Applied.Length == 2 && teamCard.Applied.All(p => p.TeamID == 4) && PlayerManager.instance.IdentityCalls == 0,
            "Transpiled actual native Pick preserves whole-team dispatch");
        CheckRefusals(source, check);
    }
    static PickBody Compile(MethodDefinition source, Action<bool, string> check)
    {
        var dynamic = new DynamicMethod("FixtureNativeCardPick", typeof(void), new[] { typeof(ApplyCardStats), typeof(int), typeof(bool), typeof(PickerType) }, typeof(NativeCardPickerFixtures).Module, true);
        var il = dynamic.GetILGenerator();
        var original = Convert(source, il);
        var before = Snapshot(original);
        var code = RoundsPort.Runtime.Canna_ApplyCardStatsPicker_Fix.Transpiler(original, signature).ToList();
        check(Snapshot(original) == before, "Production native transpiler preserves its input instruction stream");
        check(code.Count == original.Count && code.Zip(original).Count(pair => pair.First.opcode != pair.Second.opcode || !Equals(pair.First.operand, pair.Second.operand)) == 2,
            "Native transpiler changes exactly the list load and identity call");
        check(code.Zip(original).All(pair => pair.First.labels.SequenceEqual(pair.Second.labels) && pair.First.blocks.SequenceEqual(pair.Second.blocks)),
            "Native transpiler preserves every branch label and exception annotation");
        var again = RoundsPort.Runtime.Canna_ApplyCardStatsPicker_Fix.Transpiler(code, signature).ToList();
        check(Snapshot(again) == Snapshot(code), "Native transpiler is idempotent");
        foreach (var instruction in code) { foreach (var label in instruction.labels) il.MarkLabel(label); Emit(il, instruction); }
        return (PickBody)dynamic.CreateDelegate(typeof(PickBody));
    }
    static void CheckRefusals(MethodDefinition source, Action<bool, string> check)
    {
        var method = new DynamicMethod("FixtureRefusal", typeof(void), System.Type.EmptyTypes);
        var code = Convert(source, method.GetILGenerator());
        int lookup = code.FindIndex(i => i.opcode == EmitOpCodes.Callvirt && i.operand is MethodInfo m && m.DeclaringType == typeof(List<Player>) && m.Name == "get_Item");
        var ambiguous = code.Select(i => new CodeInstruction(i)).ToList(); ambiguous.Add(new CodeInstruction(code[lookup]));
        Refuses(ambiguous, signature, "Duplicate native player lookup fails closed", check);
        var wrong = code.Select(i => new CodeInstruction(i)).ToList(); wrong[lookup - 1].opcode = EmitOpCodes.Ldarg_2;
        Refuses(wrong, signature, "Unknown native picker argument pattern is refused", check);
        var labeled = code.Select(i => new CodeInstruction(i)).ToList(); labeled[lookup - 2].labels.Add(method.GetILGenerator().DefineLabel());
        Refuses(labeled, signature, "Branch into rewritten native selector middle is refused", check);
        var blocked = code.Select(i => new CodeInstruction(i)).ToList(); blocked[lookup].blocks.Add(new ExceptionBlock(ExceptionBlockType.BeginExceptionBlock));
        Refuses(blocked, signature, "Exception boundary in rewritten selector is refused", check);
        Refuses(code, typeof(CardChoice).GetMethod("ToString"), "Unrelated original method is refused", check);
    }
    static void Refuses(List<CodeInstruction> code, MethodBase original, string description, Action<bool, string> check)
    {
        string before = Snapshot(code); bool refused = false;
        try { RoundsPort.Runtime.Canna_ApplyCardStatsPicker_Fix.Transpiler(code, original).ToArray(); }
        catch (InvalidOperationException) { refused = true; }
        check(refused && Snapshot(code) == before, description + " without partially changing input");
    }
    internal static string Snapshot(IEnumerable<CodeInstruction> code) => string.Join("\n", code.Select(i => i.opcode + "|" + i.operand + "|" + i.labels.Count + "|" + i.blocks.Count));
    internal static List<CodeInstruction> Convert(MethodDefinition source, ILGenerator il)
    {
        var locals = source.Body.Variables.Select(v => il.DeclareLocal(Type(v.VariableType))).ToArray();
        var targets = source.Body.Instructions.SelectMany(i => i.Operand is Instruction target ? new[] { target } : i.Operand is Instruction[] many ? many : Array.Empty<Instruction>()).Distinct().ToDictionary(i => i, _ => il.DefineLabel());
        var result = new List<CodeInstruction>();
        foreach (var instruction in source.Body.Instructions)
        {
            object operand = instruction.Operand switch {
                Instruction target => targets[target], Instruction[] many => many.Select(t => targets[t]).ToArray(),
                VariableDefinition variable => locals[variable.Index], ParameterDefinition parameter => parameter.Index + 1,
                MethodReference method => Method(method), FieldReference field => Type(field.DeclaringType).GetField(field.Name, BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance | BindingFlags.Static),
                TypeReference type => Type(type), _ => instruction.Operand
            };
            var converted = new CodeInstruction(opcodes[instruction.OpCode.Name], operand);
            if (targets.TryGetValue(instruction, out var label)) converted.labels.Add(label);
            result.Add(converted);
        }
        return result;
    }
    static Type Type(TypeReference reference)
    {
        if (reference is ArrayType array) return Type(array.ElementType).MakeArrayType();
        if (reference is GenericInstanceType generic && generic.ElementType.FullName == "System.Collections.Generic.List`1") return typeof(List<>).MakeGenericType(generic.GenericArguments.Select(Type).ToArray());
        string name = reference.FullName.Replace('/', '+');
        return typeof(CardChoice).Assembly.GetType(name) ?? System.Type.GetType(name) ?? throw new Exception("Missing harmless type " + name);
    }
    static MethodInfo Method(MethodReference reference)
    {
        var target = Type(reference.DeclaringType);
        if (reference is GenericInstanceMethod generic)
            return target.GetMethods(BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance | BindingFlags.Static).Single(m => m.Name == reference.Name && m.IsGenericMethodDefinition && m.GetParameters().Length == reference.Parameters.Count)
                .MakeGenericMethod(generic.GenericArguments.Select(Type).ToArray());
        return target.GetMethod(reference.Name, BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance | BindingFlags.Static, null, reference.Parameters.Select(p => Type(p.ParameterType)).ToArray(), null)
            ?? throw new Exception("Missing harmless method " + reference.FullName);
    }
    internal static void Emit(ILGenerator il, CodeInstruction instruction)
    {
        var op = instruction.opcode;
        switch (instruction.operand) {
            case null: il.Emit(op); break; case Label label: il.Emit(op, label); break; case Label[] labels: il.Emit(op, labels); break;
            case LocalBuilder local: il.Emit(op, local); break; case MethodInfo method: il.Emit(op, method); break;
            case FieldInfo field: il.Emit(op, field); break; case Type type: il.Emit(op, type); break; case string text: il.Emit(op, text); break;
            case int value when op.OperandType == System.Reflection.Emit.OperandType.ShortInlineI: il.Emit(op, (sbyte)value); break;
            case int value: il.Emit(op, value); break; case sbyte value: il.Emit(op, value); break;
            default: throw new Exception("Unsupported fixture operand " + instruction.operand.GetType().Name);
        }
    }
    static Player Player(int id, int team)
    { var obj = new UnityEngine.GameObject(); var player = obj.Attach(new Player { PlayerID = id, TeamID = team }); player.data = obj.Attach(new CharacterData()); return player; }
    static ApplyCardStats Card()
    { var obj = new UnityEngine.GameObject { name = "FixtureNativeCard" }; obj.Attach(new CardInfo()); return obj.Attach(new ApplyCardStats()); }
}
