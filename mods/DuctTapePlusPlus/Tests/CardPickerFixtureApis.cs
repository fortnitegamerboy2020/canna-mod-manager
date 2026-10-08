// Harmless API surfaces for actual translated Prefix IL. No Unity/game DLL is loaded.
using Mono.Cecil;

sealed class Game
{
    public readonly ModuleDefinition AssemblyCSharp;
    public readonly FixtureResolver Resolver;
    public Game(ModuleDefinition native, ModuleDefinition harmony)
    { AssemblyCSharp = native; Resolver = new FixtureResolver(native, harmony); }
}
sealed class FixtureResolver
{
    readonly Dictionary<string, AssemblyDefinition> modules;
    public FixtureResolver(ModuleDefinition native, ModuleDefinition harmony)
    { modules = new() { ["Assembly-CSharp"] = native.Assembly, ["0Harmony"] = harmony.Assembly }; }
    public AssemblyDefinition Get(string name) => modules.GetValueOrDefault(name);
    public void Add(ModuleDefinition module) => modules[module.Assembly.Name.Name] = module.Assembly;
}
static class Scanner
{
    public static IEnumerable<TypeDefinition> AllTypes(ModuleDefinition module)
    { foreach (var type in module.Types) foreach (var item in Walk(type)) yield return item; }
    static IEnumerable<TypeDefinition> Walk(TypeDefinition type)
    { yield return type; foreach (var nested in type.NestedTypes) foreach (var item in Walk(nested)) yield return item; }
}

