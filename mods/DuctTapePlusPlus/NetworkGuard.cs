// Canna MIT license. Offline-tested compatibility preflight; live multiplayer is unverified.
using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
#if GUARD_RUNTIME
using System.Collections;
using System.IO;
using System.Reflection;
using BepInEx;
using BepInEx.Configuration;
using HarmonyLib;
using Photon.Pun;
using Photon.Realtime;
using UnityEngine;
using PhotonPlayer = Photon.Realtime.Player;
using PhotonHashtable = ExitGames.Client.Photon.Hashtable;
#endif

namespace Canna.DuctTapePlusPlus
{
    public sealed class PeerAdvertisement
    {
        public string Protocol, Profile, GameHash, ContentDigest, Epoch;
        public string ModsDigest, AssetsDigest, ConfigDigest;
        public int Actor;

        public PeerAdvertisement(string protocol, string profile, string gameHash, string digest, int actor, string epoch,
            string mods = null, string assets = null, string config = null)
        { Protocol = protocol; Profile = profile; GameHash = gameHash; ContentDigest = digest; Actor = actor; Epoch = epoch;
          ModsDigest = mods; AssetsDigest = assets; ConfigDigest = config; }

        public static bool IsHash(string value)
        { return value != null && value.Length == 64 && value.All(c => c >= '0' && c <= '9' || c >= 'a' && c <= 'f'); }
        public static bool IsEpoch(string value)
        { return value != null && value.Length == 32 && value.All(c => c >= '0' && c <= '9' || c >= 'a' && c <= 'f'); }
    }

    public struct GuardVerdict
    {
        public readonly bool Allowed;
        public readonly string Reason;
        public GuardVerdict(bool allowed, string reason) { Allowed = allowed; Reason = reason; }
    }

    public sealed class ContentStampPolicy
    {
        string lastAttempt;
        public bool NeedsVerification(string currentStamp, bool force)
        { return force || currentStamp == null || lastAttempt != currentStamp; }
        public void RecordAttempt(string stamp) { lastAttempt = stamp; }
        public void Invalidate() { lastAttempt = null; }
    }

    // No Unity, Photon or filesystem dependencies: callbacks supply the actual Photon actor ID.
    public sealed class RoomCompatibilityPolicy
    {
        public const string Protocol = ManifestContract.Protocol;
        public const string Profile = ManifestContract.Profile;
        readonly Dictionary<int, PeerAdvertisement> peers = new Dictionary<int, PeerAdvertisement>();
        readonly HashSet<int> actors = new HashSet<int>();
        PeerAdvertisement local;
        string localError = "Compatibility manifest has not been validated.";
        string rosterError = "Room membership has not been observed.";
        string epoch;
        int localActor;
        public int Generation { get; private set; }

        public void ConfigureLocal(PeerAdvertisement expected, string error = null)
        {
            local = expected;
            localError = error;
            if (String.IsNullOrEmpty(localError) && (local == null || local.Protocol != Protocol || local.Profile != Profile ||
                !PeerAdvertisement.IsHash(local.GameHash) || !PeerAdvertisement.IsHash(local.ContentDigest)))
                localError = "Local compatibility manifest is malformed or unsupported.";
        }

        public void BeginRoom(int actualLocalActor, string roomEpoch)
        {
            Clear();
            localActor = actualLocalActor;
            epoch = roomEpoch;
        }

        public void Clear()
        {
            Generation++;
            peers.Clear(); actors.Clear(); epoch = null; localActor = 0;
            rosterError = "Room membership has not been observed.";
        }

        public void ReplaceRoster(IEnumerable<int> actualActors)
        {
            actors.Clear(); rosterError = null;
            if (actualActors == null) { rosterError = "Room membership is unavailable."; return; }
            foreach (int actor in actualActors)
                if (actor <= 0 || !actors.Add(actor)) rosterError = "Room membership is malformed.";
            if (localActor <= 0 || !actors.Contains(localActor)) rosterError = "Local actor is absent from the room membership.";
            foreach (int stale in peers.Keys.Where(k => !actors.Contains(k)).ToArray()) peers.Remove(stale);
        }

        public bool ObservePeer(int actualSenderActor, PeerAdvertisement advertised, int observedGeneration)
        {
            if (observedGeneration != Generation || !actors.Contains(actualSenderActor)) return false;
            // Never index by a peer-supplied actor field, nor let a remote payload replace local evidence.
            peers[actualSenderActor] = advertised;
            return true;
        }

