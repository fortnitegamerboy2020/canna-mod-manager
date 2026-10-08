// Actual pinned Satellite Start IL executes only against these harmless lifecycle APIs.
using System.IO.Compression;
using System.Reflection;
using System.Reflection.Emit;
using System.Security.Cryptography;
using HarmonyLib;
using Mono.Cecil;
using Mono.Cecil.Cil;

static class SatelliteLifecycleFixtures
{
    static MethodDefinition Start(ModuleDefinition module) => module.GetType("CR.MonoBehaviors.SatelliteMono").Methods.Single(m => m.Name == "Start");
    static string Shape(MethodDefinition method, int skip = 0)
    {
        var code = method.Body.Instructions;
        return string.Join("\n", code.Skip(skip).Select(i => i.OpCode.Code + ":" + (i.Operand is Instruction branch
            ? (code.IndexOf(branch) - skip).ToString() : i.Operand is MemberReference member ? member.FullName : i.Operand?.ToString() ?? "")));
    }
    public static void Run(string root, Game game, Action<bool, string> check)
    {
        using var zip = ZipFile.OpenRead(Path.Combine(root, "target/rebound-local-pick-20261008/build-final-bound-bars/downloads/XAngelMoonX-CR-2.7.0.zip"));
        using var bytes = new MemoryStream(); using (var stream = zip.Entries.Single(e => e.Name == "CosmicRounds.dll").Open()) stream.CopyTo(bytes);
        byte[] source = bytes.ToArray();
        check(Convert.ToHexString(SHA256.HashData(source)).ToLowerInvariant() == "79cc4398a08cddb6a2283a5e1ec100cb7de13200fecbb3c5327b3c8299f8435b",
            "Satellite lifecycle fixture reads exact original CR2.7.0 bytes");
        using var module = ModuleDefinition.ReadModule(new MemoryStream(source));
        var start = Start(module); string original = Shape(start);
        var other = Scanner.AllTypes(module).SelectMany(t => t.Methods).Where(m => m.HasBody && m != start
            && !(m.DeclaringType.FullName == "CR.MonoBehaviors.GlueMono" && m.Name == "get_glueVisual"))
            .Select(m => m.FullName + "\n" + Shape(m)).ToArray();
        var originalBody = Compile(start);
        var prototype = New(null); bool originalFailed = false;
        try { originalBody(prototype); } catch (NullReferenceException) { originalFailed = true; }
        check(originalFailed, "Actual original Start reproduces the unparented prototype null dereference against harmless APIs");
        check(CannaFixRules.Apply(module, game).Count(c => c.StartsWith("CosmicRounds Satellite:", StringComparison.Ordinal)) == 1,
            "Pinned Satellite normalization adds exactly one lifecycle gate");
        check(start.Body.Instructions.Count == 35 && Shape(start, 8) == original,
            "All 27 original Satellite Start instructions remain intact after the eight-instruction parent gate");
        check(other.SequenceEqual(Scanner.AllTypes(module).SelectMany(t => t.Methods).Where(m => m.HasBody && m != start
            && !(m.DeclaringType.FullName == "CR.MonoBehaviors.GlueMono" && m.Name == "get_glueVisual"))
            .Select(m => m.FullName + "\n" + Shape(m))), "Satellite preserves Update, card callbacks and every method outside the two reviewed fixes");
        var normalizedBody = Compile(start); prototype = New(null); normalizedBody(prototype);
        check(prototype.move is null && prototype.sync is null && prototype.TimerCalls == 0,
            "Actual normalized Start leaves the unparented template idle without fabricating motion or synchronization");
        var parent = new SatelliteFixtureTransform(); var clone = New(parent);
        var move = new SatelliteFixtureMove { velocity = new SatelliteFixtureVector { x = 3, y = -2 } };
        var sync = new SatelliteFixtureSync(); clone.Ancestors[typeof(SatelliteFixtureMove)] = move; clone.Ancestors[typeof(SatelliteFixtureSync)] = sync;
        normalizedBody(clone);
        check(ReferenceEquals(clone.move, move) && ReferenceEquals(clone.sync, sync) && sync.active && clone.TimerCalls == 1
            && clone.xvel == 3 && clone.yvel == -2 && move.velocity.x == 3 && move.velocity.y == -2,
            "Actual attached Start preserves native parent identity, sync activation, timer and initial velocity capture");
        bool missingFailed = false;
        try { normalizedBody(New(parent)); } catch (NullReferenceException) { missingFailed = true; }
        check(missingFailed, "A malformed attached projectile still fails originally; no broad exception or missing-parent fallback is added");
        check(CannaFixRules.Apply(module, game).Count == 0, "Satellite lifecycle normalization is idempotent");
        using (var encoded = new MemoryStream())
        {
            module.Write(encoded); encoded.Position = 0; using var read = ModuleDefinition.ReadModule(encoded);
            check(CannaFixRules.Apply(read, game).Count == 0, "Serialized Satellite parent gate remains verified and idempotent");
        }
        void Refuses(Action<ModuleDefinition> tamper, string label, bool normalized = false)
        {
            using var wrong = ModuleDefinition.ReadModule(new MemoryStream(source));
            if (normalized) CannaFixRules.Apply(wrong, game);
            tamper(wrong); string before = Shape(Start(wrong)); bool refused = false;
            try { CannaFixRules.Apply(wrong, game); } catch (InvalidDataException) { refused = true; }
            check(refused && Shape(Start(wrong)) == before, label + " is refused without mutating Satellite Start");
        }
        Refuses(m => m.GetType("CR.Cards.SatelliteCard").Methods.Single(x => x.Name == "OnAddCard").Body.Instructions
            .Single(i => Equals(i.Operand, "A_Satellite")).Operand = "UnknownPrototype", "Changed prototype creation shape");
        Refuses(m => m.GetType("CR.MonoBehaviors.SatelliteMono").Methods.Single(x => x.Name == "Update").Body.Instructions
            .First(i => i.Operand is Instruction).Operand = m.GetType("CR.MonoBehaviors.SatelliteMono").Methods.Single(x => x.Name == "Update").Body.Instructions[0],
            "Changed existing Update gate");
        Refuses(m => Start(m).IsPublic = true, "Changed Start visibility");
        Refuses(m => Start(m).Body.Instructions.Single(i => i.Operand is FieldReference f && f.Name == "active").OpCode = Mono.Cecil.Cil.OpCodes.Ldfld,
            "Changed synchronization assignment");
        Refuses(m => Start(m).Body.Instructions[6].Operand = Start(m).Body.Instructions[9], "Tampered normalized branch", true);
        Refuses(m => ((MethodReference)Start(m).Body.Instructions[1].Operand).DeclaringType.Scope = new AssemblyNameReference("UnknownUnity", new Version(1, 0)),
            "Unknown normalized Unity scope", true);
        using (var different = ModuleDefinition.ReadModule(new MemoryStream(source)))
        {
            different.Assembly.Name.Name = "OtherMod"; string before = Shape(Start(different)); CannaFixRules.Apply(different, game);
            check(Shape(Start(different)) == before, "Unrelated assemblies with a Satellite type are never normalized");
        }
    }

