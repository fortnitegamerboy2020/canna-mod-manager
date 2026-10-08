// Canna MIT. A bounded adaptation for the modern game's player-ID card pick.
using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Reflection.Emit;
using System.Runtime.CompilerServices;
using HarmonyLib;

namespace RoundsPort.Runtime
{
    [HarmonyPatch(typeof(ApplyCardStats), nameof(ApplyCardStats.Pick),
        new Type[] { typeof(int), typeof(bool), typeof(PickerType) })]
    internal static class Canna_ApplyCardStatsPicker_Fix
    {
        internal static IEnumerable<CodeInstruction> Transpiler(IEnumerable<CodeInstruction> instructions, MethodBase __originalMethod)
        {
            const string error = "Unsupported modern ApplyCardStats.Pick player-ID lookup contract";
            var parameters = __originalMethod?.GetParameters();
            if (__originalMethod == null || __originalMethod.DeclaringType != typeof(ApplyCardStats)
                || __originalMethod.Name != nameof(ApplyCardStats.Pick) || __originalMethod.IsStatic
                || __originalMethod is not MethodInfo pick || pick.ReturnType != typeof(void)
                || parameters.Length != 3 || parameters[0].ParameterType != typeof(int)
                || parameters[1].ParameterType != typeof(bool) || parameters[2].ParameterType != typeof(PickerType)
                || typeof(Player).GetProperty(nameof(Player.PlayerID))?.PropertyType != typeof(int))
                throw new InvalidOperationException(error);
            var instance = AccessTools.Field(typeof(PlayerManager), "instance");
            var players = AccessTools.Field(typeof(PlayerManager), "players");
            var listItem = AccessTools.Method(typeof(List<Player>), "get_Item", new[] { typeof(int) });
            var resolver = AccessTools.Method(typeof(PlayerManager), "GetPlayerWithID", new[] { typeof(int) });
            var team = AccessTools.Method(typeof(PlayerManager), nameof(PlayerManager.GetPlayersInTeam), new[] { typeof(int) });
            if (instance == null || instance.DeclaringType != typeof(PlayerManager) || !instance.IsStatic || instance.FieldType != typeof(PlayerManager)
                || players == null || players.DeclaringType != typeof(PlayerManager) || players.IsStatic || players.FieldType != typeof(List<Player>)
                || listItem == null || resolver == null || resolver.DeclaringType != typeof(PlayerManager) || resolver.IsStatic || resolver.ReturnType != typeof(Player)
                || team == null || team.DeclaringType != typeof(PlayerManager) || team.IsStatic || team.ReturnType != typeof(Player[]))
                throw new InvalidOperationException(error);
            // Work on copies: an unsupported pattern must not leave a partly changed
            // instruction stream for another Harmony transpiler.
            var code = instructions.Select(i => new CodeInstruction(i)).ToList();
            if (code.Count > 8192 || code.Count(i => i.opcode == OpCodes.Callvirt && Equals(i.operand, team)) != 1)
                throw new InvalidOperationException(error);
            var indexed = code.Select((instruction, index) => (instruction, index))
                .Where(x => x.instruction.opcode == OpCodes.Callvirt && Equals(x.instruction.operand, listItem)).ToArray();
            var resolved = code.Select((instruction, index) => (instruction, index))
                .Where(x => x.instruction.opcode == OpCodes.Callvirt && Equals(x.instruction.operand, resolver)).ToArray();
            if (indexed.Length + resolved.Length != 1) throw new InvalidOperationException(error);
            var lookup = indexed.Length == 1 ? indexed[0].index : resolved[0].index;
            if (lookup < 3 || lookup + 1 >= code.Count
                || code[lookup - 3].opcode != OpCodes.Ldsfld || !Equals(code[lookup - 3].operand, instance)
                || code[lookup - 1].opcode != OpCodes.Ldarg_1 || code[lookup + 1].opcode != OpCodes.Stelem_Ref)
                throw new InvalidOperationException(error);
            var listLoad = code[lookup - 2];
            if ((indexed.Length == 1 && (listLoad.opcode != OpCodes.Ldfld || !Equals(listLoad.operand, players)))
                || (indexed.Length == 0 && (listLoad.opcode != OpCodes.Nop || listLoad.operand != null))
                || code.Skip(lookup - 2).Take(3).Any(i => i.labels.Count != 0 || i.blocks.Count != 0))
                throw new InvalidOperationException(error);
            if (indexed.Length == 0) return code;
            // CardChoice.Pick passes an actual PlayerID. Keep the existing team
            // branch, achievements, card application and network dispatch intact.
            listLoad.opcode = OpCodes.Nop;
            listLoad.operand = null;
            code[lookup].operand = resolver;
            return code;
        }
    }