        public GuardVerdict Evaluate()
        {
            if (!String.IsNullOrEmpty(localError)) return new GuardVerdict(false, localError);
            if (!String.IsNullOrEmpty(rosterError)) return new GuardVerdict(false, rosterError);
            if (!PeerAdvertisement.IsEpoch(epoch)) return new GuardVerdict(false, "Waiting for this room's Canna Rebound compatibility session.");
            foreach (int actor in actors.OrderBy(n => n))
            {
                PeerAdvertisement peer;
                if (!peers.TryGetValue(actor, out peer) || peer == null)
                    return new GuardVerdict(false, "Actor " + actor + " has not advertised Canna Rebound compatibility.");
                if (peer.Actor != actor) return new GuardVerdict(false, "Actor " + actor + " has invalid actor binding.");
                if (peer.Epoch != epoch) return new GuardVerdict(false, "Actor " + actor + " has stale compatibility evidence.");
                if (peer.Protocol != local.Protocol) return new GuardVerdict(false, "Actor " + actor + " has a different Canna Rebound protocol.");
                if (peer.Profile != local.Profile) return new GuardVerdict(false, "Actor " + actor + " has a different compatibility profile.");
                if (!PeerAdvertisement.IsHash(peer.GameHash) || peer.GameHash != local.GameHash)
                    return new GuardVerdict(false, "Actor " + actor + " has a different or malformed game assembly hash.");
                if (!PeerAdvertisement.IsHash(peer.ContentDigest) || peer.ContentDigest != local.ContentDigest)
                {
                    if (PeerAdvertisement.IsHash(local.ModsDigest) && PeerAdvertisement.IsHash(peer.ModsDigest) && local.ModsDigest != peer.ModsDigest)
                        return new GuardVerdict(false, "Actor " + actor + " has different prepared mod DLLs or a different Rebound release. Update Canna and reapply the same pack on both PCs.");
                    if (PeerAdvertisement.IsHash(local.AssetsDigest) && PeerAdvertisement.IsHash(peer.AssetsDigest) && local.AssetsDigest != peer.AssetsDigest)
                        return new GuardVerdict(false, "Actor " + actor + " has different mod assets or patchers. Reapply the same pack on both PCs.");
                    if (PeerAdvertisement.IsHash(local.ConfigDigest) && PeerAdvertisement.IsHash(peer.ConfigDigest) && local.ConfigDigest != peer.ConfigDigest)
                        return new GuardVerdict(false, "Actor " + actor + " has different active gameplay settings. Match enabled cards, maps and mod settings on both PCs.");
                    return new GuardVerdict(false, "Actor " + actor + " has different mods, assets or gameplay configuration.");
                }
            }
            return new GuardVerdict(true, "All current actors have matching Canna Rebound compatibility manifests.");
        }
    }
}

#if GUARD_RUNTIME
namespace Canna.DuctTapePlusPlus
{
    [BepInPlugin("canna.ducttapeplusplus.networkguard", "Canna Rebound compatibility guard", "0.1.1")]
    [BepInDependency("rounds-port.runtime")]
    [BepInDependency("com.willis.rounds.unbound", BepInDependency.DependencyFlags.SoftDependency)]
    [BepInDependency("io.olavim.rounds.rwf", BepInDependency.DependencyFlags.SoftDependency)]
    public sealed class NetworkGuard : BaseUnityPlugin, IInRoomCallbacks, IMatchmakingCallbacks, IConnectionCallbacks
    {
        const string EpochKey = "dtpp.room.v1";
        const string ProtocolKey = "dtpp.protocol", ProfileKey = "dtpp.profile", GameKey = "dtpp.game";
        const string DigestKey = "dtpp.digest", ActorKey = "dtpp.actor", PeerEpochKey = "dtpp.epoch";
        const string ModsKey = "dtpp.mods", AssetsKey = "dtpp.assets", ConfigKey = "dtpp.config";
        static NetworkGuard instance;
        readonly RoomCompatibilityPolicy policy = new RoomCompatibilityPolicy();
        readonly HashSet<MethodBase> patched = new HashSet<MethodBase>();
        readonly HashSet<MethodBase> unsupported = new HashSet<MethodBase>();
        readonly HashSet<Assembly> scanned = new HashSet<Assembly>();
        readonly ContentStampPolicy contentStamps = new ContentStampPolicy();
        int fullHashPasses;
        Harmony harmony;
        CompatibilityManifest prepared;
        PeerAdvertisement local;
        string epoch, pendingEpoch, failedEpoch, contentError, patchError, lastMessage;
        string manifestReadPath, manifestReadStatus;
        bool joined;
        float nextContentCheck, nextModeCheck;
        PendingStart pendingStart;
        sealed class PendingStart { public MethodBase Method; public object Target; public object[] Args; }
        static string PreparedPlugins { get { return Path.Combine(Paths.PluginPath, "Canna"); } }
        static string PreparedPatchers { get { return Path.Combine(Paths.PatcherPluginPath, "Canna"); } }