    static SatelliteFixtureMono New(SatelliteFixtureTransform parent) => new() {
        gameObject = new SatelliteFixtureObject { transform = new SatelliteFixtureTransform { parent = parent } }
    };
    static Type MapType(TypeReference type) => type.FullName switch {
        "CR.MonoBehaviors.SatelliteMono" => typeof(SatelliteFixtureMono), "UnityEngine.Component" => typeof(SatelliteFixtureComponent),
        "UnityEngine.GameObject" => typeof(SatelliteFixtureObject), "UnityEngine.Transform" => typeof(SatelliteFixtureTransform),
        "UnityEngine.Object" => typeof(SatelliteFixtureObjectBase), "MoveTransform" => typeof(SatelliteFixtureMove),
        "Photon.Pun.SyncProjectile" => typeof(SatelliteFixtureSync), "UnityEngine.Vector3" => typeof(SatelliteFixtureVector),
        _ => Type.GetType(type.FullName) ?? throw new Exception("Unknown Satellite fixture type " + type.FullName)
    };
    static Action<SatelliteFixtureMono> Compile(MethodDefinition source)
    {
        var dynamic = new DynamicMethod("ActualSatelliteStartFixture", typeof(void), new[] { typeof(SatelliteFixtureMono) }, typeof(SatelliteLifecycleFixtures).Module, true);
        var il = dynamic.GetILGenerator();
        var branches = source.Body.Instructions.Where(i => i.Operand is Instruction).Select(i => (Instruction)i.Operand).Distinct().ToDictionary(i => i, _ => il.DefineLabel());
        var opcodes = typeof(System.Reflection.Emit.OpCodes).GetFields(BindingFlags.Public | BindingFlags.Static)
            .Where(f => f.FieldType == typeof(System.Reflection.Emit.OpCode)).Select(f => (System.Reflection.Emit.OpCode)f.GetValue(null)).ToDictionary(o => o.Name);
        const BindingFlags flags = BindingFlags.Instance | BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic;
        foreach (var instruction in source.Body.Instructions)
        {
            if (branches.TryGetValue(instruction, out var label)) il.MarkLabel(label);
            object operand = instruction.Operand;
            if (operand is Instruction target) operand = branches[target];
            else if (operand is FieldReference field) operand = MapType(field.DeclaringType).GetField(field.Name, flags) ?? throw new Exception("Missing harmless field");
            else if (operand is GenericInstanceMethod generic)
                operand = MapType(generic.DeclaringType).GetMethods(flags).Single(m => m.Name == generic.Name && m.IsGenericMethodDefinition)
                    .MakeGenericMethod(generic.GenericArguments.Select(MapType).ToArray());
            else if (operand is MethodReference method)
                operand = MapType(method.DeclaringType).GetMethod(method.Name, flags, null, method.Parameters.Select(p => MapType(p.ParameterType)).ToArray(), null)
                    ?? throw new Exception("Missing harmless method " + method.FullName);
            NativeCardPickerFixtures.Emit(il, new CodeInstruction(opcodes[instruction.OpCode.Name], operand));
        }
        return dynamic.CreateDelegate<Action<SatelliteFixtureMono>>();
    }
}
class SatelliteFixtureObjectBase
{
    public static bool operator ==(SatelliteFixtureObjectBase left, SatelliteFixtureObjectBase right) => ReferenceEquals(left, right);
    public static bool operator !=(SatelliteFixtureObjectBase left, SatelliteFixtureObjectBase right) => !ReferenceEquals(left, right);
    public override bool Equals(object obj) => ReferenceEquals(this, obj);
    public override int GetHashCode() => System.Runtime.CompilerServices.RuntimeHelpers.GetHashCode(this);
}
sealed class SatelliteFixtureObject : SatelliteFixtureObjectBase { public SatelliteFixtureTransform transform { get; set; } }
sealed class SatelliteFixtureTransform : SatelliteFixtureObjectBase { public SatelliteFixtureTransform parent { get; set; } }
class SatelliteFixtureComponent
{
    public SatelliteFixtureObject gameObject { get; set; }
    public readonly Dictionary<Type, object> Ancestors = new();
    public T GetComponentInParent<T>() where T : class => Ancestors.GetValueOrDefault(typeof(T)) as T;
}
sealed class SatelliteFixtureMono : SatelliteFixtureComponent
{
    public SatelliteFixtureMove move = null; public SatelliteFixtureSync sync = null; public float xvel = 0f, yvel = 0f;
    public int TimerCalls;
    public void ResetTimer() { TimerCalls++; }
}
sealed class SatelliteFixtureMove { public SatelliteFixtureVector velocity; }
sealed class SatelliteFixtureSync { public bool active = false; }
struct SatelliteFixtureVector { public float x, y; }