    internal static class Canna_CardBarSlotBindings
    {
        sealed class Binding
        {
            public readonly Player[] Players;
            public readonly int[] Ids;
            public readonly CardBar[] Bars;
            public Binding(CardBar[] bars, IList<Player> players)
            {
                Bars = (CardBar[])bars.Clone();
                Players = players.ToArray();
                Ids = Players.Select(p => p.PlayerID).ToArray();
            }
        }

        static readonly ConditionalWeakTable<CardBar[], Binding> bindings = new ConditionalWeakTable<CardBar[], Binding>();

        static void Validate(CardBar[] bars, IList<Player> players)
        {
            if (bars == null || players == null || players.Count > 4096 || bars.Length != players.Count
                || bars.Any(b => b == null) || players.Any(p => p == null || p.PlayerID < 0)
                || bars.Distinct().Count() != bars.Length
                || players.Select(p => p.PlayerID).Distinct().Count() != players.Count)
                throw new InvalidOperationException("Card-bar roster is missing, duplicated or changed; rebuild its bars before adding a card");
        }

        internal static void Bind(CardBar[] bars, IList<Player> players)
        {
            Validate(bars, players);
            var binding = new Binding(bars, players);
            bindings.Remove(bars);
            bindings.Add(bars, binding);
        }

        internal static CardBar Resolve(CardBar[] bars, int playerId)
        {
            if (bars == null) throw new InvalidOperationException("Card-bar array is missing");
            if (bindings.TryGetValue(bars, out var binding))
            {
                ValidateOwnership(bars, binding);
                var slot = Array.IndexOf(binding.Ids, playerId);
                if (slot < 0) throw new InvalidOperationException("Card-bar picker ID is absent from the bound roster");
                return bars[slot];
            }
            // Unbound arrays keep the original generic slot API. CardBarPatch's
            // menu preview has four slots without four players; it is not an
            // identity-attested gameplay layout. Only Rebuild creates that binding.
            if (playerId < 0 || playerId >= bars.Length || bars[playerId] == null)
                throw new InvalidOperationException("Card-bar slot is absent from the unbound array");
            return bars[playerId];
        }

        static void ValidateOwnership(CardBar[] bars, Binding binding)
        {
            var players = PlayerManager.instance?.players;
            Validate(bars, players);
            if (binding.Players.Length != players.Count || binding.Ids.Length != bars.Length
                || binding.Bars.Length != bars.Length || binding.Bars.Where((bar, i) => !ReferenceEquals(bar, bars[i])).Any()
                || binding.Players.Where((p, i) => p == null || p.PlayerID != binding.Ids[i]
                    || !players.Any(current => ReferenceEquals(current, p))).Any())
                throw new InvalidOperationException("Card-bar ownership changed; rebuild its bars before adding a card");
        }

        internal static bool ContainsLiveBoundPlayer(CardBar[] bars, int playerId)
        {
            if (bars == null || !bindings.TryGetValue(bars, out var binding)) return false;
            ValidateOwnership(bars, binding);
            return Array.IndexOf(binding.Ids, playerId) >= 0;
        }
    }

    [HarmonyPatch]
    internal static class Canna_CardBarRebuildBindings_Fix
    {
        static MethodBase Target() => AccessTools.Method(Types.Find("UnboundLib.Extensions.CardBarHandlerExtensions"), "Rebuild", new[] { typeof(CardBarHandler) });

        static bool Prepare()
        {
            var method = Target() as MethodInfo;
            return method != null && method.IsStatic && method.ReturnType == typeof(void)
                && method.GetParameters().Length == 1 && method.GetParameters()[0].ParameterType == typeof(CardBarHandler);
        }

        static MethodBase TargetMethod() => Target();

