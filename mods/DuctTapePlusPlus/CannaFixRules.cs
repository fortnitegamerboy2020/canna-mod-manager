// Canna MIT. Additional mechanical mappings checked against the public game/core.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using Mono.Cecil;
using Mono.Cecil.Cil;

static class CannaFixRules
{
    const string PickerHelperName = "Canna_GetPickerByPlayerID";
    public static List<string> Apply(ModuleDefinition module, Game game)
    {
        var changes = new List<string>();
        UniqueCardPlayerId(module, game, changes);
        ValidateCardBarBoundsGuard(module);
        ProjectileDynamicTargets(module, game, changes);
        CardBarRosterSlots(module, game, changes);
        GlueVisualCleanup(module, game, changes);
        SatellitePrototypeLifecycle(module, changes);
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

    static bool GameType(TypeReference type, string name) => type.FullName == name
        && type.Scope is AssemblyNameReference scope && scope.Name == "Assembly-CSharp";

    static void GlueVisualCleanup(ModuleDefinition module, Game game, List<string> changes)
    {
        if (module.Assembly.Name.Name != "CosmicRounds") return;
        const string error = "Unsupported CosmicRounds Glue runtime visual cleanup contract";
        var plugin = module.GetType("CR.CR")?.CustomAttributes.Where(a => a.AttributeType.FullName == "BepInEx.BepInPlugin").ToArray();
        if (plugin == null || plugin.Length != 1 || plugin[0].ConstructorArguments.Count != 3
            || plugin[0].ConstructorArguments[0].Value is not string guid || guid != "com.XAngelMoonX.rounds.CosmicRounds"
            || plugin[0].ConstructorArguments[2].Value is not string version || version != "2.7.0")
            throw new InvalidDataException(error);
        var type = module.GetType("CR.MonoBehaviors.GlueMono");
        var getters = type?.Methods.Where(m => m.Name == "get_glueVisual").ToArray();
        if (type?.BaseType?.FullName != "ModdingUtils.RoundsEffects.HitSurfaceEffect"
            || getters == null || getters.Length != 1) throw new InvalidDataException(error);
        var method = getters[0];
        if (!method.IsPublic || !method.IsStatic || !method.IsSpecialName || method.GenericParameters.Count != 0
            || method.Parameters.Count != 0 || method.ReturnType.FullName != "UnityEngine.GameObject"
            || !method.HasBody || !method.Body.InitLocals || method.Body.ExceptionHandlers.Count != 0
            || method.Body.Instructions.Count != 146 || method.Body.Variables.Count != 4)
            throw new InvalidDataException(error);
        var code = method.Body.Instructions;
        var cleanups = new List<Instruction>();
        foreach (string component in new[] { "Explosion", "Explosion_Overpower" })
        {
            var matches = code.Where(i => i.OpCode == OpCodes.Callvirt && i.Operand is GenericInstanceMethod get
                && get.Name == "GetComponent" && get.HasThis && get.Parameters.Count == 0
                && get.DeclaringType.FullName == "UnityEngine.GameObject" && get.GenericArguments.Count == 1
                && GameType(get.GenericArguments[0], component)).ToArray();
            if (matches.Length != 1) throw new InvalidDataException(error);
            int at = code.IndexOf(matches[0]);
            if (at < 1 || at + 1 >= code.Count || code[at - 1].OpCode != OpCodes.Ldsfld
                || code[at - 1].Operand is not FieldReference visual || visual.Name != "glueVisu"
                || visual.DeclaringType.FullName != type.FullName || visual.DeclaringType.Scope != module
                || visual.FieldType.FullName != "UnityEngine.GameObject" || code[at + 1].OpCode != OpCodes.Call
                || code[at + 1].Operand is not MethodReference destroy || destroy.HasThis || destroy.Parameters.Count != 1
                || destroy.Parameters[0].ParameterType.FullName != "UnityEngine.Object" || destroy.ReturnType.FullName != "System.Void"
                || destroy.DeclaringType.FullName != "UnityEngine.Object" || destroy.Name is not ("Destroy" or "DestroyImmediate")
                || destroy.DeclaringType.Scope.Name is not ("UnityEngine" or "UnityEngine.CoreModule"))
                throw new InvalidDataException(error);
            cleanups.Add(code[at + 1]);
        }
        // The complete pinned getter proves glueVisu is a newly instantiated runtime clone.
        // Canonicalize only the two reviewed calls, so a partial normalization remains retryable.
        var shape = code.Select(i => i.OpCode.Code + ":" + (i.Operand is Instruction branch
            ? code.IndexOf(branch).ToString(System.Globalization.CultureInfo.InvariantCulture)
            : i.Operand is MemberReference member
                ? cleanups.Contains(i) ? member.FullName.Replace("::DestroyImmediate(", "::Destroy(") : member.FullName
                : Convert.ToString(i.Operand, System.Globalization.CultureInfo.InvariantCulture) ?? ""));
        using (var sha = System.Security.Cryptography.SHA256.Create())
        {
            var digest = BitConverter.ToString(sha.ComputeHash(System.Text.Encoding.UTF8.GetBytes(string.Join("\n", shape))))
                .Replace("-", "").ToLowerInvariant();
            if (digest != "4521e34163f0b689c9b9961c6f198d2bc5e9f9c78bd71eb267526b3f7fcb431c")
                throw new InvalidDataException(error);
        }
        var destroyers = game.Resolver.Get("UnityEngine.CoreModule")?.MainModule.GetType("UnityEngine.Object")?.Methods
            .Where(m => m.Name == "DestroyImmediate" && m.IsPublic && m.IsStatic && m.ReturnType.FullName == "System.Void"
                && m.Parameters.Count == 1 && m.Parameters[0].ParameterType.FullName == "UnityEngine.Object").ToArray();
        if (destroyers == null || destroyers.Length != 1 || !destroyers[0].HasBody)
            throw new InvalidDataException(error);
        // The one-argument native overload forwards allowDestroyingAssets=false.
        var native = destroyers[0].Body.Instructions;
        if (!native.Select(i => i.OpCode.Code).SequenceEqual(new[] { Code.Nop, Code.Ldc_I4_0, Code.Stloc_0,
                Code.Ldarg_0, Code.Ldloc_0, Code.Call, Code.Nop, Code.Ret })
            || native[5].Operand is not MethodReference forward || forward.Name != "DestroyImmediate" || forward.HasThis
            || forward.DeclaringType.FullName != "UnityEngine.Object" || forward.Parameters.Count != 2
            || forward.Parameters[0].ParameterType.FullName != "UnityEngine.Object"
            || forward.Parameters[1].ParameterType.FullName != "System.Boolean") throw new InvalidDataException(error);
        foreach (var cleanup in cleanups)
        {
            if (((MethodReference)cleanup.Operand).Name == "DestroyImmediate") continue;
            cleanup.Operand = module.ImportReference(destroyers[0]);
            changes.Add("CosmicRounds Glue: synchronously remove "
                + ((GenericInstanceMethod)cleanup.Previous.Operand).GenericArguments[0].FullName
                + " from the runtime visual clone before its first copy (assets unchanged)");
        }
    }

    static bool GameField(Instruction instruction, OpCode code, string type, string name, string fieldType) =>
        instruction.OpCode == code && instruction.Operand is FieldReference field
        && GameType(field.DeclaringType, type) && field.Name == name && field.FieldType.FullName == fieldType;

    static string BodyDigest(MethodDefinition method, int skip = 0)
    {
        var code = method.Body.Instructions;
        var shape = code.Skip(skip).Select(i => i.OpCode.Code + ":" + (i.Operand is Instruction branch
            ? (code.IndexOf(branch) - skip).ToString(System.Globalization.CultureInfo.InvariantCulture)
            : i.Operand is MemberReference member ? member.FullName
            : Convert.ToString(i.Operand, System.Globalization.CultureInfo.InvariantCulture) ?? ""));
        using var sha = System.Security.Cryptography.SHA256.Create();
        return BitConverter.ToString(sha.ComputeHash(System.Text.Encoding.UTF8.GetBytes(string.Join("\n", shape))))
            .Replace("-", "").ToLowerInvariant();
    }

    static void SatellitePrototypeLifecycle(ModuleDefinition module, List<string> changes)
    {
        if (module.Assembly.Name.Name != "CosmicRounds") return;
        const string error = "Unsupported CosmicRounds Satellite prototype lifecycle contract";
        var type = module.GetType("CR.MonoBehaviors.SatelliteMono");
        var starts = type?.Methods.Where(m => m.Name == "Start").ToArray();
        var updates = type?.Methods.Where(m => m.Name == "Update").ToArray();
        var adds = module.GetType("CR.Cards.SatelliteCard")?.Methods.Where(m => m.Name == "OnAddCard").ToArray();
        if (type?.BaseType?.FullName != "UnityEngine.MonoBehaviour" || starts == null || starts.Length != 1
            || updates == null || updates.Length != 1 || adds == null || adds.Length != 1)
            throw new InvalidDataException(error);
        var start = starts[0]; var update = updates[0]; var add = adds[0];
        foreach (var method in new[] { start, update })
            if (method.IsStatic || method.IsPublic || method.Parameters.Count != 0 || method.GenericParameters.Count != 0
                || method.ReturnType.FullName != "System.Void" || !method.HasBody || method.Body.ExceptionHandlers.Count != 0
                || method.Body.Variables.Count != 0) throw new InvalidDataException(error);
        // The pinned card creates an active, unparented runtime prototype. Native Gun
        // clones it under the real bullet; deactivating the prototype would deactivate
        // those copies. Update already excludes the unparented prototype explicitly.
        if (!add.IsPublic || add.IsStatic || add.Parameters.Count != 8 || !add.HasBody
            || add.Body.Instructions.Count != 53 || add.Body.Variables.Count != 3 || add.Body.ExceptionHandlers.Count != 0
            || update.Body.Instructions.Count != 104
            || BodyDigest(add) != "1ea3c2ba6bfaf4e2b994602ae6b22c65a8a3124c3b9dfe23f324d082d0b18827"
            || BodyDigest(update) != "db8eb4394a5267645ce7efcf435d65b977e8b1113fc3547b5a4b250a8fcc636a")
            throw new InvalidDataException(error);
        var code = start.Body.Instructions;
        bool normalized = code.Count == 35;
        if ((!normalized && code.Count != 27) || BodyDigest(start, normalized ? 8 : 0)
            != "74d092864df07b8cb730d88925303d33162e7e12914ec48ad05dea4add2ee09f")
            throw new InvalidDataException(error);
        var gate = update.Body.Instructions.Skip(7).Take(6).ToArray();
        if (!gate.Select(i => i.OpCode.Code).SequenceEqual(new[] { Code.Ldarg_0, Code.Call, Code.Callvirt,
                Code.Callvirt, Code.Ldnull, Code.Call })
            || gate[1].Operand is not MethodReference gameObject || gameObject.FullName != "UnityEngine.GameObject UnityEngine.Component::get_gameObject()"
            || gate[2].Operand is not MethodReference transform || transform.FullName != "UnityEngine.Transform UnityEngine.GameObject::get_transform()"
            || gate[3].Operand is not MethodReference parent || parent.FullName != "UnityEngine.Transform UnityEngine.Transform::get_parent()"
            || gate[5].Operand is not MethodReference inequality || inequality.FullName != "System.Boolean UnityEngine.Object::op_Inequality(UnityEngine.Object,UnityEngine.Object)"
            || new[] { gameObject, transform, parent, inequality }.Any(m => m.DeclaringType.Scope.Name is not ("UnityEngine" or "UnityEngine.CoreModule")))
            throw new InvalidDataException(error);
        if (normalized)
        {
            for (int i = 0; i < gate.Length; i++)
                if (code[i].OpCode != gate[i].OpCode || (code[i].Operand is MemberReference member
                    ? gate[i].Operand is not MemberReference expected || member.FullName != expected.FullName
                        || member.DeclaringType.Scope.Name != expected.DeclaringType.Scope.Name
                    : !Equals(code[i].Operand, gate[i].Operand))) throw new InvalidDataException(error);
            if (code[6].OpCode != OpCodes.Brtrue || code[6].Operand != code[8] || code[7].OpCode != OpCodes.Ret)
                throw new InvalidDataException(error);
            return;
        }
        var il = start.Body.GetILProcessor(); var original = code[0];
        foreach (var instruction in gate)
            il.InsertBefore(original, instruction.Operand is MethodReference method
                ? il.Create(instruction.OpCode, method) : il.Create(instruction.OpCode));
        il.InsertBefore(original, il.Create(OpCodes.Brtrue, original));
        il.InsertBefore(original, il.Create(OpCodes.Ret));
        start.Body.MaxStackSize = Math.Max(start.Body.MaxStackSize, 2);
        changes.Add("CosmicRounds Satellite: apply existing parent gate to prototype Start; attached projectile lifecycle unchanged");
    }

    static bool PlayerList(TypeReference type) => type is GenericInstanceType list
        && list.ElementType.FullName == "System.Collections.Generic.List`1" && list.GenericArguments.Count == 1
        && GameType(list.GenericArguments[0], "Player");

    static bool PickerArgument(Instruction instruction, MethodDefinition method) => instruction.OpCode == OpCodes.Ldarg_1
        || ((instruction.OpCode == OpCodes.Ldarg || instruction.OpCode == OpCodes.Ldarg_S)
            && instruction.Operand == method.Parameters[1]);

    static void UniqueCardPlayerId(ModuleDefinition module, Game game, List<string> changes)
    {
        if (module.Assembly.Name.Name != "CardChoiceSpawnUniqueCardPatch") return;
        const string error = "Unsupported CardChoiceSpawnUniqueCardPatch player-ID lookup contract";
        var type = module.GetType("CardChoiceSpawnUniqueCardPatch.CardChoicePatchSpawnUniqueCard");
        var patches = type?.CustomAttributes.Where(a => a.AttributeType.FullName == "HarmonyLib.HarmonyPatch").ToArray();
        var prefixes = type?.Methods.Where(m => m.Name == "Prefix").ToArray();
        if (patches == null || patches.Length != 1 || patches[0].AttributeType.Scope.Name != "0Harmony"
            || patches[0].Fields.Count != 0 || patches[0].Properties.Count != 0
            || patches[0].ConstructorArguments.Count != 2 || patches[0].Constructor.Parameters.Count != 2
            || patches[0].ConstructorArguments[0].Value is not TypeReference cardChoice || !GameType(cardChoice, "CardChoice")
            || patches[0].ConstructorArguments[1].Value is not string target || target != "SpawnUniqueCard"
            || prefixes == null || prefixes.Length != 1) throw new InvalidDataException(error);
        var method = prefixes[0];
        if (!method.IsStatic || !method.HasBody || method.ReturnType.FullName != "System.Boolean"
            || method.Parameters.Count != 4 || method.Parameters[0].ParameterType.FullName != "UnityEngine.GameObject&"
            || !GameType(method.Parameters[1].ParameterType, "CardChoice")
            || method.Parameters[2].ParameterType.FullName != "UnityEngine.Vector3"
            || method.Parameters[3].ParameterType.FullName != "UnityEngine.Quaternion"
            || method.Body.ExceptionHandlers.Count != 0 || method.Body.Instructions.Count > 4096)
            throw new InvalidDataException(error);
        var playerManager = game.AssemblyCSharp.GetType("PlayerManager");
        var resolvers = playerManager?.Methods.Where(m => m.Name == "GetPlayerWithID" && !m.IsStatic
            && m.Parameters.Count == 1 && m.Parameters[0].ParameterType.FullName == "System.Int32"
            && m.ReturnType.FullName == "Player").ToArray();
        if (resolvers == null || resolvers.Length != 1) throw new InvalidDataException(error);
        var instructions = method.Body.Instructions;
        var listCalls = instructions.Where(i => i.OpCode == OpCodes.Callvirt && i.Operand is MethodReference m
            && m.Name == "get_Item" && PlayerList(m.DeclaringType)).ToArray();
        var idCalls = instructions.Where(i => (i.OpCode == OpCodes.Callvirt && i.Operand is MethodReference m
                && GameType(m.DeclaringType, "PlayerManager") && m.Name == "GetPlayerWithID")
            || (i.OpCode == OpCodes.Call && i.Operand is MethodReference helper
                && helper.DeclaringType.FullName == type!.FullName && helper.Name == PickerHelperName)).ToArray();
        if (listCalls.Length + idCalls.Length != 1) throw new InvalidDataException(error);
        var call = listCalls.Length == 1 ? listCalls[0] : idCalls[0];
        var index = instructions.IndexOf(call);
        if (index < 4 || !GameField(instructions[index - 4], OpCodes.Ldsfld, "PlayerManager", "instance", "PlayerManager")
            || !PickerArgument(instructions[index - 2], method)
            || !GameField(instructions[index - 1], OpCodes.Ldfld, "CardChoice", "pickrID", "System.Int32")
            || call.Operand is not MethodReference lookup) throw new InvalidDataException(error);
        bool helperCall = lookup.DeclaringType.FullName == type!.FullName && lookup.Name == PickerHelperName;
        if (helperCall && (lookup.DeclaringType.Scope != module || !GameType(lookup.ReturnType, "Player")))
            throw new InvalidDataException(error);
        if (helperCall ? lookup.HasThis || lookup.Parameters.Count != 2
                || !GameType(lookup.Parameters[0].ParameterType, "PlayerManager") || lookup.Parameters[1].ParameterType.FullName != "System.Int32"
            : !lookup.HasThis || lookup.Parameters.Count != 1 || lookup.Parameters[0].ParameterType.FullName != "System.Int32")
            throw new InvalidDataException(error);
        var listLoad = instructions[index - 3];
        if (listCalls.Length == 1)
        {
            if (listLoad.OpCode != OpCodes.Ldfld || listLoad.Operand is not FieldReference players
                || !GameType(players.DeclaringType, "PlayerManager") || players.Name != "players"
                || !PlayerList(players.FieldType) || lookup.ReturnType is not GenericParameter { Position: 0 })
                throw new InvalidDataException(error);
        }
        else if (listLoad.OpCode != OpCodes.Nop || lookup.ReturnType.FullName != "Player")
            throw new InvalidDataException(error);
        // Keep the team lookup untouched. These are the two branches of the pinned Prefix,
        // not a general rewrite of list indexing in mods or in the game.
        if (instructions.Count(i => i.Operand is MethodReference m && GameType(m.DeclaringType, "PlayerManager")
            && m.Name == "GetPlayersInTeam" && m.HasThis && m.Parameters.Count == 1
            && m.Parameters[0].ParameterType.FullName == "System.Int32" && m.ReturnType.FullName == "Player[]") != 1)
            throw new InvalidDataException(error);
        var middle = instructions.Skip(index - 3).Take(4).ToHashSet();
        if (instructions.Any(i => (i.Operand is Instruction branch && middle.Contains(branch))
            || (i.Operand is Instruction[] branches && branches.Any(middle.Contains))))
            throw new InvalidDataException(error);
        var helperMethod = PickerHelper(module, type, playerManager!, resolvers[0], error);
        if (helperCall) return; // Already normalized; helper body was also checked.
        // Modern DoPick receives Player.PlayerID, which may be sparse or reordered.
        // Nop preserves instruction/branch identities while retaining PlayerManager on the stack.
        listLoad.OpCode = OpCodes.Nop;
        listLoad.Operand = null;
        call.OpCode = OpCodes.Call;
        call.Operand = helperMethod;
        changes.Add("CardChoiceSpawnUniqueCardPatch: picker PlayerID -> local public-API identity lookup (team lookup unchanged)");
    }

    static MethodDefinition PickerHelper(ModuleDefinition module, TypeDefinition owner, TypeDefinition manager,
        MethodDefinition nativeResolver, string error)
    {
        // GetPlayerWithID is internal in the supported game. Ordinary dependency IL
        // must not call it across assemblies; implement its linear ID lookup locally.
        var rosters = manager.Fields.Where(f => f.Name == "players" && !f.IsStatic && f.IsPublic && PlayerList(module.ImportReference(f.FieldType))).ToArray();
        var getters = manager.Module.GetType("Player")?.Methods.Where(m => m.Name == "get_PlayerID"
            && !m.IsStatic && m.IsPublic && m.Parameters.Count == 0 && m.ReturnType.FullName == "System.Int32").ToArray();
        if (rosters.Length != 1 || getters == null || getters.Length != 1 || !nativeResolver.HasBody)
            throw new InvalidDataException(error);
        var items = nativeResolver.Body.Instructions.Select(i => i.Operand).OfType<MethodReference>()
            .Where(m => m.Name == "get_Item" && PlayerList(module.ImportReference(m.DeclaringType)))
            .GroupBy(m => m.FullName).Select(g => g.First()).ToArray();
        var counts = nativeResolver.Body.Instructions.Select(i => i.Operand).OfType<MethodReference>()
            .Where(m => m.Name == "get_Count" && PlayerList(module.ImportReference(m.DeclaringType)))
            .GroupBy(m => m.FullName).Select(g => g.First()).ToArray();
        if (items.Length != 1 || counts.Length != 1) throw new InvalidDataException(error);
        var expected = new MethodDefinition(PickerHelperName, MethodAttributes.Private | MethodAttributes.Static | MethodAttributes.HideBySig,
            module.ImportReference(nativeResolver.ReturnType));
        expected.Parameters.Add(new ParameterDefinition("manager", ParameterAttributes.None, module.ImportReference(manager)));
        expected.Parameters.Add(new ParameterDefinition("playerId", ParameterAttributes.None, module.TypeSystem.Int32));
        expected.Body.InitLocals = true;
        expected.Body.MaxStackSize = 3;
        expected.Body.Variables.Add(new VariableDefinition(module.TypeSystem.Int32));
        var il = expected.Body.GetILProcessor();
        var loop = il.Create(OpCodes.Ldarg_0);
        var next = il.Create(OpCodes.Ldloc_0);
        var check = il.Create(OpCodes.Ldloc_0);
        var roster = module.ImportReference(rosters[0]);
        var item = module.ImportReference(items[0]);
        il.Append(il.Create(OpCodes.Ldc_I4_0));
        il.Append(il.Create(OpCodes.Stloc_0));
        il.Append(il.Create(OpCodes.Br_S, check));
        il.Append(loop);
        il.Append(il.Create(OpCodes.Ldfld, roster));
        il.Append(il.Create(OpCodes.Ldloc_0));
        il.Append(il.Create(OpCodes.Callvirt, item));
        il.Append(il.Create(OpCodes.Callvirt, module.ImportReference(getters[0])));
        il.Append(il.Create(OpCodes.Ldarg_1));
        il.Append(il.Create(OpCodes.Bne_Un_S, next));
        il.Append(il.Create(OpCodes.Ldarg_0));
        il.Append(il.Create(OpCodes.Ldfld, roster));
        il.Append(il.Create(OpCodes.Ldloc_0));
        il.Append(il.Create(OpCodes.Callvirt, item));
        il.Append(il.Create(OpCodes.Ret));
        il.Append(next);
        il.Append(il.Create(OpCodes.Ldc_I4_1));
        il.Append(il.Create(OpCodes.Add));
        il.Append(il.Create(OpCodes.Stloc_0));
        il.Append(check);
        il.Append(il.Create(OpCodes.Ldarg_0));
        il.Append(il.Create(OpCodes.Ldfld, roster));
        il.Append(il.Create(OpCodes.Callvirt, module.ImportReference(counts[0])));
        il.Append(il.Create(OpCodes.Blt_S, loop));
        il.Append(il.Create(OpCodes.Ldnull));
        il.Append(il.Create(OpCodes.Ret));
        var existing = owner.Methods.Where(m => m.Name == PickerHelperName).ToArray();
        if (existing.Length == 0)
        {
            owner.Methods.Add(expected);
            return expected;
        }
        if (existing.Length != 1 || existing[0].Attributes != expected.Attributes
            || existing[0].ReturnType.FullName != expected.ReturnType.FullName
            || !existing[0].Parameters.Select(p => p.ParameterType.FullName).SequenceEqual(expected.Parameters.Select(p => p.ParameterType.FullName))
            || existing[0].CustomAttributes.Count != 0 || !existing[0].HasBody || !existing[0].Body.InitLocals
            || existing[0].Body.ExceptionHandlers.Count != 0
            || !existing[0].Body.Variables.Select(v => v.VariableType.FullName).SequenceEqual(expected.Body.Variables.Select(v => v.VariableType.FullName))
            || !BodyShape(existing[0]).SequenceEqual(BodyShape(expected))) throw new InvalidDataException(error);
        return existing[0];
    }

    static IEnumerable<string> BodyShape(MethodDefinition method) => method.Body.Instructions.Select(i => i.OpCode.Code + ":" +
        (i.Operand is Instruction branch ? method.Body.Instructions.IndexOf(branch).ToString()
            : i.Operand is MemberReference member ? member.FullName + "@" + member.DeclaringType?.Scope.Name : i.Operand?.ToString() ?? ""));

    static void CardBarRosterSlots(ModuleDefinition module, Game game, List<string> changes)
    {
        if (module.Assembly.Name.Name != "UnboundLib") return;
        const string error = "Unsupported UnboundLib card-bar roster slot contract";
        var type = module.GetType("UnboundLib.Extensions.CardBarHandlerExtensions");
        var methods = type?.Methods.Where(m => m.Name == "Rebuild").ToArray();
        if (methods == null || methods.Length != 1) throw new InvalidDataException(error);
        var method = methods[0];
        if (!method.IsStatic || !method.HasBody || method.ReturnType.FullName != "System.Void" || method.Parameters.Count != 1
            || !GameType(method.Parameters[0].ParameterType, "CardBarHandler") || method.Body.Instructions.Count > 4096
            || method.Body.ExceptionHandlers.Count != 0 || method.Body.Variables.Count < 4)
            throw new InvalidDataException(error);
        var closures = type.NestedTypes.Where(t => t.FullName == method.Body.Variables[3].VariableType.FullName).ToArray();
        var fields = closures.Length == 1 ? closures[0].Fields.Where(f => f.Name == "i" && f.FieldType.FullName == "System.Int32" && !f.IsStatic).ToArray() : null;
        var predicates = closures.Length == 1 ? closures[0].Methods.Where(m => m.Name == "<Rebuild>b__0").ToArray() : null;
        if (fields == null || fields.Length != 1 || predicates == null || predicates.Length != 1) throw new InvalidDataException(error);
        var predicate = predicates[0];
        if (predicate.IsStatic || !predicate.HasBody || predicate.Parameters.Count != 1 || !GameType(predicate.Parameters[0].ParameterType, "Player")
            || predicate.ReturnType.FullName != "System.Boolean" || predicate.Body.Instructions.Count != 6)
            throw new InvalidDataException(error);
        var body = predicate.Body.Instructions;
        if (body[0].OpCode != OpCodes.Ldarg_1 || body[1].OpCode != OpCodes.Callvirt || body[1].Operand is not MethodReference getter
            || !GameType(getter.DeclaringType, "Player") || getter.Name != "get_PlayerID" || !getter.HasThis || getter.Parameters.Count != 0
            || getter.ReturnType.FullName != "System.Int32" || body[2].OpCode != OpCodes.Ldarg_0
            || body[3].OpCode != OpCodes.Ldfld || body[3].Operand is not FieldReference field || field.FullName != fields[0].FullName
            || body[4].OpCode != OpCodes.Ceq || body[5].OpCode != OpCodes.Ret) throw new InvalidDataException(error);
        var instructions = method.Body.Instructions;
        var finds = instructions.Where(i => i.OpCode == OpCodes.Callvirt && i.Operand is MethodReference m
            && PlayerList(m.DeclaringType) && m.Name == "Find").ToArray();
        var items = instructions.Where(i => i.OpCode == OpCodes.Callvirt && i.Operand is MethodReference m
            && PlayerList(m.DeclaringType) && m.Name == "get_Item").ToArray();
        if (finds.Length + items.Length != 1) throw new InvalidDataException(error);
        var call = finds.Length == 1 ? finds[0] : items[0];
        var index = instructions.IndexOf(call);
        if (index < 5 || !GameField(instructions[index - 5], OpCodes.Ldsfld, "PlayerManager", "instance", "PlayerManager")
            || instructions[index - 4].OpCode != OpCodes.Ldfld || instructions[index - 4].Operand is not FieldReference roster
            || !GameType(roster.DeclaringType, "PlayerManager") || roster.Name != "players" || !PlayerList(roster.FieldType)
            || instructions[index - 3].OpCode != OpCodes.Ldloc_3) throw new InvalidDataException(error);
        var selector = instructions[index - 2];
        var construction = instructions[index - 1];
        if (finds.Length == 1)
        {
            if (selector.OpCode != OpCodes.Ldftn || selector.Operand is not MethodReference callback || callback.FullName != predicate.FullName
                || construction.OpCode != OpCodes.Newobj || construction.Operand is not MethodReference constructor
                || constructor.DeclaringType.FullName != "System.Predicate`1<Player>" || constructor.Parameters.Count != 2
                || constructor.Parameters[0].ParameterType.FullName != "System.Object" || constructor.Parameters[1].ParameterType.FullName != "System.IntPtr"
                || call.Operand is not MethodReference find || !find.HasThis || find.Parameters.Count != 1
                || find.Parameters[0].ParameterType.FullName != "System.Predicate`1<!0>") throw new InvalidDataException(error);
        }
        else if (selector.OpCode != OpCodes.Ldfld || selector.Operand is not FieldReference slot || slot.FullName != fields[0].FullName
            || construction.OpCode != OpCodes.Nop || construction.Operand != null
            || call.Operand is not MethodReference item || !item.HasThis || item.Parameters.Count != 1
            || item.Parameters[0].ParameterType.FullName != "System.Int32") throw new InvalidDataException(error);
        var middle = instructions.Skip(index - 2).Take(3).ToHashSet();
        if (instructions.Any(i => (i.Operand is Instruction branch && middle.Contains(branch))
            || (i.Operand is Instruction[] branches && branches.Any(middle.Contains)))) throw new InvalidDataException(error);
        if (finds.Length == 0) return;
        var native = game.AssemblyCSharp.GetType("PlayerManager")?.Methods.SingleOrDefault(m => m.Name == "GetPlayerWithID" && m.Parameters.Count == 1);
        var nativeItem = native?.Body.Instructions.Select(i => i.Operand).OfType<MethodReference>()
            .FirstOrDefault(m => m.Name == "get_Item" && PlayerList(module.ImportReference(m.DeclaringType)));
        if (nativeItem == null) throw new InvalidDataException(error);
        selector.OpCode = OpCodes.Ldfld;
        selector.Operand = fields[0];
        construction.OpCode = OpCodes.Nop;
        construction.Operand = null;
        call.Operand = module.ImportReference(nativeItem);
        changes.Add("UnboundLib: rebuilt card-bar color owners use roster slots (PlayerIDs unchanged)");
    }

    static void ProjectileDynamicTargets(ModuleDefinition module, Game game, List<string> changes)
    {
        if (module.Assembly.Name.Name != "ModdingUtils") return;
        const string error = "Unsupported ModdingUtils ProjectileInit dynamic target contract";
        var contracts = new[] {
            (type: "ModdingUtils.AIMinion.Patches.ProjectileInit_PatchGetCorrectPlayerOffline", prefix: "OFFLINE_"),
            (type: "ModdingUtils.AIMinion.Patches.ProjectileInit_PatchGetCorrectPlayerRPCs", prefix: "RPCA_")
        };
        var plans = new List<(TypeDefinition type, CustomAttribute attribute)>();
        foreach (var contract in contracts)
        {
            var type = module.GetType(contract.type);
            var patches = type?.CustomAttributes.Where(a => a.AttributeType.FullName == "HarmonyLib.HarmonyPatch").ToArray();
            var targets = type?.Methods.Where(m => m.Name == "RPCMethods").ToArray();
            var transpilers = type?.Methods.Where(m => m.Name == "ConvertToPlayerID").ToArray();
            if (type == null || patches == null || patches.Length != 1 || targets == null || targets.Length != 1
                || transpilers == null || transpilers.Length != 1) throw new InvalidDataException(error);
            var attribute = patches[0];
            if (attribute.AttributeType.Scope.Name != "0Harmony" || attribute.Fields.Count != 0 || attribute.Properties.Count != 0
                || attribute.Constructor.Parameters.Count != attribute.ConstructorArguments.Count
                || attribute.ConstructorArguments.Count > 1 || (attribute.ConstructorArguments.Count == 1
                    && (attribute.ConstructorArguments[0].Value is not TypeReference projectile || !GameType(projectile, "ProjectileInit"))))
                throw new InvalidDataException(error);
            var target = targets[0];
            var transpiler = transpilers[0];
            if (!target.IsStatic || !target.HasBody || target.Parameters.Count != 0
                || target.ReturnType.FullName != "System.Collections.Generic.IEnumerable`1<System.Reflection.MethodBase>"
                || target.CustomAttributes.Count(a => a.AttributeType.FullName == "HarmonyLib.HarmonyTargetMethods") != 1
                || !transpiler.IsStatic || !transpiler.HasBody || transpiler.Parameters.Count != 1
                || transpiler.Parameters[0].ParameterType.FullName != "System.Collections.Generic.IEnumerable`1<HarmonyLib.CodeInstruction>"
                || transpiler.ReturnType.FullName != "System.Collections.Generic.IEnumerable`1<HarmonyLib.CodeInstruction>"
                || transpiler.CustomAttributes.Count(a => a.AttributeType.FullName == "HarmonyLib.HarmonyTranspiler") != 1
                || type.Methods.Any(m => m.CustomAttributes.Any(a => a.AttributeType.FullName == "HarmonyLib.HarmonyPatch")))
                throw new InvalidDataException(error);
            var iterators = target.CustomAttributes.Where(a => a.AttributeType.FullName == "System.Runtime.CompilerServices.IteratorStateMachineAttribute").ToArray();
            if (iterators.Length != 1 || iterators[0].ConstructorArguments.Count != 1
                || iterators[0].ConstructorArguments[0].Value is not TypeReference iterator)
                throw new InvalidDataException(error);
            var nested = type.NestedTypes.SingleOrDefault(t => t.FullName == iterator.FullName);
            var moves = nested?.Methods.Where(m => m.Name == "MoveNext" && !m.IsStatic && m.Parameters.Count == 0
                && m.ReturnType.FullName == "System.Boolean" && m.HasBody).ToArray();
            if (moves == null || moves.Length != 1 || moves[0].Body.Instructions.Count > 4096)
                throw new InvalidDataException(error);
            var instructions = moves[0].Body.Instructions;
            var names = new[] { contract.prefix + "Init", contract.prefix + "Init_SeparateGun", contract.prefix + "Init_noAmmoUse" };
            if (!instructions.Where(i => i.OpCode == OpCodes.Ldstr).Select(i => i.Operand as string).SequenceEqual(names)
                || instructions.Count(i => i.OpCode == OpCodes.Ldtoken && i.Operand is TypeReference t && t.FullName == "ProjectileInit") != 3
                || instructions.Count(i => i.OpCode == OpCodes.Ldtoken && i.Operand is TypeReference t && GameType(t, "ProjectileInit")) != 3
                || instructions.Count(i => i.Operand is MethodReference m && m.DeclaringType.FullName == "HarmonyLib.AccessTools" && m.Name == "Method") != 3
                || instructions.Count(i => i.OpCode == OpCodes.Call && i.Operand is MethodReference m
                    && m.DeclaringType.FullName == "HarmonyLib.AccessTools" && m.DeclaringType.Scope.Name == "0Harmony"
                    && m.Name == "Method" && !m.HasThis && m.Parameters.Count == 4
                    && m.Parameters[0].ParameterType.FullName == "System.Type" && m.Parameters[1].ParameterType.FullName == "System.String"
                    && m.Parameters[2].ParameterType.FullName == "System.Type[]" && m.Parameters[3].ParameterType.FullName == "System.Type[]"
                    && m.ReturnType.FullName == "System.Reflection.MethodInfo") != 3)
                throw new InvalidDataException(error);
            var native = game.AssemblyCSharp.GetType("ProjectileInit");
            foreach (var name in names)
            {
                var expected = name.EndsWith("_SeparateGun", StringComparison.Ordinal)
                    ? new[] { "System.Int32", "System.Int32", "System.Int32", "System.Single", "System.Single" }
                    : new[] { "System.Int32", "System.Int32", "System.Single", "System.Single" };
                if (native == null || native.Methods.Count(m => m.Name == name && !m.IsStatic && m.ReturnType.FullName == "System.Void"
                    && m.Parameters.Select(p => p.ParameterType.FullName).SequenceEqual(expected)) != 1)
                    throw new InvalidDataException(error);
                if (contract.prefix == "OFFLINE_" && native.Methods.Count(m => m.Name == name) != 1)
                    throw new InvalidDataException(error); // Offline iterator does not pass argumentTypes.
            }
            if (attribute.ConstructorArguments.Count == 1) plans.Add((type, attribute));
        }
        if (plans.Count == 0) return;
        var harmonyPatch = game.Resolver.Get("0Harmony")?.MainModule.GetType("HarmonyLib.HarmonyPatch");
        var constructors = harmonyPatch?.Methods.Where(m => m.IsConstructor && !m.IsStatic && m.Parameters.Count == 0).ToArray();
        if (constructors == null || constructors.Length != 1) throw new InvalidDataException(error);
        foreach (var plan in plans)
        {
            // RPCMethods already supplies complete targets. HarmonyX forbids combining
            // those dynamic targets with a separate class-level ProjectileInit target.
            var marker = new CustomAttribute(module.ImportReference(constructors[0]));
            plan.type.CustomAttributes[plan.type.CustomAttributes.IndexOf(plan.attribute)] = marker;
            changes.Add("ModdingUtils: " + plan.type.Name + " dynamic ProjectileInit targets use an empty HarmonyPatch marker");
        }
    }

    static void ValidateCardBarBoundsGuard(ModuleDefinition module)
    {
        if (module.Assembly.Name.Name != "ModdingUtils") return;
        const string error = "Unsupported ModdingUtils card-bar ID-bounds guard contract";
        var type = module.GetType("ModdingUtils.AIMinion.Patches.CardBarHandlerPatchAddCard");
        var attributes = type?.CustomAttributes.Where(a => a.AttributeType.FullName == "HarmonyLib.HarmonyPatch").ToArray();
        var prefixes = type?.Methods.Where(m => m.Name == "Prefix").ToArray();
        if (attributes == null || attributes.Length != 1 || attributes[0].AttributeType.Scope.Name != "0Harmony"
            || attributes[0].Fields.Count != 0 || attributes[0].Properties.Count != 0 || attributes[0].ConstructorArguments.Count != 2
            || attributes[0].ConstructorArguments[0].Value is not TypeReference handler || !GameType(handler, "CardBarHandler")
            || attributes[0].ConstructorArguments[1].Value is not string name || name != "AddCard"
            || prefixes == null || prefixes.Length != 1) throw new InvalidDataException(error);
        var prefix = prefixes[0];
        if (!prefix.IsStatic || !prefix.IsPrivate || prefix.GenericParameters.Count != 0 || !prefix.HasBody || prefix.ReturnType.FullName != "System.Boolean"
            || prefix.Parameters.Count != 2 || !GameType(prefix.Parameters[0].ParameterType, "CardBarHandler")
            || prefix.Parameters[1].ParameterType.FullName != "System.Int32" || prefix.CustomAttributes.Count != 0
            || prefix.Body.ExceptionHandlers.Count != 0 || !prefix.Body.InitLocals || prefix.Body.Variables.Count != 2
            || prefix.Body.Variables.Any(v => v.VariableType.FullName != "System.Boolean")) throw new InvalidDataException(error);
        var expected = new[] { Code.Nop, Code.Ldarg_1, Code.Ldarg_0, Code.Call, Code.Ldstr, Code.Callvirt, Code.Callvirt,
            Code.Castclass, Code.Ldlen, Code.Conv_I4, Code.Clt, Code.Ldc_I4_0, Code.Ceq, Code.Stloc_0, Code.Ldloc_0,
            Code.Brfalse_S, Code.Nop, Code.Ldc_I4_0, Code.Stloc_1, Code.Br_S, Code.Ldc_I4_1, Code.Stloc_1,
            Code.Br_S, Code.Ldloc_1, Code.Ret };
        var code = prefix.Body.Instructions;
        if (!code.Select(i => i.OpCode.Code).SequenceEqual(expected) || code[4].Operand is not string field || field != "cardBars"
            || code[7].Operand is not ArrayType bars || !GameType(bars.ElementType, "CardBar")
            || code[15].Operand != code[20] || code[19].Operand != code[23] || code[22].Operand != code[23])
            throw new InvalidDataException(error);
        bool TraverseCall(int index, string methodName, bool instance, string result, params string[] arguments) =>
            code[index].Operand is MethodReference method && method.DeclaringType.FullName == "HarmonyLib.Traverse"
            && method.DeclaringType.Scope.Name == "0Harmony" && method.Name == methodName && method.HasThis == instance
            && method.ReturnType.FullName == result && method.Parameters.Select(p => p.ParameterType.FullName).SequenceEqual(arguments);
        if (!TraverseCall(3, "Create", false, "HarmonyLib.Traverse", "System.Object")
            || !TraverseCall(5, "Field", true, "HarmonyLib.Traverse", "System.String")
            || !TraverseCall(6, "GetValue", true, "System.Object")) throw new InvalidDataException(error);
        // This method is not rewritten. Runtime may override only its known
        // out-of-range false result for an attested, still-live bound player.
    }
}