namespace UnityEngine
{
    public class Object
    {
        public string name { get; set; }
        public static implicit operator bool(Object value) => value is not null;
        public static bool operator ==(Object left, Object right) => ReferenceEquals(left, right);
        public static bool operator !=(Object left, Object right) => !ReferenceEquals(left, right);
        public override bool Equals(object obj) => ReferenceEquals(this, obj);
        public override int GetHashCode() => System.Runtime.CompilerServices.RuntimeHelpers.GetHashCode(this);
    }
    public class GameObject : Object
    {
        readonly Dictionary<Type, Component> components = new();
        public T Attach<T>(T component) where T : Component
        { component.gameObject = this; components[typeof(T)] = component; return component; }
        public T GetComponent<T>() => (T)(object)components.GetValueOrDefault(typeof(T));
        public T GetComponentInChildren<T>() => GetComponent<T>();
    }
    public class Component : Object
    {
        public GameObject gameObject { get; set; }
        public T GetComponent<T>() => gameObject.GetComponent<T>();
    }
    public class Behaviour : Component { public bool enabled { get; set; } }
    public class Collider2D : Behaviour { }
    public struct Vector3 { }
    public struct Quaternion { }
}
namespace HarmonyLib
{
    [AttributeUsage(AttributeTargets.Class)]
    public class HarmonyPatch : Attribute
    { public HarmonyPatch() { } public HarmonyPatch(Type type, string name, Type[] parameters) { } }
    public static class AccessTools
    {
        const System.Reflection.BindingFlags Flags = System.Reflection.BindingFlags.Public | System.Reflection.BindingFlags.NonPublic
            | System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.Static;
        public static System.Reflection.FieldInfo Field(Type type, string name) => type.GetField(name, Flags);
        public static System.Reflection.MethodInfo Method(Type type, string name, Type[] parameters) => type.GetMethod(name, Flags, null, parameters, null);
    }
    public sealed class Patch { public System.Reflection.MethodInfo PatchMethod; }
    public sealed class Patches
    {
        public readonly List<Patch> Prefixes = new(), Postfixes = new(), Transpilers = new(), Finalizers = new();
    }
    public static class Harmony
    {
        public static readonly Dictionary<System.Reflection.MethodBase, Patches> FixturePatches = new();
        public static Patches GetPatchInfo(System.Reflection.MethodBase method) => method == null ? null : FixturePatches.GetValueOrDefault(method);
    }
    public class Traverse
    {
        readonly object target; string field;
        Traverse(object value) { target = value; }
        public static Traverse Create(object value) => new(value);
        public Traverse Field(string name) { field = name; return this; }
        public object GetValue() => AccessTools.Field(target.GetType(), field).GetValue(target);
    }
    public enum ExceptionBlockType { BeginExceptionBlock }
    public class ExceptionBlock { public readonly ExceptionBlockType blockType; public ExceptionBlock(ExceptionBlockType type) { blockType = type; } }
    public class CodeInstruction
    {
        public System.Reflection.Emit.OpCode opcode; public object operand;
        public List<System.Reflection.Emit.Label> labels = new(); public List<ExceptionBlock> blocks = new();
        public CodeInstruction(System.Reflection.Emit.OpCode op, object arg = null) { opcode = op; operand = arg; }
        public CodeInstruction(CodeInstruction source)
        { opcode = source.opcode; operand = source.operand; labels = new(source.labels); blocks = new(source.blocks); }
    }
}
public enum PickerType { Team = 0, Player = 1 }
public class Player : UnityEngine.Component
{
    public int PlayerID { get; set; }
    public int TeamID { get; set; }
    public CharacterData data;
    public bool IsLocal { get; set; }
    public int NumRareCards, NumBlockCards;
    public readonly HashSet<CardCategory> BlockedCategories = new();
}
public class PlayerManager
{
    public static PlayerManager instance = new();
    public List<Player> players = new();
    public int IdentityCalls, TeamCalls;
    internal Player GetPlayerWithID(int id) { IdentityCalls++; return players.FirstOrDefault(p => p.PlayerID == id); }
    public Player[] GetPlayersInTeam(int team) { TeamCalls++; return players.Where(p => p.TeamID == team).ToArray(); }
}
public class CardInfo : UnityEngine.Component
{
    public enum Rarity { Common, Uncommon, Rare }
    public Rarity rarity;
    public bool AffectsBlock { get; set; }
    public string CardName => gameObject.name;
    public CardInfo sourceCard;
    public CardCategory[] categories = Array.Empty<CardCategory>();
    public CardCategory[] blacklistedCategories = Array.Empty<CardCategory>();
}
public class CardCategory : UnityEngine.Object { }
public class CharacterData : UnityEngine.Component { public List<CardInfo> currentCards = new(); public Photon.Pun.PhotonView view; }
public class Holding : UnityEngine.Component { public Holdable holdable; }
public class Holdable : UnityEngine.Component { }
public class Gun : UnityEngine.Component { public bool lockGunToDefault; }
public class GunAmmo : UnityEngine.Component { }
public class HealthHandler : UnityEngine.Component { }
public class Gravity : UnityEngine.Component { }
public class Block : UnityEngine.Component { }
public class CharacterStatModifiers : UnityEngine.Component { }
public class DamagableEvent : UnityEngine.Component { }
public class CardChoice
{
    public static CardChoice instance;
    public int pickrID;
    public PickerType pickerType;
    public CardInfo[] cards = Array.Empty<CardInfo>();
    public List<UnityEngine.GameObject> spawnedCards = new();
    private UnityEngine.GameObject Spawn(UnityEngine.GameObject source, UnityEngine.Vector3 pos, UnityEngine.Quaternion rot)
    {
        var copy = new UnityEngine.GameObject { name = source.name + "(Clone)" };
        copy.Attach(new CardInfo()); copy.Attach(new DamagableEvent()); copy.Attach(new UnityEngine.Collider2D());
        return copy;
    }
}
public class ApplyCardStats : UnityEngine.Component
{
    public bool done;
    public Player[] Applied;
    public void Pick(int id, bool force, PickerType kind) { throw new Exception("Fixture signature only; native IL invoked separately."); }
    private void Start() { }
    private void OFFLINE_Pick(Player[] players)
    { Applied = players; foreach (var player in players) player.data.currentCards.Add(GetComponent<CardInfo>()); }
}
public class CardBar : UnityEngine.Component
{ public readonly List<CardInfo> Added = new(); public void AddCard(CardInfo card) { Added.Add(card); } }
public class CardBarHandler : UnityEngine.Component
{
    private CardBar[] cardBars;
    public CardBar[] FixtureBars { get => cardBars; set => cardBars = value; }
    public void AddCard(int id, CardInfo card) { throw new Exception("Fixture signature only; native IL invoked separately."); }
}
namespace RoundsPort.Runtime
{ static class Types { public static Type Find(string name) => typeof(CardBarHandler).Assembly.GetType(name); } }
namespace UnboundLib.Extensions
{ public static class CardBarHandlerExtensions { public static void Rebuild(CardBarHandler handler) { } } }
namespace UnboundLib.Cards
{
    public static class CardData
    {
        public static readonly Dictionary<int, List<string>> Cards = new();
        public static void AddCard(int playerId, string card)
        { if (!Cards.TryGetValue(playerId, out var owned)) Cards[playerId] = owned = new(); owned.Add(card); }
    }
}
namespace UnityEngine
{ public static class Debug { public static void Log(object value, Object context) { } } }
namespace Photon.Pun
{
    public enum RpcTarget { All }
    public static class PhotonNetwork { public static bool OfflineMode { get; set; } = true; }
    public class PhotonView : UnityEngine.Component
    { public int ControllerActorNr { get; set; } public void RPC(string name, RpcTarget target, object[] args) { throw new Exception("Online dispatch is outside fixture scope."); } }
}
namespace FriendlyFoe.Platforms { public enum Achievements { Placeholder } }
namespace FriendlyFoe.Platform
{ public static class PlatformManager { public static void UnlockAchievement(FriendlyFoe.Platforms.Achievements achievement) { } } }
namespace ModdingUtils.Utils
{
    public class Cards
    {
        public static Cards instance = new();
        public List<CardInfo> Candidates = new();
        public Player LastPlayer;
        public int EligibilityCalls, RandomCalls;
        public bool PlayerIsAllowedCard(Player player, CardInfo card)
        {
            EligibilityCalls++;
            if (player is null) throw new InvalidOperationException("Missing picker identity.");
            return !card.categories.Any(category => player.BlockedCategories.Contains(category))
                && !player.data.currentCards.Any(owned => owned.blacklistedCategories.Any(card.categories.Contains));
        }
        public CardInfo GetRandomCardWithCondition(Player player, Gun gun, GunAmmo ammo, CharacterData data,
            HealthHandler health, Gravity gravity, Block block, CharacterStatModifiers stats,
            Func<CardInfo, Player, Gun, GunAmmo, CharacterData, HealthHandler, Gravity, Block, CharacterStatModifiers, bool> condition,
            int maxAttempts)
        {
            RandomCalls++; LastPlayer = player;
            if (player is null) throw new InvalidOperationException("Missing picker identity.");
            foreach (var card in Candidates)
                if (condition(card, player, gun, ammo, player.data, health, gravity, block, stats)) return card;
            return null;
        }
    }
}
namespace CardChoiceSpawnUniqueCardPatch.CustomCategories
{
    public static class CustomCardCategories
    {
        public static CardCategory CanDrawMultipleCategory { get; } = new() { name = "CanDrawMultiple" };
    }
}