        internal static void Postfix(CardBarHandler __0)
        {
            var field = AccessTools.Field(typeof(CardBarHandler), "cardBars");
            if (__0 == null || field == null || field.IsStatic || field.DeclaringType != typeof(CardBarHandler) || field.FieldType != typeof(CardBar[]))
                throw new InvalidOperationException("Unsupported modern CardBarHandler slot binding contract");
            Canna_CardBarSlotBindings.Bind((CardBar[])field.GetValue(__0), PlayerManager.instance?.players);
        }
    }

    [HarmonyPatch(typeof(CardBarHandler), nameof(CardBarHandler.AddCard), new Type[] { typeof(int), typeof(CardInfo) })]
    internal static class Canna_CardBarAddCard_Fix
    {
        internal static IEnumerable<CodeInstruction> Transpiler(IEnumerable<CodeInstruction> instructions, MethodBase __originalMethod)
        {
            const string error = "Unsupported modern CardBarHandler.AddCard slot lookup contract";
            var method = __originalMethod as MethodInfo;
            var field = AccessTools.Field(typeof(CardBarHandler), "cardBars");
            var add = AccessTools.Method(typeof(CardBar), "AddCard", new[] { typeof(CardInfo) });
            var resolve = AccessTools.Method(typeof(Canna_CardBarSlotBindings), nameof(Canna_CardBarSlotBindings.Resolve), new[] { typeof(CardBar[]), typeof(int) });
            if (method == null || method.DeclaringType != typeof(CardBarHandler) || method.Name != nameof(CardBarHandler.AddCard)
                || method.IsStatic || method.ReturnType != typeof(void)
                || !method.GetParameters().Select(p => p.ParameterType).SequenceEqual(new[] { typeof(int), typeof(CardInfo) })
                || field == null || field.DeclaringType != typeof(CardBarHandler) || field.IsStatic || field.FieldType != typeof(CardBar[])
                || add == null || add.DeclaringType != typeof(CardBar) || add.IsStatic || add.ReturnType != typeof(void)
                || resolve == null || !resolve.IsStatic || resolve.ReturnType != typeof(CardBar))
                throw new InvalidOperationException(error);
            var code = instructions.Select(i => new CodeInstruction(i)).ToList();
            if (code.Count != 7 || code[0].opcode != OpCodes.Ldarg_0 || code[1].opcode != OpCodes.Ldfld || !Equals(code[1].operand, field)
                || code[2].opcode != OpCodes.Ldarg_1 || code[4].opcode != OpCodes.Ldarg_2
                || code[5].opcode != OpCodes.Callvirt || !Equals(code[5].operand, add) || code[6].opcode != OpCodes.Ret
                || code[3].labels.Count != 0 || code[3].blocks.Count != 0)
                throw new InvalidOperationException(error);
            if (code[3].opcode == OpCodes.Call && Equals(code[3].operand, resolve)) return code;
            if (code[3].opcode != OpCodes.Ldelem_Ref || code[3].operand != null) throw new InvalidOperationException(error);
            // This body-only conversion leaves the original argument visible to
            // UnboundLib's Prefix, which stores CardData under the actual PlayerID.
            code[3].opcode = OpCodes.Call;
            code[3].operand = resolve;
            return code;
        }
    }

    [HarmonyPatch]
    internal static class Canna_ModdingUtilsCardBarBounds_Fix
    {
        const string TargetType = "ModdingUtils.AIMinion.Patches.CardBarHandlerPatchAddCard";
        static readonly byte[] boundsBody = {
            0x00, 0x03, 0x02, 0x28, 0, 0, 0, 0, 0x72, 0, 0, 0, 0, 0x6f, 0, 0, 0, 0,
            0x6f, 0, 0, 0, 0, 0x74, 0, 0, 0, 0, 0x8e, 0x69, 0xfe, 0x04, 0x16, 0xfe,
            0x01, 0x0a, 0x06, 0x2c, 0x05, 0x00, 0x16, 0x0b, 0x2b, 0x04, 0x17, 0x0b,
            0x2b, 0x00, 0x07, 0x2a
        };

        static MethodInfo Target() => AccessTools.Method(Types.Find(TargetType), "Prefix", new[] { typeof(CardBarHandler), typeof(int) });

        static bool OnlyOurPostfix(MethodBase method)
        {
            if (method == null) return false;
            var patches = Harmony.GetPatchInfo(method);
            return patches == null || (patches.Prefixes.Count == 0 && patches.Transpilers.Count == 0 && patches.Finalizers.Count == 0
                && patches.Postfixes.All(p => p.PatchMethod.DeclaringType == typeof(Canna_ModdingUtilsCardBarBounds_Fix)
                    && p.PatchMethod.Name == nameof(Postfix)));
        }

