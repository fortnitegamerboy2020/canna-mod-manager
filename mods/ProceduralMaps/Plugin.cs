using System;
using System.IO;
using System.Collections.Generic;
using System.Reflection;
using BepInEx;
using BepInEx.Configuration;
using BepInEx.Logging;
using BoplFixedMath;
using HarmonyLib;
using UnityEngine;
using UnityEngine.SceneManagement;

namespace Canna.ProceduralMaps
{
    [BepInPlugin("family.canna.proceduralmaps", "Canna Procedural Maps", "1.0.1")]
    public sealed class Plugin : BaseUnityPlugin
    {
        internal static ManualLogSource Log;
        internal static ConfigEntry<bool> Enabled;
        public static readonly Dictionary<AnimateVelocity, Island> Moving = new Dictionary<AnimateVelocity, Island>();
        internal static GameSessionHandler Session;
        internal static Layout Map;
        internal const string Protocol = "canna-proc-1.0.1";
        private void Awake()
        {
            Log = Logger;
            Enabled = Config.Bind("General", "Enabled", true, "Use the same generator version and enabled state on every family member's PC. Online start is blocked while a lobby member is missing the matching generator.");
            new Harmony("family.canna.proceduralmaps").PatchAll(typeof(Plugin).Assembly);
            Log.LogInfo("Canna Procedural Maps 1.0.1: shared round seeds, fixed simulation movement and Steam lobby compatibility checks loaded.");
        }
        private void OnDestroy() { Log.LogInfo("Keeping procedural map hooks active across scene changes."); }
        internal static Fix F(int hundredths) { return (Fix)(long)hundredths / (Fix)100L; }
        internal static Vec2 Position(Island p) { return new Vec2(F(p.x), F(p.y)); }
        internal static void Set(object target, string name, object value) { AccessTools.Field(target.GetType(), name).SetValue(target, value); }
    }

    [HarmonyPatch(typeof(GameSessionHandler), "Awake")]
    internal static class GenerateMap
    {
        private static void Prefix(GameSessionHandler __instance)
        {
            Plugin.Moving.Clear(); Plugin.Map = null; Plugin.Session = __instance;
            if (!Plugin.Enabled.Value || GameLobby.isPlayingAReplay || __instance.Level == null) return;
            // Lobby metadata gates host startup only. Its asynchronous client cache must
            // never decide simulation behavior after the synchronized start packet arrives.
            StickyRoundedRectangle[] originals = __instance.Level.transform.root.GetComponentsInChildren<StickyRoundedRectangle>();
            if (originals.Length == 0) { Plugin.Log.LogWarning("No native platform template; retaining native map."); return; }
            // Online packet seed is identical on all participants; no UnityEngine.Random or wall clock.
            uint seed = GameLobby.isOnlineGame ? SteamManager.startParameters.seed : (uint)Updater.RandomInt(1, int.MaxValue);
            Layout map = Layout.Generate(seed, 6 + (int)((seed >> 16) % 4));
            Audit(__instance, originals, map);
            List<StickyRoundedRectangle> platforms = new List<StickyRoundedRectangle>(originals);
            while (platforms.Count > map.islands.Length)
            {
                StickyRoundedRectangle excess = platforms[platforms.Count - 1];
                excess.gameObject.SetActive(false);
                Updater.DestroyFix(excess.gameObject);
                platforms.RemoveAt(platforms.Count - 1);
            }
            while (platforms.Count < map.islands.Length)
            {
                // Native prefab registration assigns unique deterministic hierarchy numbers.
                StickyRoundedRectangle copy = FixTransform.InstantiateFixed(originals[0], Plugin.Position(map.islands[platforms.Count]), Fix.Zero);
                copy.transform.SetParent(originals[0].transform.parent, false);
                copy.name = "Canna generated island " + platforms.Count;
                platforms.Add(copy);
            }
            for (int i = 0; i < platforms.Count; i++)
            {
                Island p = map.islands[i];
                FixTransform ft = platforms[i].GetComponent<FixTransform>();
                Plugin.Set(ft, "_position", Plugin.Position(p));
                Plugin.Set(ft, "_rotation", Fix.Zero);
                Plugin.Set(ft, "_up", Vec2.up); Plugin.Set(ft, "_right", Vec2.right);
                ft.position = Plugin.Position(p); ft.rotation = Fix.Zero;
                ft.offset = Vec2.zero;
                ft.SetScale_DPhysicsOnly(Fix.One);
                BoplBody body = platforms[i].GetComponent<BoplBody>();
                if (body != null)
                {
                    body.StartVelocity = Vec2.zero;
                    body.StartAngularVelocity = Fix.Zero;
                    body.gravityScale = Fix.Zero;
                }
                DPhysicsRoundedRect rr = platforms[i].GetComponent<DPhysicsRoundedRect>();
                Plugin.Set(rr, "startExtents", new Vec2(Plugin.F(p.width), Plugin.F(p.height)));
                Plugin.Set(rr, "startRadius", Plugin.F(p.radius));
                AnimateVelocity movement = platforms[i].GetComponent<AnimateVelocity>();
                if (movement != null)
                {
                    movement.HomeIsMovable = true;
                    movement.speed = (Fix)10L;
                    movement.mu = (Fix)8L;
                    Plugin.Moving[movement] = p;
                }
            }
            __instance.teamSpawns = new Vec2[4];
            for (int i = 0; i < 4; i++)
            {
                Island p = map.islands[i];
                __instance.teamSpawns[i] = new Vec2(Plugin.F(p.x), Plugin.F(p.y + p.height + p.radius + 150));
            }
            __instance.teammateSpawnSpacing = (Fix)2L;
            __instance.levelType = map.moon ? LevelType.space : LevelType.grass;
            Plugin.Map = map;
            Plugin.Log.LogInfo("Generated " + (map.moon ? "moon" : "normal-gravity") + " map: seed=" + seed + ", islands=" + map.islands.Length + ", layout=" + map.fingerprint + ". Compare this fingerprint on every client.");
        }
        private static void Audit(GameSessionHandler session, StickyRoundedRectangle[] originals, Layout map)
        {
            try
            {
                string folder = Path.Combine(Paths.ConfigPath, "CannaMaps"); Directory.CreateDirectory(folder);
                string scene = SceneManager.GetActiveScene().buildIndex.ToString();
                string audit = "scene,platform,x,y,half_width,half_height,radius,type\n";
                for (int i = 0; i < originals.Length; i++)
                {
                    FixTransform ft = originals[i].GetComponent<FixTransform>();
                    DPhysicsRoundedRect rr = originals[i].GetComponent<DPhysicsRoundedRect>();
                    Vec2 ext = (Vec2)AccessTools.Field(typeof(DPhysicsRoundedRect), "startExtents").GetValue(rr);
                    Fix radius = (Fix)AccessTools.Field(typeof(DPhysicsRoundedRect), "startRadius").GetValue(rr);
                    audit += scene + "," + i + "," + ft.serializedPosition.x + "," + ft.serializedPosition.y + "," + ext.x + "," + ext.y + "," + radius + "," + originals[i].platformType + "\n";
                }
                File.WriteAllText(Path.Combine(folder, "native-scene-" + scene + ".csv"), audit);
                File.WriteAllText(Path.Combine(folder, "last-generated-map.json"), map.ToJson());
            }
            catch (Exception error) { Plugin.Log.LogWarning("Map audit unavailable: " + error.GetType().Name); }
        }
    }