        void Awake()
        {
            instance = this;
            harmony = new Harmony("canna.ducttapeplusplus.networkguard");
            // Install gates even when the manifest fails; an invalid profile never permits online start.
            try
            {
                // Same flags as BepInEx's HideManagerGameObject=true, applied before
                // the first game scene cleanup; no persistent core config rewrite.
                var manager = BepInEx.Bootstrap.Chainloader.ManagerObject;
                if (!manager) throw new InvalidOperationException("The shared plugin manager is unavailable.");
                manager.hideFlags |= HideFlags.HideAndDontSave;
                // Chainloader already called DontDestroyOnLoad before attaching plugins.
                PatchRequired(typeof(GM_ArmsRace), "StartGame", Type.EmptyTypes, false);
                PatchRequired(typeof(GM_ArmsRace), "DoStartGame", Type.EmptyTypes, true);
                PatchRequired(typeof(CharacterSelectionInstance), "ReadyUp", Type.EmptyTypes, false);
            }
            catch (Exception ex) {
                patchError = "Native multiplayer gate installation failed: " + ex.GetType().Name; Logger.LogError(patchError);
#if GUARD_FIXTURE
                Logger.LogError(ex.ToString());
#endif
            }
            ReadPreparedManifest();
            RefreshContent(true);
            PhotonNetwork.AddCallbackTarget(this);
        }

        void Start() { ScanModeGates(); }

        void Update()
        {
            if (Time.realtimeSinceStartup >= nextModeCheck)
            { nextModeCheck = Time.realtimeSinceStartup + 1f; ScanModeGates(); }
            if (Time.realtimeSinceStartup >= nextContentCheck)
            { nextContentCheck = Time.realtimeSinceStartup + 2f; RefreshContent(false); }
            if (!PhotonNetwork.InRoom || PhotonNetwork.OfflineMode) return;
            if (!joined) JoinCurrentRoom();
            RefreshRoom();
            // Resume only a start that the game already requested, after every peer becomes compatible.
            if (pendingStart != null && Evaluate().Allowed)
            {
                var request = pendingStart; pendingStart = null;
                try
                {
                    var targetObject = request.Target as UnityEngine.Object;
                    if (request.Target != null && targetObject != null && !targetObject) return;
                    request.Method.Invoke(request.Target, request.Args);
                }
                catch (Exception ex) { Logger.LogError("Deferred match start failed: " + ex.GetType().Name); }
            }
        }

        void ReadPreparedManifest()
        {
            string stage = "path", validationReason = null;
            manifestReadStatus = null;
            try
            {
                manifestReadPath = Path.GetFullPath(Path.Combine(PreparedPlugins, ManifestContract.ManifestRelativePath.Replace('/', Path.DirectorySeparatorChar)));
                ManifestContract.RejectLinks(manifestReadPath);
                stage = "size";
                if (!File.Exists(manifestReadPath) || new FileInfo(manifestReadPath).Length > 8 * 1024 * 1024)
                    throw new InvalidDataException("A prepared compatibility manifest is required.");
                stage = "read";
                byte[] bytes = File.ReadAllBytes(manifestReadPath);
                if (bytes.Length > ManifestJson.MaximumTextLength) throw new InvalidDataException("Prepared manifest exceeds limits.");
                string json = new UTF8Encoding(false, true).GetString(bytes);
                stage = "parse";
                prepared = ManifestJson.Parse(json);
                stage = "validation";
                // Own fixed labels and counts only: no payload, file names or arbitrary exception text.
                manifestReadStatus = "object=" + (prepared == null ? "null" : "present")
                    + "; protocol=" + (prepared != null && prepared.protocol == ManifestContract.Protocol ? "match" : "mismatch")
                    + "; profile=" + (prepared != null && prepared.profile == ManifestContract.Profile ? "match" : "mismatch")
                    + "; game-hash=" + (prepared != null && ManifestContract.IsHash(prepared.game_sha256) ? "valid" : "invalid")
                    + "; digest=" + (prepared != null && ManifestContract.IsHash(prepared.digest) ? "valid" : "invalid")
                    + "; assemblies=" + (prepared == null || prepared.assemblies == null ? "null" : prepared.assemblies.Length.ToString(System.Globalization.CultureInfo.InvariantCulture))
                    + "; files=" + (prepared == null || prepared.files == null ? "null" : prepared.files.Length.ToString(System.Globalization.CultureInfo.InvariantCulture));
                if (!ManifestContract.Validate(prepared, out validationReason)) throw new InvalidDataException("Prepared manifest validation failed.");
                manifestReadStatus += "; validation=passed";
                contentError = null;
            }
            catch (Exception ex)
            {
                prepared = null;
                // Validate returns fixed contract reasons. Never include ex.Message, which may contain paths or source data.
                var decodeError = ex as ManifestDecodeException;
                if (decodeError != null) validationReason = decodeError.Reason;
                manifestReadStatus = "stage=" + stage + "; " + (manifestReadStatus ?? "object=not-read")
                    + "; reason=" + (validationReason ?? ex.GetType().Name);
                contentError = "Prepared compatibility manifest is missing or invalid at " + stage + " ("
                    + (validationReason ?? ex.GetType().Name) + "). Run Canna Rebound preflight again.";
            }
        }