        internal static bool KnownBoundsBody(MethodInfo method)
        {
            if (method == null || method.IsGenericMethod || !method.IsPrivate || !method.IsStatic || method.ReturnType != typeof(bool)
                || !method.GetParameters().Select(p => p.ParameterType).SequenceEqual(new[] { typeof(CardBarHandler), typeof(int) })) return false;
            var body = method.GetMethodBody();
            if (body == null || !body.InitLocals || body.ExceptionHandlingClauses.Count != 0
                || body.LocalVariables.Count != 2 || body.LocalVariables.Any(v => v.LocalType != typeof(bool))) return false;
            var code = body.GetILAsByteArray();
            if (code == null || code.Length != boundsBody.Length) return false;
            var tokens = new[] { 4, 9, 14, 19, 24 };
            for (int i = 0; i < code.Length; i++)
                if (!tokens.Any(start => i >= start && i < start + 4) && code[i] != boundsBody[i]) return false;
            try
            {
                var create = method.Module.ResolveMethod(BitConverter.ToInt32(code, 4)) as MethodInfo;
                var field = method.Module.ResolveMethod(BitConverter.ToInt32(code, 14)) as MethodInfo;
                var value = method.Module.ResolveMethod(BitConverter.ToInt32(code, 19)) as MethodInfo;
                return create != null && create.DeclaringType == typeof(Traverse) && create.Name == nameof(Traverse.Create)
                    && create.IsStatic && create.ReturnType == typeof(Traverse) && create.GetParameters().Select(p => p.ParameterType).SequenceEqual(new[] { typeof(object) })
                    && field != null && field.DeclaringType == typeof(Traverse) && field.Name == nameof(Traverse.Field)
                    && !field.IsStatic && field.ReturnType == typeof(Traverse) && field.GetParameters().Select(p => p.ParameterType).SequenceEqual(new[] { typeof(string) })
                    && value != null && value.DeclaringType == typeof(Traverse) && value.Name == nameof(Traverse.GetValue)
                    && !value.IsStatic && value.ReturnType == typeof(object) && value.GetParameters().Length == 0
                    && method.Module.ResolveString(BitConverter.ToInt32(code, 9)) == "cardBars"
                    && method.Module.ResolveType(BitConverter.ToInt32(code, 24)) == typeof(CardBar[]);
            }
            catch (ArgumentException) { return false; }
        }

        static bool Prepare()
        {
            var method = Target();
            if (method == null || method.DeclaringType.FullName != TargetType || method.DeclaringType.Assembly.GetName().Name != "ModdingUtils"
                || !KnownBoundsBody(method) || !OnlyOurPostfix(method)) return false;
            var annotations = CustomAttributeData.GetCustomAttributes(method.DeclaringType).Where(a => a.AttributeType == typeof(HarmonyPatch)).ToArray();
            return annotations.Length == 1 && annotations[0].NamedArguments.Count == 0 && annotations[0].ConstructorArguments.Count == 2
                && Equals(annotations[0].ConstructorArguments[0].Value, typeof(CardBarHandler))
                && Equals(annotations[0].ConstructorArguments[1].Value, "AddCard");
        }

        static MethodBase TargetMethod() => Target();

        internal static void Postfix(CardBarHandler __0, int __1, bool __runOriginal, MethodBase __originalMethod, ref bool __result)
        {
            if (__result || !__runOriginal || __0 == null || !OnlyOurPostfix(__originalMethod)) return;
            var field = AccessTools.Field(typeof(CardBarHandler), "cardBars");
            if (field == null || field.IsStatic || field.FieldType != typeof(CardBar[]) || field.DeclaringType != typeof(CardBarHandler)) return;
            var bars = (CardBar[])field.GetValue(__0);
            // The validated original returns false solely for ID >= array length.
            // Keep unbound AI/menu guards and any other Harmony conditions intact.
            if (bars == null || __1 < bars.Length || !Canna_CardBarSlotBindings.ContainsLiveBoundPlayer(bars, __1)) return;
            Canna_CardBarSlotBindings.Resolve(bars, __1);
            __result = true;
        }
    }
}