    [HarmonyPatch(typeof(DPhysicsRoundedRect), "Initialize")]
    internal static class ResizeAfterInit
    {
        private static void Postfix(DPhysicsRoundedRect __instance)
        {
            if (Plugin.Map == null) return;
            AnimateVelocity movement = __instance.GetComponent<AnimateVelocity>();
            Island p;
            if (movement == null || !Plugin.Moving.TryGetValue(movement, out p)) return;
            ResizablePlatform resizer = __instance.GetComponent<ResizablePlatform>();
            if (resizer != null) resizer.ResizePlatform(Plugin.F(p.height), Plugin.F(p.width), Plugin.F(p.radius), false);
        }
    }

    [HarmonyPatch(typeof(AnimateVelocity), "UpdateSim")]
    internal static class MoveIslands
    {
        private static readonly FieldInfo InProgress = AccessTools.Field(typeof(GameSessionHandler), "gameInProgress");
        private static void Prefix(AnimateVelocity __instance)
        {
            Island p;
            if (Plugin.Map == null || Plugin.Session == null || !Plugin.Moving.TryGetValue(__instance, out p) || p.drift == 0 ||
                __instance.inSuddenDeath || __instance.isBeingControlled || GameSessionHandler.SuddenDeathInProgress ||
                !(bool)InProgress.GetValue(Plugin.Session)) return;
            // Fixed simulation ticks advance together in Bopl's lockstep network, including replay timing.
            __instance.HomePosition = Plugin.Position(p) + new Vec2(Plugin.F(Layout.DriftAt(p, Updater.SimulationTicks)), Fix.Zero);
        }
    }

    [HarmonyPatch(typeof(SteamManager), "Update")]
    internal static class LobbyAdvertisement
    {
        private static float next;
        private static void Postfix(SteamManager __instance)
        {
            if (Time.unscaledTime < next || __instance.currentLobby.Id.Value == 0) return;
            next = Time.unscaledTime + 1;
            __instance.currentLobby.SetMemberData("canna_map_generator", Plugin.Enabled.Value ? Plugin.Protocol : "disabled");
        }
    }
    [HarmonyPatch]
    internal static class LobbyCompatibility
    {
        private static IEnumerable<MethodBase> TargetMethods()
        {
            yield return AccessTools.Method(typeof(SteamManager), "HostGame");
            yield return AccessTools.Method(typeof(SteamManager), "HostNextLevel");
        }
        private static bool Prefix(SteamManager __instance)
        {
            if (!Plugin.Enabled.Value || __instance.currentLobby.Id.Value == 0 || __instance.currentLobby.MemberCount < 2) return true;
            __instance.currentLobby.SetMemberData("canna_map_generator", Plugin.Protocol);
            foreach (Steamworks.Friend member in __instance.currentLobby.Members)
            {
                if (__instance.currentLobby.GetMemberData(member, "canna_map_generator") != Plugin.Protocol)
                {
                    Plugin.Log.LogWarning("Online round blocked: " + member.Name + " needs Canna Procedural Maps 1.0.1 enabled. Wait a moment after joining, then retry.");
                    return false;
                }
            }
            return true;
        }
    }
}
