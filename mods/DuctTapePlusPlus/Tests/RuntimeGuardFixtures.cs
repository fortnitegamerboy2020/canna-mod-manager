// Exercise the real guard and installed Harmony against harmless in-process API stubs.
// These fixtures do not load Unity, Photon transports, game DLLs or any real plugins.
using System;
using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Web.Script.Serialization;
using Canna.DuctTapePlusPlus;

static class RuntimeGuardFixtures
{
    static int passed;
    static void Check(bool condition, string name)
    { if (!condition) throw new Exception(name); Console.WriteLine("PASS " + name); passed++; }
    static void Call(object guard, string method) { guard.GetType().GetMethod(method, BindingFlags.NonPublic | BindingFlags.Instance).Invoke(guard, null); }
    static void Match(Photon.Realtime.Player remote)
    {
        var props = new ExitGames.Client.Photon.Hashtable();
        foreach (var row in Photon.Pun.PhotonNetwork.LocalPlayer.CustomProperties) props[row.Key] = row.Value;
        props["dtpp.actor"] = remote.ActorNumber;
        remote.SetCustomProperties(props);
    }
    static void Main()
    {
        string scratch = Path.Combine(Path.GetTempPath(), "canna-guard-fixture-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(scratch);
        BepInEx.Paths.PluginPath = Path.Combine(scratch, "plugins");
        BepInEx.Paths.PatcherPluginPath = Path.Combine(scratch, "patchers");
        BepInEx.Paths.ConfigPath = Path.Combine(scratch, "config");
        BepInEx.Paths.ManagedPath = Path.Combine(scratch, "managed");
        string plugins = Path.Combine(BepInEx.Paths.PluginPath, "Canna"), patchers = Path.Combine(BepInEx.Paths.PatcherPluginPath, "Canna");
        foreach (string directory in new[] { plugins, patchers, BepInEx.Paths.ConfigPath, BepInEx.Paths.ManagedPath }) Directory.CreateDirectory(directory);
        string copied = Path.Combine(plugins, "Fixture.dll"); File.Copy(Assembly.GetExecutingAssembly().Location, copied);
        string game = Path.Combine(BepInEx.Paths.ManagedPath, "Assembly-CSharp.dll"); File.WriteAllText(game, "fixture public game fingerprint");
        var manifest = new CompatibilityManifest { protocol = ManifestContract.Protocol, profile = ManifestContract.Profile,
            game_sha256 = ManifestContract.Hash(File.ReadAllBytes(game)), assemblies = ManifestContract.CollectAssemblies(plugins),
            files = ManifestContract.CollectFiles(plugins, patchers, BepInEx.Paths.ConfigPath) };
        manifest.digest = ManifestContract.Fingerprint(manifest);
        string manifestPath = Path.Combine(plugins, ManifestContract.ManifestRelativePath.Replace('/', Path.DirectorySeparatorChar));
        Directory.CreateDirectory(Path.GetDirectoryName(manifestPath));
        File.WriteAllText(manifestPath, new JavaScriptSerializer().Serialize(manifest));
        var guard = new NetworkGuard();
        var earlyWrapper = new HarmonyLib.Harmony("canna.fixture.early-wrapper");
        var lateWrapper = new HarmonyLib.Harmony("com.willis.rounds.unbound");
        try
        {
            var doStart = HarmonyLib.AccessTools.Method(typeof(GM_ArmsRace), "DoStartGame");
            earlyWrapper.Patch(doStart, postfix: new HarmonyLib.HarmonyMethod(typeof(RuntimeGuardFixtures), nameof(EarlyCoroutineWrapper)));
            Call(guard, "Awake"); Call(guard, "Start");
            // Same lowest priority as the guard, installed later: explicit owner ordering
            // must still place the final guard after Unbound's pass-through wrapper.
            var latePostfix = new HarmonyLib.HarmonyMethod(typeof(RuntimeGuardFixtures), nameof(LateCoroutineWrapper));
            latePostfix.priority = Int32.MinValue;
            lateWrapper.Patch(doStart, postfix: latePostfix);
            Check((BepInEx.Bootstrap.Chainloader.ManagerObject.hideFlags & UnityEngine.HideFlags.HideAndDontSave) == UnityEngine.HideFlags.HideAndDontSave,
                "Shared plugin manager receives BepInEx-equivalent startup protection");
            Check(Field(guard, "manifestReadStatus").Contains("assemblies=1; files=0; validation=passed"),
                "Prepared empty-file array has bounded successful schema diagnostics: " + Field(guard, "manifestReadStatus"));
            string validJson = File.ReadAllText(manifestPath);
            File.WriteAllText(manifestPath, validJson.Replace(ManifestContract.Protocol, "untrusted-marker-never-logged"));
            Call(guard, "ReadPreparedManifest");
            Check(Field(guard, "manifestReadStatus").Contains("stage=validation") &&
                Field(guard, "manifestReadStatus").Contains("Invalid compatibility manifest header or limits") &&
                !Field(guard, "manifestReadStatus").Contains("untrusted-marker"),
                "Malformed manifest exposes only a fixed validation reason, never raw payload");
            File.WriteAllText(manifestPath, validJson); Call(guard, "ReadPreparedManifest");
            guard.GetType().GetMethod("RefreshContent", BindingFlags.NonPublic | BindingFlags.Instance).Invoke(guard, new object[] { true });
            int hashes = HashPasses(guard);
            UnityEngine.Time.realtimeSinceStartup = 3f; Call(guard, "Update");
            Check(HashPasses(guard) == hashes, "Unchanged bounded metadata polling skips full content hashes");
            Photon.Pun.PhotonNetwork.OfflineMode = false;
            var native = new GM_ArmsRace();
            native.StartGame(); Check(Counters.NativeStarts == 0, "Real Harmony prefix blocks native start with no joined room");
            Check(!native.DoStartGame().MoveNext(), "Real Harmony prefix replaces denied native start coroutine");
            Check(Counters.EarlyCoroutineSteps == 0 && Counters.LateCoroutineSteps == 0 && Counters.NativeCoroutineFactories == 0 && Counters.NativeCoroutineSteps == 0,
                "Final denied coroutine gate prevents early and late pass-through wrapper side effects");
            new CharacterSelectionInstance().ReadyUp(); Check(Counters.Ready == 0, "Real Harmony prefix blocks readiness effects before mismatch");
            var host = new Photon.Realtime.Player { ActorNumber = 1 };
            var peer = new Photon.Realtime.Player { ActorNumber = 2 };
            Photon.Pun.PhotonNetwork.LocalPlayer = host;
            Photon.Pun.PhotonNetwork.PlayerList = new[] { host, peer };
            Photon.Pun.PhotonNetwork.CurrentRoom = new Photon.Realtime.Room();
            Photon.Pun.PhotonNetwork.InRoom = true; Photon.Pun.PhotonNetwork.IsMasterClient = true;
            guard.OnJoinedRoom();
            native.StartGame(); Check(Counters.NativeStarts == 0, "Native body stays blocked until every peer advertises");
            RWF.PrivateRoomHandler.StartGame(); Check(Counters.PrivateStarts == 0, "Real Harmony prefix blocks static RWF start entrypoint");
            Match(peer); native.StartGame(); Check(Counters.NativeStarts == 1, "Matching peer manifests permit native body");
            var yielded = new List<object>(); var allowedCoroutine = native.DoStartGame();
            while (allowedCoroutine.MoveNext()) yielded.Add(allowedCoroutine.Current);
            Check(yielded.SequenceEqual(new object[] { "late", "early", "started" }) && Counters.EarlyCoroutineSteps == 2 && Counters.LateCoroutineSteps == 2
                && Counters.NativeCoroutineFactories == 1 && Counters.NativeCoroutineSteps == 1,
                "Allowed native coroutine preserves both pass-through wrappers and original iterator work");
            Check(HashPasses(guard) > hashes, "Actual native gate forces a new full content hash");
            var rwf = new RWF.GameModes.GM_Deathmatch(); rwf.StartGame(); Check(Counters.RwfStarts == 1, "Known RWF inherited start body is permitted");
            new RWF.GameModes.UnknownMode().StartGame(); Check(Counters.RwfStarts == 1, "Unknown inherited RWF game mode cannot bypass base gate");
            var third = new Photon.Realtime.Player { ActorNumber = 3 };
            Photon.Pun.PhotonNetwork.PlayerList = new[] { host, peer, third }; guard.OnPlayerEnteredRoom(third);
            native.StartGame(); Check(Counters.NativeStarts == 1, "Late join immediately prevents native start body");
            Match(third); native.StartGame(); Check(Counters.NativeStarts == 2, "Compatible late join permits native start body");
            peer.SetCustomProperties(new ExitGames.Client.Photon.Hashtable { { "dtpp.digest", new string('f', 64) } });
            native.StartGame(); Check(Counters.NativeStarts == 2, "Changed peer digest prevents native body");
            var deniedCoroutine = native.DoStartGame();
            Match(peer);
            Check(!deniedCoroutine.MoveNext() && Counters.NativeCoroutineFactories == 1 && Counters.EarlyCoroutineSteps == 2 && Counters.LateCoroutineSteps == 2,
                "Coroutine denied at creation stays empty after peer evidence later recovers");
            File.WriteAllText(Path.Combine(BepInEx.Paths.ConfigPath, "new-generated.cfg"), "gameplay=changed");
            hashes = HashPasses(guard);
            UnityEngine.Time.realtimeSinceStartup += 3f; Call(guard, "Update");
            Check(HashPasses(guard) > hashes, "Changed gameplay config metadata triggers polling verification");
            native.StartGame(); Check(Counters.NativeStarts == 2, "New live gameplay config changes advertisement and blocks stale peers");
            Match(peer); Match(third); native.StartGame(); Check(Counters.NativeStarts == 3, "Matching live configuration digest restores start");
            string outside = Path.Combine(BepInEx.Paths.PluginPath, "manual.dll"); File.Copy(copied, outside);
            native.StartGame(); Check(Counters.NativeStarts == 3, "DLL outside Canna profile blocks actual native start");
            File.Delete(outside); Match(peer); Match(third);
            string asset = Path.Combine(plugins, "unexpected.bundle"); File.WriteAllText(asset, "unprepared asset");
            native.StartGame(); Check(Counters.NativeStarts == 3, "Unknown asset blocks actual native start");
            File.Delete(asset);
            BepInEx.Bootstrap.Chainloader.PluginInfos["external-loaded-plugin"] = new BepInEx.PluginInfo { Instance = guard };
            UnityEngine.Time.realtimeSinceStartup += 3f; Call(guard, "Update");
            native.StartGame(); Check(Counters.NativeStarts == 3, "Loaded plugin registry outside prepared root revokes readiness");
            BepInEx.Bootstrap.Chainloader.PluginInfos.Clear();
            string preserved = Path.Combine(BepInEx.Paths.ConfigPath, "new-generated.cfg");
            var stamp = File.GetLastWriteTimeUtc(preserved);
            int size = File.ReadAllText(preserved).Length;
            File.WriteAllText(preserved, new string('x', size)); File.SetLastWriteTimeUtc(preserved, stamp);
            native.StartGame(); Check(Counters.NativeStarts == 3, "Forced native gate detects changed config with preserved size and timestamp");
            Photon.Pun.PhotonNetwork.OfflineMode = true;
            native.StartGame(); Check(Counters.NativeStarts == 4, "Offline native start remains allowed without peer agreement");
            Photon.Pun.PhotonNetwork.OfflineMode = false;
            guard.OnLeftRoom(); Photon.Pun.PhotonNetwork.InRoom = false;
            native.StartGame(); Check(Counters.NativeStarts == 4, "Leaving room clears readiness and blocks old state");
            CheckAdaptedRuntimePrefixes();
            Console.WriteLine("Runtime-stub Harmony checks: " + passed + "; no game or multiplayer session was run.");
        }
        finally { Call(guard, "OnDestroy"); lateWrapper.UnpatchSelf(); earlyWrapper.UnpatchSelf(); Directory.Delete(scratch, true); }
    }
    static int HashPasses(object guard)
    { return (int)guard.GetType().GetField("fullHashPasses", BindingFlags.Instance | BindingFlags.NonPublic).GetValue(guard); }
    static string Field(object guard, string name)
    { return (string)guard.GetType().GetField(name, BindingFlags.Instance | BindingFlags.NonPublic).GetValue(guard); }

    static void CheckAdaptedRuntimePrefixes()
    {
        // Link the exact source normalized by the reviewed build, then install its actual
        // Harmony classes. Do not mirror the prefix implementation in these test bodies.
        var patches = new HarmonyLib.Harmony("canna.reviewed-runtime-fixtures");
        try
        {
            RoundsPort.Runtime.UL_UpdateNotice_Fix.Log = new BepInEx.Logging.ManualLogSource();
            patches.CreateClassProcessor(typeof(RoundsPort.Runtime.UL_HealthBarRespawns_Fix)).Patch();
            patches.CreateClassProcessor(typeof(RoundsPort.Runtime.UL_UpdateNotice_Fix)).Patch();
            UnboundLib.Patches.HealthBar_Patch_Update.Postfix(new HealthBar(), null);
            Check(Counters.HealthPostfixes == 0, "Adapted health prefix suppresses null CharacterData through actual Harmony");
            UnboundLib.Patches.HealthBar_Patch_Update.Postfix(new HealthBar(), new CharacterData());
            Check(Counters.HealthPostfixes == 0, "Adapted health prefix suppresses absent stats through actual Harmony");
            UnboundLib.Patches.HealthBar_Patch_Update.Postfix(new HealthBar(), new CharacterData { stats = new object() });
            Check(Counters.HealthPostfixes == 1, "Adapted health prefix preserves ordinary player stats through actual Harmony");
            var exact = new UnboundLib.Utils.UI.UpdateChecker.ModUpdateChecker { repoOwner = "Bknibb", repoName = "RoundsWithFriends", currentVersion = "3.0.10" };
            UnboundLib.Utils.UI.UpdateChecker.RegisterModUpdateChecker(exact);
            UnboundLib.Utils.UI.UpdateChecker.CreateUpdateMenu(exact);
            Check(Counters.UpdateRegistrations == 0 && Counters.UpdateMenus == 0, "Adapted update prefix skips only the exact bundled RWF version through Harmony");
            exact.currentVersion = "3.0.11"; CallUpdate(exact);
            Check(Counters.UpdateRegistrations == 1 && Counters.UpdateMenus == 1, "Other RWF versions keep both update-check bodies");
            exact.currentVersion = "3.0.10"; exact.repoOwner = "OtherOwner"; CallUpdate(exact);
            Check(Counters.UpdateRegistrations == 2 && Counters.UpdateMenus == 2, "Other repository owners keep both update-check bodies");
            exact.repoOwner = "Bknibb"; exact.repoName = "OtherMod"; CallUpdate(exact);
            Check(Counters.UpdateRegistrations == 3 && Counters.UpdateMenus == 3, "Other mods keep both update-check bodies");
            CallUpdate(null);
            Check(Counters.UpdateRegistrations == 4 && Counters.UpdateMenus == 4, "Null update checker preserves original bodies without prefix exceptions");
        }
        finally { patches.UnpatchSelf(); }
    }
    static void CallUpdate(UnboundLib.Utils.UI.UpdateChecker.ModUpdateChecker checker)
    { UnboundLib.Utils.UI.UpdateChecker.RegisterModUpdateChecker(checker); UnboundLib.Utils.UI.UpdateChecker.CreateUpdateMenu(checker); }
    static IEnumerator EarlyCoroutineWrapper(IEnumerator result)
    { Counters.EarlyCoroutineSteps++; yield return "early"; while (result.MoveNext()) yield return result.Current; Counters.EarlyCoroutineSteps++; }
    static IEnumerator LateCoroutineWrapper(IEnumerator result)
    { Counters.LateCoroutineSteps++; yield return "late"; while (result.MoveNext()) yield return result.Current; Counters.LateCoroutineSteps++; }
}

static class Counters {
    public static int NativeStarts, Ready, PrivateStarts, RwfStarts, HealthPostfixes, UpdateRegistrations, UpdateMenus;
    public static int EarlyCoroutineSteps, LateCoroutineSteps, NativeCoroutineFactories, NativeCoroutineSteps;
}
public class CharacterData { public object stats; }
public class HealthBar { }
namespace RoundsPort.Runtime
{ internal static class Types { internal static Type Find(string fullName) { return typeof(RuntimeGuardFixtures).Assembly.GetType(fullName, false); } } }
namespace BepInEx.Logging
{ public class ManualLogSource { public void LogInfo(object value) { } } }
namespace UnboundLib.Patches
{
    public static class HealthBar_Patch_Update
    {
        [MethodImpl(MethodImplOptions.NoInlining)] public static void Postfix(HealthBar __instance, CharacterData ___data)
        { Counters.HealthPostfixes++; if (___data == null || ___data.stats == null) throw new InvalidOperationException("The adapted prefix must suppress missing stats."); }
    }
}
namespace UnboundLib.Utils.UI
{
    public static class UpdateChecker
    {
        public sealed class ModUpdateChecker { public string repoOwner, repoName, currentVersion; }
        [MethodImpl(MethodImplOptions.NoInlining)] public static void RegisterModUpdateChecker(ModUpdateChecker checker) { Counters.UpdateRegistrations++; }
        [MethodImpl(MethodImplOptions.NoInlining)] public static void CreateUpdateMenu(ModUpdateChecker checker) { Counters.UpdateMenus++; }
    }
}
public class GM_ArmsRace
{
    [MethodImpl(MethodImplOptions.NoInlining)] public void StartGame() { Counters.NativeStarts++; }
    [MethodImpl(MethodImplOptions.NoInlining)] public IEnumerator DoStartGame() { Counters.NativeCoroutineFactories++; return Body(); }
    static IEnumerator Body() { Counters.NativeCoroutineSteps++; yield return "started"; }
}
public class CharacterSelectionInstance
{ [MethodImpl(MethodImplOptions.NoInlining)] public void ReadyUp() { Counters.Ready++; } }
namespace RWF
{
    public static class PrivateRoomHandler
    { [MethodImpl(MethodImplOptions.NoInlining)] public static void StartGame() { Counters.PrivateStarts++; } }
}
namespace RWF.GameModes
{
    public class RWFGameMode
    {
        [MethodImpl(MethodImplOptions.NoInlining)] public virtual void StartGame() { Counters.RwfStarts++; }
        [MethodImpl(MethodImplOptions.NoInlining)] public virtual IEnumerator DoStartGame() { return Body(); }
        static IEnumerator Body() { yield return "started"; }
    }
    public class GM_Deathmatch : RWFGameMode { }
    public class GM_TeamDeathmatch : RWFGameMode { }
    public class UnknownMode : RWFGameMode { }
}
namespace UnityEngine
{
    public class Object
    { public HideFlags hideFlags; public static implicit operator bool(Object obj) { return obj != null; } public static void DontDestroyOnLoad(Object obj) { } }
    public class GameObject : Object { }
    [Flags] public enum HideFlags { None = 0, HideAndDontSave = 61 }
    public class MonoBehaviour : Object { }
    public static class Time { public static float realtimeSinceStartup; }
    public struct Rect { public Rect(float x, float y, float w, float h) { } }
    public static class GUI { public static void Box(Rect rect, string text) { } public static void Label(Rect rect, string text) { } }
    public static class JsonUtility
    { public static T FromJson<T>(string text) { return new JavaScriptSerializer().Deserialize<T>(text); } }
}
namespace BepInEx
{
    [AttributeUsage(AttributeTargets.Class)] public class BepInPlugin : Attribute { public BepInPlugin(string id, string name, string version) { } }
    [AttributeUsage(AttributeTargets.Class, AllowMultiple = true)] public class BepInDependency : Attribute
    { public enum DependencyFlags { SoftDependency } public BepInDependency(string id) { } public BepInDependency(string id, DependencyFlags flags) { } }
    public class BaseUnityPlugin : UnityEngine.MonoBehaviour { public readonly Log Logger = new Log(); }
    public class Log { public void LogError(object message) { Console.WriteLine("LOG " + message); } public void LogWarning(object message) { } }
    public static class Paths { public static string PluginPath, ConfigPath, ManagedPath, PatcherPluginPath; }
    public class PluginInfo { public BaseUnityPlugin Instance; }
}
namespace BepInEx.Bootstrap
{ public static class Chainloader { public static Dictionary<string, BepInEx.PluginInfo> PluginInfos = new Dictionary<string, BepInEx.PluginInfo>(); public static UnityEngine.GameObject ManagerObject = new UnityEngine.GameObject(); } }
namespace ExitGames.Client.Photon
{
    public class Hashtable : Dictionary<object, object>
    { public new object this[object key] { get { object value; return TryGetValue(key, out value) ? value : null; } set { base[key] = value; } } }
}
namespace Photon.Pun
{
    public static class PhotonNetwork
    {
        public static bool InRoom, OfflineMode, IsMasterClient;
        public static Photon.Realtime.Player LocalPlayer;
        public static Photon.Realtime.Player[] PlayerList = new Photon.Realtime.Player[0];
        public static Photon.Realtime.Room CurrentRoom;
        public static readonly List<object> Callbacks = new List<object>();
        public static void AddCallbackTarget(object target) { Callbacks.Add(target); }
        public static void RemoveCallbackTarget(object target) { Callbacks.Remove(target); }
    }
}
namespace Photon.Realtime
{
    using ExitGames.Client.Photon;
    public class Player
    {
        public int ActorNumber;
        public readonly Hashtable CustomProperties = new Hashtable();
        public bool SetCustomProperties(Hashtable props)
        {
            foreach (var row in props) if (row.Value == null) CustomProperties.Remove(row.Key); else CustomProperties[row.Key] = row.Value;
            foreach (var callback in Photon.Pun.PhotonNetwork.Callbacks.OfType<IInRoomCallbacks>().ToArray()) callback.OnPlayerPropertiesUpdate(this, props);
            return true;
        }
    }
    public class Room
    {
        public readonly Hashtable CustomProperties = new Hashtable();
        public bool SetCustomProperties(Hashtable props)
        {
            foreach (var row in props) CustomProperties[row.Key] = row.Value;
            foreach (var callback in Photon.Pun.PhotonNetwork.Callbacks.OfType<IInRoomCallbacks>().ToArray()) callback.OnRoomPropertiesUpdate(props);
            return true;
        }
    }
    public class FriendInfo { }
    public class RegionHandler { }
    public enum DisconnectCause { Unknown }
    public interface IInRoomCallbacks
    { void OnPlayerEnteredRoom(Player player); void OnPlayerLeftRoom(Player player); void OnPlayerPropertiesUpdate(Player player, Hashtable props); void OnRoomPropertiesUpdate(Hashtable props); void OnMasterClientSwitched(Player player); }
    public interface IMatchmakingCallbacks
    { void OnFriendListUpdate(List<FriendInfo> friends); void OnCreatedRoom(); void OnCreateRoomFailed(short code, string message); void OnJoinedRoom(); void OnJoinRoomFailed(short code, string message); void OnJoinRandomFailed(short code, string message); void OnLeftRoom(); }
    public interface IConnectionCallbacks
    { void OnConnected(); void OnConnectedToMaster(); void OnDisconnected(DisconnectCause cause); void OnRegionListReceived(RegionHandler regions); void OnCustomAuthenticationResponse(Dictionary<string, object> data); void OnCustomAuthenticationFailed(string message); }
}