        void RefreshContent(bool force)
        {
            string previousDigest = local == null ? null : local.ContentDigest;
            string stamp = null;
            try
            {
                if (prepared == null) { policy.ConfigureLocal(null, contentError); return; }
                stamp = ReadContentStamp();
                if (!contentStamps.NeedsVerification(stamp, force)) return;
                fullHashPasses++;
                string game = ManifestContract.Hash(File.ReadAllBytes(Path.Combine(Paths.ManagedPath, "Assembly-CSharp.dll")));
                RejectOutsideContent(Paths.PluginPath, PreparedPlugins);
                RejectOutsideContent(Paths.PatcherPluginPath, PreparedPatchers);
                var current = ManifestContract.CreateRuntimeManifest(prepared, PreparedPlugins, PreparedPatchers, Paths.ConfigPath);
                ApplyRuntimeConfig(current);
                current.game_sha256 = game; current.digest = ManifestContract.Fingerprint(current);
                string error;
                if (!ManifestContract.VerifyImmutable(prepared, current, out error)) throw new InvalidDataException(error);
                CheckLoadedPlugins(current);
                local = new PeerAdvertisement(current.protocol, current.profile, game, current.digest, 0, null,
                    ManifestContract.ComponentFingerprint(current, "mods"), ManifestContract.ComponentFingerprint(current, "assets"),
                    ManifestContract.ComponentFingerprint(current, "config"));
                contentError = null;
                policy.ConfigureLocal(local, patchError);
            }
            catch (Exception ex)
            {
                local = null;
                contentError = "Installed mods, assets or patchers do not match the prepared profile (" + ex.GetType().Name + "). Run Canna Rebound preflight again.";
                policy.ConfigureLocal(null, contentError);
            }
            if (stamp == null) contentStamps.Invalidate();
            else contentStamps.RecordAttempt(stamp);
            if (joined && (local == null || previousDigest != local.ContentDigest)) PublishLocal();
        }

        void ApplyRuntimeConfig(CompatibilityManifest current)
        {
            var bound = new Dictionary<string, ConfigFile>(StringComparer.OrdinalIgnoreCase);
            foreach (var info in BepInEx.Bootstrap.Chainloader.PluginInfos.Values)
            {
                if (info.Instance == null) continue;
                var standard = info.Instance.Config;
                if (standard != null) bound[ManifestContract.Relative(Paths.ConfigPath, standard.ConfigFilePath)] = standard;
                // The pinned Unbound port uses its public, already-initialized custom ConfigFile.
                if (info.Instance.GetType().Assembly.GetName().Name == "UnboundLib")
                {
                    var field = info.Instance.GetType().GetField("config", BindingFlags.Public | BindingFlags.Static);
                    if (field != null && field.FieldType == typeof(ConfigFile))
                    {
                        var config = field.GetValue(null) as ConfigFile;
                        if (config != null) bound[ManifestContract.Relative(Paths.ConfigPath, config.ConfigFilePath)] = config;
                    }
                }
            }
            var rows = current.files.Where(r => r.root != "config").ToList();
            foreach (var row in current.files.Where(r => r.root == "config"))
            {
                // These known plugin-owned configs cannot affect a pack without their owner.
                if ((row.path == "fr.flofl.rounds.hollowpurple.cfg" && !BepInEx.Bootstrap.Chainloader.PluginInfos.ContainsKey("fr.flofl.rounds.hollowpurple"))
                    || (row.path == "local.rounds.cardcontrol.cfg" && !BepInEx.Bootstrap.Chainloader.PluginInfos.ContainsKey("local.rounds.cardcontrol"))) continue;
                if (bound.ContainsKey(row.path)) continue;
                if (row.path.EndsWith(".cfg", StringComparison.OrdinalIgnoreCase))
                    row.sha256 = ManifestContract.ConfigHash(File.ReadAllBytes(Path.Combine(Paths.ConfigPath, row.path.Replace('/', Path.DirectorySeparatorChar))));
                rows.Add(row);
            }
            foreach (var entry in bound)
            {
                var values = new List<KeyValuePair<string, string>>();
                foreach (var key in entry.Value.Keys)
                {
                    if (entry.Key == "UnboundLib.cfg" && key.Section == "Config Options" && key.Key == "LockMouse") continue;
                    string identity = key.Section.Length + ":" + key.Section + key.Key.Length + ":" + key.Key;
                    values.Add(new KeyValuePair<string, string>(identity, entry.Value[key].GetSerializedValue()));
                }
                if (values.Count != 0) rows.Add(new FileRow { root = "config", path = entry.Key, sha256 = ManifestContract.ConfigValuesHash(values) });
            }
            current.files = rows.ToArray();
        }

