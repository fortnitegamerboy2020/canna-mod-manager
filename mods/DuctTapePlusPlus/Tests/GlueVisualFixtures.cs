// Reads actual pinned CR IL as metadata only. No CR, Unity or game runtime executes.
using System.IO.Compression;
using System.Security.Cryptography;
using Mono.Cecil;
using Mono.Cecil.Cil;

static class GlueVisualFixtures
{
    static MethodDefinition Getter(ModuleDefinition module) => module.GetType("CR.MonoBehaviors.GlueMono").Methods.Single(m => m.Name == "get_glueVisual");
    static string Shape(MethodDefinition method) => string.Join("\n", method.Body.Instructions.Select(i => i.OpCode.Code + ":" +
        (i.Operand is Instruction branch ? method.Body.Instructions.IndexOf(branch).ToString()
            : i.Operand is MemberReference member ? member.FullName : i.Operand?.ToString() ?? "")));
    static bool Cleanup(Instruction instruction) => instruction.Operand is MethodReference method
        && method.DeclaringType.FullName == "UnityEngine.Object" && method.Name.StartsWith("Destroy", StringComparison.Ordinal);
    public static void Run(string root, string cached, Game game, Action<bool, string> check)
    {
        using var zip = ZipFile.OpenRead(Path.Combine(root, "target/rebound-local-pick-20261008/build-final-bound-bars/downloads/XAngelMoonX-CR-2.7.0.zip"));
        var entry = zip.Entries.Single(e => e.Name == "CosmicRounds.dll");
        using var bytes = new MemoryStream(); using (var stream = entry.Open()) stream.CopyTo(bytes);
        byte[] source = bytes.ToArray();
        check(Convert.ToHexString(SHA256.HashData(source)).ToLowerInvariant() == "79cc4398a08cddb6a2283a5e1ec100cb7de13200fecbb3c5327b3c8299f8435b",
            "Glue fixture reads exact pinned original CR2.7.0 bytes from the cached package");
        string corePath = Path.Combine(cached, "ROUNDS_Data/Managed/UnityEngine.CoreModule.dll");
        using var core = ModuleDefinition.ReadModule(corePath); game.Resolver.Add(core);
        using var module = ModuleDefinition.ReadModule(new MemoryStream(source));
        var getter = Getter(module);
        var original = getter.Body.Instructions.Select(i => i.ToString()).ToArray();
        bool Unrelated(MethodDefinition m) => m.HasBody && m != getter
            && !(m.DeclaringType.FullName == "CR.MonoBehaviors.SatelliteMono" && m.Name == "Start");
        var unrelated = Scanner.AllTypes(module).SelectMany(t => t.Methods).Where(Unrelated)
            .Select(m => m.FullName + "\n" + Shape(m)).ToArray();
        check(getter.Body.Instructions.Count(Cleanup) == 6 && getter.Body.Instructions.Count(i => i.Operand is MethodReference m
            && m.DeclaringType.FullName == "UnityEngine.Object" && m.Name == "Instantiate") == 1,
            "Actual Glue getter has six removals and one runtime visual instantiation");
        var result = CannaFixRules.Apply(module, game);
        check(result.Count(c => c.StartsWith("CosmicRounds Glue:", StringComparison.Ordinal)) == 2,
            "Actual Glue rule synchronizes exactly the two intentionally removed Explosion components");
        check(getter.Body.Instructions.Count == original.Length && getter.Body.Instructions.Select((i, at) => i.ToString() != original[at]).Count(c => c) == 2,
            "Exact original Glue getter changes only two call operands; instruction and branch identities stay intact");
        var immediate = getter.Body.Instructions.Where(i => i.Operand is MethodReference m && m.Name == "DestroyImmediate").ToArray();
        check(immediate.Length == 2 && immediate.Select(i => ((GenericInstanceMethod)i.Previous.Operand).GenericArguments[0].FullName)
            .SequenceEqual(new[] { "Explosion", "Explosion_Overpower" }),
            "Only Explosion and Explosion_Overpower use synchronous cleanup");
        check(immediate.All(i => i.Operand is MethodReference m && !m.HasThis && m.Parameters.Count == 1
            && m.Parameters[0].ParameterType.FullName == "UnityEngine.Object" && m.DeclaringType.Scope.Name == "UnityEngine.CoreModule"),
            "Cleanup imports the inspected native one-argument overload that forbids destroying assets");
        check(getter.Body.Instructions.Count(i => i.Operand is MethodReference m && m.Name == "Destroy") == 4,
            "Other original component and child-object Destroy calls retain deferred semantics");
        check(unrelated.SequenceEqual(Scanner.AllTypes(module).SelectMany(t => t.Methods).Where(Unrelated)
            .Select(m => m.FullName + "\n" + Shape(m))), "Every CR method outside the two reviewed fixes retains its original instruction semantics");
        check(CannaFixRules.Apply(module, game).Count == 0, "Glue normalization is idempotent");
        using (var encoded = new MemoryStream())
        {
            module.Write(encoded); encoded.Position = 0; using var read = ModuleDefinition.ReadModule(encoded);
            check(CannaFixRules.Apply(read, game).Count == 0, "Serialized Glue cleanup remains verified and idempotent");
        }
        void Refuses(Action<ModuleDefinition> tamper, string label)
        {
            using var wrong = ModuleDefinition.ReadModule(new MemoryStream(source)); tamper(wrong);
            string before = Shape(Getter(wrong)); bool refused = false;
            try { CannaFixRules.Apply(wrong, game); } catch (InvalidDataException) { refused = true; }
            check(refused && Shape(Getter(wrong)) == before, label + " is refused before any cleanup mutation");
        }
        Refuses(m => Getter(m).Body.Instructions.Single(i => Equals(i.Operand, "E_Pulsar")).Operand = "UnknownClone", "Changed runtime-clone creation shape");
        Refuses(m => Getter(m).Body.Instructions.First(i => i.Operand is GenericInstanceMethod g && g.Name == "GetComponent"
            && g.GenericArguments[0].FullName == "Explosion_Overpower").Operand = Getter(m).Body.Instructions.First(i => i.Operand is GenericInstanceMethod g
                && g.Name == "GetComponent" && g.GenericArguments[0].FullName == "Explosion").Operand, "Duplicate Explosion selector");
        Refuses(m => Getter(m).IsPublic = false, "Unexpected getter visibility");
        Refuses(m => Getter(m).Body.Instructions.First(i => i.Operand is Instruction).Operand = Getter(m).Body.Instructions.Last(), "Changed branch destination");
        Refuses(m =>
        {
            var call = Getter(m).Body.Instructions.First(i => i.Operand is MethodReference mr && mr.Name == "Destroy");
            call.OpCode = OpCodes.Callvirt;
        }, "Nonmatching unrelated removal opcode");
        Refuses(m =>
        {
            var plugin = m.GetType("CR.CR").CustomAttributes.Single(a => a.AttributeType.FullName == "BepInEx.BepInPlugin");
            plugin.ConstructorArguments[2] = new CustomAttributeArgument(m.TypeSystem.String, "2.8.0");
        }, "Unreviewed CR version");
        using (var partial = ModuleDefinition.ReadModule(new MemoryStream(source)))
        {
            var first = Getter(partial).Body.Instructions.First(i => i.Previous?.Operand is GenericInstanceMethod g
                && g.Name == "GetComponent" && g.GenericArguments[0].FullName == "Explosion");
            first.Operand = partial.ImportReference(core.GetType("UnityEngine.Object").Methods.Single(m => m.Name == "DestroyImmediate" && m.Parameters.Count == 1));
            check(CannaFixRules.Apply(partial, game).Count(c => c.StartsWith("CosmicRounds Glue:", StringComparison.Ordinal)) == 1,
                "A validated partial two-component cleanup normalizes only its remaining call");
        }
        using (var alternate = ModuleDefinition.ReadModule(new MemoryStream(source)))
        {
            alternate.Assembly.Name.Name = "UnrelatedMod";
            string before = Shape(Getter(alternate)); CannaFixRules.Apply(alternate, game);
            check(Shape(Getter(alternate)) == before, "A different assembly with the same getter name is never rewritten");
        }
        using (var wrongCore = ModuleDefinition.ReadModule(corePath))
        using (var fresh = ModuleDefinition.ReadModule(new MemoryStream(source)))
        {
            var destroy = wrongCore.GetType("UnityEngine.Object").Methods.Single(m => m.Name == "DestroyImmediate" && m.Parameters.Count == 1);
            destroy.Body.Instructions.First(i => i.OpCode == OpCodes.Ldc_I4_0).OpCode = OpCodes.Ldc_I4_1;
            var wrongGame = new Game(game.AssemblyCSharp, game.Resolver.Get("0Harmony").MainModule); wrongGame.Resolver.Add(wrongCore);
            string before = Shape(Getter(fresh)); bool refused = false;
            try { CannaFixRules.Apply(fresh, wrongGame); } catch (InvalidDataException) { refused = true; }
            check(refused && Shape(Getter(fresh)) == before, "Native asset-destroy permission change is refused without rewriting the mod");
        }
    }
}
