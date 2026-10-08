// Canna MIT. Additional mechanical mappings checked against the public game/core.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using Mono.Cecil;
using Mono.Cecil.Cil;

static class CannaFixRules
{
    public static List<string> Apply(ModuleDefinition module, Game game)
    {
        var changes = new List<string>();
        var harmony = game.Resolver.Get("0Harmony")?.MainModule.GetType("HarmonyLib.Harmony");
        var constructor = harmony?.Methods.SingleOrDefault(m => m.IsConstructor && !m.IsStatic
            && m.Parameters.Count == 1 && m.Parameters[0].ParameterType.FullName == "System.String");
        var unpatchSelf = harmony?.Methods.SingleOrDefault(m => m.Name == "UnpatchSelf" && !m.IsStatic
            && m.Parameters.Count == 0 && m.ReturnType.FullName == "System.Void");
        bool hasUnpatchId = harmony != null && harmony.Methods.Any(m => m.Name == "UnpatchID" && m.IsStatic
            && m.Parameters.Count == 1 && m.Parameters[0].ParameterType.FullName == "System.String"
            && m.ReturnType.FullName == "System.Void");
        foreach (var method in Scanner.AllTypes(module).SelectMany(t => t.Methods).Where(m => m.HasBody))
            foreach (var instruction in method.Body.Instructions.ToArray())
            {
                if (instruction.Operand is not MethodReference target || target.DeclaringType.FullName != "HarmonyLib.Harmony"
                    || target.Name != "UnpatchID" || target.DeclaringType.Scope is not AssemblyNameReference scope
                    || scope.Name != "0Harmony" || hasUnpatchId) continue;
                if (instruction.OpCode != OpCodes.Call || target.HasThis || target.Parameters.Count != 1
                    || target.Parameters[0].ParameterType.FullName != "System.String" || target.ReturnType.FullName != "System.Void"
                    || constructor == null || unpatchSelf == null)
                    throw new InvalidDataException("Unsupported Harmony.UnpatchID call/core contract: " + method.FullName);
                // Both operations remove patches by owner ID. Creating a Harmony
                // handle does not apply patches; UnpatchSelf scopes removal to it.
                instruction.OpCode = OpCodes.Newobj;
                instruction.Operand = module.ImportReference(constructor);
                var il = method.Body.GetILProcessor();
                il.InsertAfter(instruction, il.Create(OpCodes.Callvirt, module.ImportReference(unpatchSelf)));
                changes.Add("Harmony.UnpatchID(id) -> new Harmony(id).UnpatchSelf()");
            }
        var core = game.Resolver.Get("UnityEngine.CoreModule")?.MainModule;
        var pipeline = core?.GetType("UnityEngine.Rendering.RenderPipelineAsset");
        foreach (var type in module.GetTypeReferences())
        {
            if (type.FullName != "UnityEngine.Experimental.Rendering.RenderPipelineAsset") continue;
            if (pipeline == null || type.Scope is not AssemblyNameReference scope
                || scope.Name != "UnityEngine.CoreModule")
                throw new InvalidDataException("Unsupported legacy render pipeline type scope");
            type.Namespace = pipeline.Namespace;
            changes.Add("Experimental.Rendering.RenderPipelineAsset -> Rendering.RenderPipelineAsset");
        }
        return changes.Distinct().ToList();
    }
}