        string ReadContentStamp()
        {
            var text = new StringBuilder();
            int count = 0;
            foreach (var group in new[] { new[] { "plugins", Paths.PluginPath }, new[] { "patchers", Paths.PatcherPluginPath }, new[] { "config", Paths.ConfigPath } })
                foreach (var file in ManifestContract.ReadFiles(group[1]))
                {
                    if (++count > 16384) throw new InvalidDataException("Installed content stamp file count exceeds limits.");
                    string relative = ManifestContract.Relative(group[1], file);
                    if (group[0] == "config" && relative.Equals("BepInEx.cfg", StringComparison.OrdinalIgnoreCase)) continue;
                    AddFileStamp(text, group[0] + "/" + relative, file);
                }
            string game = Path.Combine(Paths.ManagedPath, "Assembly-CSharp.dll");
            ManifestContract.RejectLinks(game);
            AddFileStamp(text, "game/Assembly-CSharp.dll", game);
            foreach (var entry in BepInEx.Bootstrap.Chainloader.PluginInfos.OrderBy(p => p.Key, StringComparer.Ordinal))
            {
                if (entry.Value.Instance == null) continue;
                var assembly = entry.Value.Instance.GetType().Assembly;
                if (assembly.IsDynamic || String.IsNullOrEmpty(assembly.Location)) throw new InvalidDataException("A loaded plugin has no prepared physical assembly.");
                AddStampAtom(text, "loaded"); AddStampAtom(text, entry.Key); AddStampAtom(text, assembly.GetName().FullName);
                AddStampAtom(text, ManifestContract.Relative(PreparedPlugins, assembly.Location));
            }
            return ManifestContract.Hash(Encoding.UTF8.GetBytes(text.ToString()));
        }

        static void AddFileStamp(StringBuilder text, string key, string path)
        {
            var info = new FileInfo(path);
            if (!info.Exists || info.Length < 0) throw new InvalidDataException("Installed content stamp could not be read.");
            AddStampAtom(text, key); AddStampAtom(text, info.Length.ToString(System.Globalization.CultureInfo.InvariantCulture));
            AddStampAtom(text, info.LastWriteTimeUtc.Ticks.ToString(System.Globalization.CultureInfo.InvariantCulture));
        }

        static void AddStampAtom(StringBuilder text, string atom)
        {
            if (atom == null || atom.Length > 8192 || text.Length > 8 * 1024 * 1024)
                throw new InvalidDataException("Installed content stamp exceeds limits.");
            text.Append(atom.Length).Append(':').Append(atom);
        }

        static void RejectOutsideContent(string root, string preparedRoot)
        {
            var prefix = Path.GetFullPath(preparedRoot).TrimEnd('\\', '/') + Path.DirectorySeparatorChar;
            foreach (var file in ManifestContract.ReadFiles(root))
                if (!Path.GetFullPath(file).StartsWith(prefix, StringComparison.OrdinalIgnoreCase))
                    throw new InvalidDataException("Content outside the prepared Canna profile is present.");
        }

        void CheckLoadedPlugins(CompatibilityManifest current)
        {
            var identities = new HashSet<string>(current.assemblies.Select(a => a.identity), StringComparer.Ordinal);
            foreach (var plugin in BepInEx.Bootstrap.Chainloader.PluginInfos.Values)
            {
                if (plugin.Instance == null) continue;
                var assembly = plugin.Instance.GetType().Assembly;
                if (assembly.IsDynamic || String.IsNullOrEmpty(assembly.Location) || !identities.Contains(assembly.GetName().FullName))
                    throw new InvalidDataException("A loaded plugin is outside the prepared manifest.");
                string relative = ManifestContract.Relative(PreparedPlugins, assembly.Location);
                if (String.IsNullOrEmpty(relative)) throw new InvalidDataException("A loaded plugin is outside the prepared plugin root.");
                var expected = current.assemblies.Single(a => a.identity == assembly.GetName().FullName);
                if (ManifestContract.Hash(File.ReadAllBytes(assembly.Location)) != expected.sha256)
                    throw new InvalidDataException("A loaded plugin payload has changed.");
            }
        }

