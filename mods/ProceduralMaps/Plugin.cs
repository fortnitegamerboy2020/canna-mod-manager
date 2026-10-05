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
    [BepInPlugin("family.canna.proceduralmaps", "Canna Procedural Maps", "1.1.1")]
    public sealed class Plugin : BaseUnityPlugin
    {
        internal static ManualLogSource Log;
        internal static ConfigEntry<bool> Enabled;
        public static readonly Dictionary<AnimateVelocity, Island> Moving = new Dictionary<AnimateVelocity, Island>();
        internal static GameSessionHandler Session;
        internal static Layout Map;
        internal const string Protocol = "canna-proc-1.1.1";
        private void Awake()
        {
            Log = Logger;
            Enabled = Config.Bind("General", "Enabled", true, "Use the same generator version and enabled state on every family member's PC. Online start is blocked while a lobby member is missing the matching generator.");
            new Harmony("family.canna.proceduralmaps").PatchAll(typeof(Plugin).Assembly);
            Log.LogInfo("Canna Procedural Maps 1.1.1: shared round seeds, fixed simulation movement and Steam lobby compatibility checks loaded.");
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
            uint seed=Theme.Seed;
            Layout map = Layout.Generate(seed, 6 + (int)((seed >> 16) % 4));
            Audit(__instance, originals, map);
            List<StickyRoundedRectangle> platforms = new List<StickyRoundedRectangle>();
            foreach (StickyRoundedRectangle original in originals)
                if (original.GetComponent<AnimateVelocity>() != null && original.GetComponent<BoplBody>() != null && original.GetComponent<SpriteRenderer>() != null && IsLayoutTemplate(original))
                    platforms.Add(original);
            if (platforms.Count == 0) { Plugin.Log.LogWarning("No native physics template; retaining native map."); return; }
            if(map.islands[0].width>=800)platforms.Sort(delegate(StickyRoundedRectangle a,StickyRoundedRectangle b){return Aspect(a).CompareTo(Aspect(b));});
            else {int rotation=(int)(seed%(uint)platforms.Count);for(int i=0;i<rotation;i++){StickyRoundedRectangle first=platforms[0];platforms.RemoveAt(0);platforms.Add(first);}}
            if(map.moon) {
                // Keep astronauts spawning on moons. Satellite panels belong to
                // optional upper platforms and retain their native art/Drill layers.
                platforms.Sort(delegate(StickyRoundedRectangle a,StickyRoundedRectangle b){
                    return (IsSatellite(a)?1:0).CompareTo(IsSatellite(b)?1:0);
                });
                if((seed & 4)!=0)for(int slot=Math.Min(map.islands.Length,platforms.Count)-1;slot>=0;slot--) {
                    bool spawn=false;foreach(int index in map.spawnIslands)if(index==slot)spawn=true;
                    if(spawn)continue;
                    int source=platforms.FindIndex(IsSatellite);
                    if(source<0)break;
                    StickyRoundedRectangle old=platforms[slot];platforms[slot]=platforms[source];platforms[source]=old;
                    map.islands[slot].spin=(seed & 8)==0?0:((seed & 16)==0?1:-1)*(20+(int)((seed>>8)%101));
                    break;
                }
            }
            foreach (StickyRoundedRectangle original in originals)
                if (!platforms.Contains(original)) { original.gameObject.SetActive(false); Updater.DestroyFix(original.gameObject); }
            while (platforms.Count > map.islands.Length)
            {
                StickyRoundedRectangle excess = platforms[platforms.Count - 1];
                excess.gameObject.SetActive(false); Updater.DestroyFix(excess.gameObject); platforms.RemoveAt(platforms.Count - 1);
            }
            while (platforms.Count < map.islands.Length)
            {
                StickyRoundedRectangle template=platforms[platforms.Count>1?1:0];
                StickyRoundedRectangle copy = FixTransform.InstantiateFixed(template, Plugin.Position(map.islands[platforms.Count]), Fix.Zero);
                copy.transform.SetParent(platforms[0].transform.parent, false);
                MonoUpdatable[] registered = copy.GetComponentsInChildren<MonoUpdatable>();
                Array.Sort(registered, delegate(MonoUpdatable a, MonoUpdatable b) { return a.HierarchyNumber.CompareTo(b.HierarchyNumber); });
                for (int component = 0; component < registered.Length; component++)
                    registered[component].HierarchyNumber = 200000000 + platforms.Count * 1000 + component;
                copy.name = "Canna generated island " + platforms.Count;
                platforms.Add(copy);
            }
            for (int i = 0; i < platforms.Count; i++)
            {
                Island p = map.islands[i];
                // Authored map paths/size animations refer to the old map's coordinates.
                // Preserve terrain gameplay; let only the generated layout drive motion.
                foreach (MonoUpdatable controller in platforms[i].GetComponents<MonoUpdatable>())
                    if (controller is AntiLockPlatform || controller is VectorFieldPlatform || controller is AnimatePlatformSize)
                        controller.enabled = false;
                FixTransform ft = platforms[i].GetComponent<FixTransform>();
                Plugin.Set(ft, "_position", Plugin.Position(p));
                Plugin.Set(ft, "_rotation", Fix.Zero);
                Plugin.Set(ft, "_up", Vec2.up); Plugin.Set(ft, "_right", Vec2.right);
                ft.position = Plugin.Position(p); ft.rotation = Fix.Zero;

                BoplBody body = platforms[i].GetComponent<BoplBody>();
                if (body != null)
                {
                    body.StartVelocity = Vec2.zero;
                    body.StartAngularVelocity = Fix.Zero;
                    body.gravityScale = Fix.Zero;
                }
                DPhysicsRoundedRect rr = platforms[i].GetComponent<DPhysicsRoundedRect>();
                // Use the game's own uniform scaling API. It keeps the stock sprite,
                // material, slime trails, collision shape and Drill terrain layers together.
                // Keep the requested horizontal span and the native silhouette's vertical depth.
                Vec2 extents = (Vec2)AccessTools.Field(typeof(DPhysicsRoundedRect), "startExtents").GetValue(rr);
                Fix radius = (Fix)AccessTools.Field(typeof(DPhysicsRoundedRect), "startRadius").GetValue(rr);
                Fix ratio = Plugin.F(p.width + p.radius) / (extents.x + radius);
                Fix requestedScale = rr.Scale * ratio;
                rr.MinScale = Fix.Min(rr.MinScale, requestedScale);
                rr.MaxScale = Fix.Max(rr.MaxScale, requestedScale);
                rr.Scale = requestedScale;
                platforms[i].baseScaleForPlatform = rr.Scale;
                GrowOnStart grow = platforms[i].GetComponent<GrowOnStart>();
                if (grow != null) Plugin.Set(grow, "scaleUp", rr.Scale);
                extents = (Vec2)AccessTools.Field(typeof(DPhysicsRoundedRect), "startExtents").GetValue(rr);
                radius = (Fix)AccessTools.Field(typeof(DPhysicsRoundedRect), "startRadius").GetValue(rr);
                p.width = (int)(long)(extents.x * (Fix)100L);
                p.height = (int)(long)(extents.y * (Fix)100L);
                p.radius = (int)(long)(radius * (Fix)100L);
                AnimateVelocity movement = platforms[i].GetComponent<AnimateVelocity>();
                if (movement != null)
                {
                    movement.HomeIsMovable = true;
                    movement.speed = (Fix)10L;
                    movement.mu = (Fix)8L;
                    Plugin.Set(movement,"inRotationMode",false);
                    if(p.spin!=0){
                        movement.interpolateRotationHome=true;
                        movement.rotationSpeed=(Fix)20L;
                        movement.rotationMu=(Fix)1L/(Fix)20L;
                    }
                    Plugin.Moving[movement] = p;
                }
            }
            map.FinishNativeGeometry();
            for(int i=0;i<platforms.Count;i++){
                FixTransform ft=platforms[i].GetComponent<FixTransform>();
                Plugin.Set(ft,"_position",Plugin.Position(map.islands[i]));ft.position=Plugin.Position(map.islands[i]);
            }
            __instance.teamSpawns = new Vec2[4];
            for (int i = 0; i < 4; i++)
            {
                int host=map.spawnIslands[i];
                Island p = map.islands[host];
                // Authored map paths/size animations refer to the old map's coordinates.
                // Preserve terrain gameplay; let only the generated layout drive motion.
                foreach (MonoUpdatable controller in platforms[host].GetComponents<MonoUpdatable>())
                    if (controller is AntiLockPlatform || controller is VectorFieldPlatform || controller is AnimatePlatformSize)
                        controller.enabled = false;
                int total=0,rank=0;for(int j=0;j<4;j++)if(map.spawnIslands[j]==host){total++;if(j<i)rank++;}
                int offset=(2*rank-total+1)*Math.Min(150,(p.width+p.radius)/5);
                int dx=Math.Max(0,Math.Abs(offset)-p.width);
                int surface=p.height+Layout.IntSqrt((long)p.radius*p.radius-(long)dx*dx);
                __instance.teamSpawns[i] = new Vec2(Plugin.F(p.x+offset), Plugin.F(p.y+surface+150));
            }
            __instance.teammateSpawnSpacing = (Fix)2L;
            __instance.levelType = map.moon ? LevelType.space : LevelType.grass;
            map.RefreshFingerprint();
            File.WriteAllText(Path.Combine(Paths.ConfigPath, "CannaMaps/last-generated-map.json"), map.ToJson());
            Plugin.Map = map;
            Plugin.Log.LogInfo("Generated " + (map.moon ? "moon" : "normal-gravity") + " map: seed=" + seed + ", islands=" + map.islands.Length + ", layout=" + map.fingerprint + ". Compare this fingerprint on every client.");
        }
        private static bool IsLayoutTemplate(StickyRoundedRectangle platform)
        {
            DPhysicsRoundedRect rr = platform.GetComponent<DPhysicsRoundedRect>();
            Vec2 ext = (Vec2)AccessTools.Field(typeof(DPhysicsRoundedRect), "startExtents").GetValue(rr);
            Fix radius = (Fix)AccessTools.Field(typeof(DPhysicsRoundedRect), "startRadius").GetValue(rr);
            return ext.x + radius > Fix.Zero && (IsSatellite(platform) || ext.y + radius >= (ext.x + radius) * (Fix)1L/(Fix)10L)
                && ext.y + radius <= (ext.x + radius) * (Fix)101L / (Fix)100L;
        }
        private static bool IsSatellite(StickyRoundedRectangle p){return p.platformType==PlatformType.robot;}
        private static Fix Aspect(StickyRoundedRectangle p){
            DPhysicsRoundedRect rr=p.GetComponent<DPhysicsRoundedRect>();
            Vec2 ext=(Vec2)AccessTools.Field(typeof(DPhysicsRoundedRect),"startExtents").GetValue(rr);
            Fix r=(Fix)AccessTools.Field(typeof(DPhysicsRoundedRect),"startRadius").GetValue(rr);
            return (ext.y+r)/(ext.x+r);
        }
        private static void Audit(GameSessionHandler session, StickyRoundedRectangle[] originals, Layout map)
        {
            try
            {
                string folder = Path.Combine(Paths.ConfigPath, "CannaMaps"); Directory.CreateDirectory(folder);
                string scene = SceneManager.GetActiveScene().buildIndex.ToString();
                if(Array.IndexOf(Environment.GetCommandLineArgs(),"--canna-map-audit")>=0) {
                    string objects="Native scene "+scene+" type="+session.levelType+" path="+SceneManager.GetActiveScene().path+"\n";
                    foreach(StickyRoundedRectangle p in originals) {
                        SpriteRenderer art=p.GetComponent<SpriteRenderer>();
                        objects+=p.name+" sprite="+(art.sprite==null?"null":art.sprite.name)+" type="+p.platformType+"\n";
                    }
                    File.WriteAllText(Path.Combine(folder,"native-objects-"+scene+".txt"),objects);
                }
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

    [HarmonyPatch]
    internal static class AuthoredMapControllers
    {
        private static IEnumerable<MethodBase> TargetMethods()
        {
            foreach (Type type in new Type[] { typeof(AntiLockPlatform), typeof(VectorFieldPlatform), typeof(AnimatePlatformSize) })
            {
                yield return AccessTools.Method(type, "Init");
                yield return AccessTools.Method(type, "UpdateSim");
            }
        }
        private static bool Prefix(MonoUpdatable __instance)
        {
            AnimateVelocity movement = __instance.GetComponent<AnimateVelocity>();
            return movement == null || !Plugin.Moving.ContainsKey(movement);
        }
    }

    [HarmonyPatch(typeof(AnimateVelocity), "UpdateSim")]
    internal static class MoveIslands
    {
        private static readonly FieldInfo InProgress = AccessTools.Field(typeof(GameSessionHandler), "gameInProgress");
        private static void Prefix(AnimateVelocity __instance,Fix simDeltaTime)
        {
            Island p;
            if (Plugin.Map == null || Plugin.Session == null || !Plugin.Moving.TryGetValue(__instance, out p) ||
                __instance.inSuddenDeath || __instance.isBeingControlled || GameSessionHandler.SuddenDeathInProgress ||
                !(bool)InProgress.GetValue(Plugin.Session)) return;
            if(p.spin!=0) {
                FieldInfo rotation=AccessTools.Field(typeof(AnimateVelocity),"homeRotation");
                Fix angle=(Fix)rotation.GetValue(__instance)+Plugin.F(p.spin)*simDeltaTime*GameTime.PlayerTimeScale;
                rotation.SetValue(__instance,((angle % Fix.PiTimes2)+Fix.PiTimes2)%Fix.PiTimes2);
            }
            if(p.drift==0)return;
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
                    Plugin.Log.LogWarning("Online round blocked: " + member.Name + " needs Canna Procedural Maps 1.1.1 enabled. Wait a moment after joining, then retry.");
                    return false;
                }
            }
            return true;
        }
    }
}