        void PatchRequired(Type type, string name, Type[] args, bool coroutine)
        {
            var method = AccessTools.Method(type, name, args);
            if (method == null || method.IsAbstract || method.ContainsGenericParameters ||
                method.ReturnType != (coroutine ? typeof(IEnumerator) : typeof(void)))
                throw new MissingMethodException(type.FullName, name);
            Patch(method, coroutine, false);
        }

        void Patch(MethodBase method, bool coroutine, bool unsupportedMode)
        {
            if (patched.Contains(method)) { if (unsupportedMode) unsupported.Add(method); return; }
            var prefix = new HarmonyMethod(typeof(NetworkGuard), coroutine ? nameof(CoroutineGate) : nameof(VoidGate));
            prefix.priority = Int32.MaxValue;
            HarmonyMethod postfix = null;
            if (coroutine)
            {
                // Unbound uses a pass-through iterator postfix, which runs after ref-result
                // postfixes. Reassert denial in that same final stage, after its wrappers.
                postfix = new HarmonyMethod(typeof(NetworkGuard), nameof(CoroutineResultGate));
                postfix.priority = Int32.MinValue;
                var existing = Harmony.GetPatchInfo(method);
                postfix.after = (existing == null ? Enumerable.Empty<string>() : existing.Postfixes.Select(p => p.owner))
                    .Concat(new[] { "com.willis.rounds.unbound", "io.olavim.rounds.rwf" })
                    .Where(owner => owner != harmony.Id).Distinct(StringComparer.Ordinal).ToArray();
            }
            harmony.Patch(method, prefix: prefix, postfix: postfix);
            patched.Add(method);
            if (unsupportedMode) unsupported.Add(method);
        }

        static bool IsSupportedHandler(Type type)
        {
            return type.FullName == "UnboundLib.GameModes.ArmsRaceHandler" ||
                type.FullName == "UnboundLib.GameModes.SandboxHandler" ||
                type.FullName == "RWF.GameModes.DeathmatchHandler" ||
                type.FullName == "RWF.GameModes.TeamDeathmatchHandler";
        }

        void ScanModeGates()
        {
            try
            {
                foreach (var assembly in AppDomain.CurrentDomain.GetAssemblies())
                {
                    if (!scanned.Add(assembly)) continue;
                    Type[] types;
                    try { types = assembly.GetTypes(); }
                    catch (ReflectionTypeLoadException ex) { types = ex.Types.Where(t => t != null).ToArray(); }
                    foreach (var type in types)
                    {
                        if (type.ContainsGenericParameters || type.IsAbstract || type.IsInterface) continue;
                        foreach (var contract in type.GetInterfaces().Where(i => i.FullName == "UnboundLib.GameModes.IGameModeHandler" && i.Assembly.GetName().Name == "UnboundLib"))
                        {
                            var map = type.GetInterfaceMap(contract);
                            for (int i = 0; i < map.InterfaceMethods.Length; i++)
                                if (map.InterfaceMethods[i].Name == "StartGame" && map.TargetMethods[i].GetParameters().Length == 0)
                                    // Shared inherited methods must be judged by the actual instance,
                                    // otherwise an unsupported subclass would also disable a supported mode.
                                    Patch(map.TargetMethods[i], false, false);
                        }
                    }
                    var rwf = assembly.GetType("RWF.GameModes.RWFGameMode", false);
                    if (rwf != null)
                    {
                        PatchRequired(rwf, "StartGame", Type.EmptyTypes, false);
                        PatchRequired(rwf, "DoStartGame", Type.EmptyTypes, true);
                        // Overrides can bypass the base implementation; deny unrecognized online modes.
                        foreach (var type in types.Where(t => t != null && t.IsSubclassOf(rwf) && !t.ContainsGenericParameters))
                            foreach (var name in new[] { "StartGame", "DoStartGame" })
                            {
                                var method = type.GetMethod(name, BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance | BindingFlags.DeclaredOnly);
                                if (method != null) Patch(method, name == "DoStartGame", type.FullName != "RWF.GameModes.GM_Deathmatch" && type.FullName != "RWF.GameModes.GM_TeamDeathmatch");
                            }
                    }
                    var room = assembly.GetType("RWF.PrivateRoomHandler", false);
                    if (room != null) PatchRequired(room, "StartGame", Type.EmptyTypes, false);
                }
            }
            catch (Exception ex)
            { patchError = "A multiplayer game-mode gate is unsupported (" + ex.GetType().Name + ")."; policy.ConfigureLocal(local, patchError); Logger.LogError(patchError); }
        }

        GuardVerdict Evaluate()
        {
            if (!String.IsNullOrEmpty(patchError)) return new GuardVerdict(false, patchError);
            return policy.Evaluate();
        }

        bool Gate(MethodBase method, object target, bool coroutine)
        {
            if (PhotonNetwork.OfflineMode) return true;
            if (!PhotonNetwork.InRoom) return Deny("Online start requires a joined room and matching Canna Rebound peers.");
            if (!joined) JoinCurrentRoom();
            // Metadata stamps are a polling optimization. Every actual start/readiness
            // gate hashes content again, even when size and timestamps were preserved.
            RefreshContent(true); RefreshRoom();
            if (unsupported.Contains(method)) return Deny("This online game mode is outside the Canna Rebound preview's supported start gates.");
            if (target != null)
            {
                var type = target.GetType();
                if (type.GetInterfaces().Any(i => i.FullName == "UnboundLib.GameModes.IGameModeHandler" && i.Assembly.GetName().Name == "UnboundLib") && !IsSupportedHandler(type))
                    return Deny("This online game-mode handler is outside the supported preview profile.");
                for (var parent = type; parent != null; parent = parent.BaseType)
                    if (parent.FullName == "RWF.GameModes.RWFGameMode" && type.FullName != "RWF.GameModes.GM_Deathmatch" && type.FullName != "RWF.GameModes.GM_TeamDeathmatch")
                        return Deny("This online RWF game mode is outside the supported preview profile.");
            }
            var verdict = Evaluate();
            if (verdict.Allowed) return true;
            // Readiness remains a user action. Only actual zero-argument start requests are deferred.
            if (!coroutine && method.Name == "StartGame" && method.GetParameters().Length == 0)
                pendingStart = new PendingStart { Method = method, Target = target, Args = null };
            return Deny(verdict.Reason);
        }

        bool Deny(string reason)
        {
            if (lastMessage != reason) { lastMessage = reason; Logger.LogWarning("Canna Rebound blocked online start: " + reason); }
            return false;
        }

        static bool VoidGate(MethodBase __originalMethod, object __instance)
        { return instance != null && instance.Gate(__originalMethod, __instance, false); }

        static bool CoroutineGate(MethodBase __originalMethod, object __instance, ref IEnumerator __result, out bool __state)
        {
            __state = instance == null || !instance.Gate(__originalMethod, __instance, true);
            if (!__state) return true;
            __result = EmptyCoroutine(); return false;
        }

        static IEnumerator CoroutineResultGate(IEnumerator result, bool __state)
        { return __state ? EmptyCoroutine() : result; }

        static IEnumerator EmptyCoroutine() { yield break; }

        void JoinCurrentRoom()
        {
            if (!PhotonNetwork.InRoom || PhotonNetwork.OfflineMode || PhotonNetwork.LocalPlayer == null) return;
            joined = true; epoch = null; pendingEpoch = null; failedEpoch = null; pendingStart = null;
            policy.BeginRoom(PhotonNetwork.LocalPlayer.ActorNumber, null);
            if (PhotonNetwork.IsMasterClient) PublishNewEpoch();
            RefreshRoom();
        }

        void PublishNewEpoch()
        {
            if (PhotonNetwork.CurrentRoom == null || !PhotonNetwork.IsMasterClient) return;
            pendingEpoch = Guid.NewGuid().ToString("N");
            if (!PhotonNetwork.CurrentRoom.SetCustomProperties(new PhotonHashtable { { EpochKey, pendingEpoch } }))
                Deny("The room compatibility session could not be published.");
        }

        void RefreshRoom()
        {
            if (!joined || !PhotonNetwork.InRoom || PhotonNetwork.CurrentRoom == null || PhotonNetwork.LocalPlayer == null) return;
            string published = PhotonNetwork.CurrentRoom.CustomProperties[EpochKey] as string;
            if (pendingEpoch != null && published != pendingEpoch) published = null;
            if (failedEpoch != null && published == failedEpoch) published = null;
            if (!PeerAdvertisement.IsEpoch(published)) published = null;
            if (published != epoch)
            {
                epoch = published; policy.BeginRoom(PhotonNetwork.LocalPlayer.ActorNumber, epoch);
                if (epoch != null) { pendingEpoch = null; failedEpoch = null; PublishLocal(); }
            }
            var players = PhotonNetwork.PlayerList;
            policy.ReplaceRoster(players.Select(p => p.ActorNumber));
            foreach (var player in players) ObservePlayer(player);
            var verdict = Evaluate();
            if (!verdict.Allowed) Deny(verdict.Reason);
            else lastMessage = null;
        }

        void PublishLocal()
        {
            if (!PhotonNetwork.InRoom || PhotonNetwork.LocalPlayer == null) return;
            var ad = new PhotonHashtable {
                { ProtocolKey, local == null ? null : local.Protocol }, { ProfileKey, local == null ? null : local.Profile },
                { GameKey, local == null ? null : local.GameHash }, { DigestKey, local == null ? null : local.ContentDigest },
                { ActorKey, PhotonNetwork.LocalPlayer.ActorNumber }, { PeerEpochKey, epoch }
                , { ModsKey, local == null ? null : local.ModsDigest }, { AssetsKey, local == null ? null : local.AssetsDigest },
                { ConfigKey, local == null ? null : local.ConfigDigest }
            };
            if (!PhotonNetwork.LocalPlayer.SetCustomProperties(ad)) Deny("Local compatibility advertisement could not be published.");
        }

        void ObservePlayer(PhotonPlayer actualPlayer)
        {
            var props = actualPlayer.CustomProperties;
            PeerAdvertisement peer = null;
            if (props[ActorKey] is int)
                peer = new PeerAdvertisement(props[ProtocolKey] as string, props[ProfileKey] as string,
                    props[GameKey] as string, props[DigestKey] as string, (int)props[ActorKey], props[PeerEpochKey] as string,
                    props[ModsKey] as string, props[AssetsKey] as string, props[ConfigKey] as string);
            policy.ObservePeer(actualPlayer.ActorNumber, peer, policy.Generation);
        }

        void ClearConnection()
        {
            joined = false; epoch = null; pendingEpoch = null; failedEpoch = null; pendingStart = null; lastMessage = null;
            policy.Clear();
            if (PhotonNetwork.LocalPlayer != null)
                PhotonNetwork.LocalPlayer.SetCustomProperties(new PhotonHashtable {
                    { ProtocolKey, null }, { ProfileKey, null }, { GameKey, null }, { DigestKey, null }, { ActorKey, null }, { PeerEpochKey, null },
                    { ModsKey, null }, { AssetsKey, null }, { ConfigKey, null }
                });
        }

        void OnGUI()
        {
            if (!PhotonNetwork.InRoom || PhotonNetwork.OfflineMode || String.IsNullOrEmpty(lastMessage)) return;
            GUI.Box(new Rect(18, 18, 520, 110), "Canna Rebound multiplayer compatibility");
            GUI.Label(new Rect(32, 47, 490, 72), lastMessage + "\nEvery participant needs the same prepared mods, assets and gameplay configuration.");
        }

        void OnDestroy()
        {
            ClearConnection(); PhotonNetwork.RemoveCallbackTarget(this);
            // Keep installed gates fail-closed if this plugin is removed during an online session.
            if (instance == this) instance = null;
        }

        public void OnJoinedRoom() { JoinCurrentRoom(); }
        public void OnLeftRoom() { ClearConnection(); }
        public void OnDisconnected(DisconnectCause cause) { ClearConnection(); }
        public void OnPlayerEnteredRoom(PhotonPlayer player) { RefreshRoom(); }
        public void OnPlayerLeftRoom(PhotonPlayer player) { RefreshRoom(); }
        public void OnPlayerPropertiesUpdate(PhotonPlayer targetPlayer, PhotonHashtable changedProps) { RefreshRoom(); }
        public void OnRoomPropertiesUpdate(PhotonHashtable changedProps) { RefreshRoom(); }
        public void OnMasterClientSwitched(PhotonPlayer newMasterClient)
        {
            failedEpoch = epoch; epoch = null; pendingEpoch = null; pendingStart = null;
            policy.BeginRoom(PhotonNetwork.LocalPlayer.ActorNumber, null);
            if (PhotonNetwork.IsMasterClient) PublishNewEpoch();
            RefreshRoom();
        }
        public void OnFriendListUpdate(List<FriendInfo> friendList) { }
        public void OnCreatedRoom() { }
        public void OnCreateRoomFailed(short returnCode, string message) { }
        public void OnJoinRoomFailed(short returnCode, string message) { ClearConnection(); }
        public void OnJoinRandomFailed(short returnCode, string message) { ClearConnection(); }
        public void OnConnected() { }
        public void OnConnectedToMaster() { }
        public void OnRegionListReceived(RegionHandler regionHandler) { }
        public void OnCustomAuthenticationResponse(Dictionary<string, object> data) { }
        public void OnCustomAuthenticationFailed(string debugMessage) { ClearConnection(); }
    }
}
#endif
