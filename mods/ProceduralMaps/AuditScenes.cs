// Development-only scene audit. This assembly is never part of the mod ZIP.
using System;
using System.IO;
using System.Reflection;
using System.Collections.Generic;
using BepInEx;
using BoplFixedMath;
using HarmonyLib;
using UnityEngine;
using UnityEngine.SceneManagement;

[BepInPlugin("family.canna.mapaudit", "Canna Scene Audit (development)", "1.0.0")]
public class MapAudit : BaseUnityPlugin
{
    internal static bool Active;
    internal static int NextScene = 6;
    internal static float NextAt = 5;
    internal static string Folder;
    private void Awake()
    {
        Active = Array.IndexOf(Environment.GetCommandLineArgs(), "--canna-map-audit") >= 0;
        if (!Active) return;
        Folder = Path.Combine(Paths.ConfigPath, "CannaMaps");
        Directory.CreateDirectory(Folder);
        new Harmony("family.canna.mapaudit").PatchAll(typeof(MapAudit).Assembly);
    }
}
[HarmonyPatch]
static class AuditNoGameplay
{
    static IEnumerable<MethodBase> TargetMethods()
    {
        yield return AccessTools.Method(typeof(GameSessionHandler), "Init");
        yield return AccessTools.Method(typeof(GameSessionHandler), "Update");
        yield return AccessTools.Method(typeof(GameSessionHandler), "UpdateSim");
    }
    static bool Prefix() { return !MapAudit.Active; }
}
[HarmonyPatch(typeof(SteamManager), "Update")]
static class AuditNextScene
{
    static void Postfix()
    {
        if (!MapAudit.Active || Time.unscaledTime < MapAudit.NextAt) return;
        MapAudit.NextAt = Time.unscaledTime + 0.25f;
        if (MapAudit.NextScene > 6)
        {
            try { ValidateCurrentScene(); }
            catch (Exception error)
            {
                File.WriteAllText(Path.Combine(MapAudit.Folder, "audit-failure.txt"), "Scene " + SceneManager.GetActiveScene().buildIndex + ": " + error);
                MapAudit.NextAt = float.MaxValue;
                Application.Quit(); return;
            }
        }
        if (MapAudit.NextScene >= SceneManager.sceneCountInBuildSettings)
        {
            File.WriteAllText(Path.Combine(MapAudit.Folder, "audit-complete.txt"), "Audited playable build scenes 6 through " + (MapAudit.NextScene - 1));
            Application.Quit(); return;
        }
        Updater.PreLevelLoad();
        SceneManager.LoadScene(MapAudit.NextScene++);
        Updater.PostLevelLoad();
    }
    static void ValidateCurrentScene()
    {
        if (Canna.ProceduralMaps.Plugin.Moving.Count < 6) throw new Exception("Generator did not create the required platform controllers");
        AccessTools.Field(typeof(Host), "recordReplay").SetValue(null, false);
        GameTime.PlayerTimeScale = Fix.One;
        GameSessionHandler.GameIsPaused = false;
        Updater.TickSimulation((Fix)1L / (Fix)60L);
        // Exercise the same native resize API used before player spawning, after physics initialization.
        foreach (KeyValuePair<AnimateVelocity, Canna.ProceduralMaps.Island> entry in Canna.ProceduralMaps.Plugin.Moving)
        {
            DPhysicsRoundedRect rr = entry.Key.GetComponent<DPhysicsRoundedRect>();
            if (!rr.initHasBeenCalled) throw new Exception("Platform physics failed to initialize");
            ResizablePlatform resize = entry.Key.GetComponent<ResizablePlatform>();
            if (resize != null) resize.ResizePlatform((Fix)(long)entry.Value.height / (Fix)100L, (Fix)(long)entry.Value.width / (Fix)100L, (Fix)(long)entry.Value.radius / (Fix)100L, false);
            Vec2 position = new Vec2((Fix)(long)entry.Value.x / (Fix)100L, (Fix)(long)entry.Value.y / (Fix)100L);
            entry.Key.GetComponent<BoplBody>().position = position;
            entry.Key.GetComponent<FixTransform>().position = position;
            entry.Key.Initialize(position, Fix.Zero);
            entry.Key.enabled = true;
        }
        GameSessionHandler session = UnityEngine.Object.FindObjectOfType<GameSessionHandler>();
        AccessTools.Field(typeof(GameSessionHandler), "gameInProgress").SetValue(session, true);
        List<AnimateVelocity> platforms = new List<AnimateVelocity>(Canna.ProceduralMaps.Plugin.Moving.Keys);
        for (int i = 0; i < platforms.Count; i++)
        {
            DPhysicsRoundedRect a = platforms[i].GetComponent<DPhysicsRoundedRect>();
            Canna.ProceduralMaps.Island planned = Canna.ProceduralMaps.Plugin.Moving[platforms[i]];
            Vec2 extA = a.CalcExtents();
            Fix tolerance = (Fix)1L / (Fix)1000L;
            if (Fix.Abs(extA.x - (Fix)(long)planned.width / (Fix)100L) > tolerance || Fix.Abs(extA.y - (Fix)(long)planned.height / (Fix)100L) > tolerance)
                throw new Exception("World collider dimensions mismatch on " + platforms[i].name + ": actual=" + extA.x + "," + extA.y + "; expected=" + planned.width + "," + planned.height + "; resizer=" + (platforms[i].GetComponent<ResizablePlatform>() != null));
            for (int j = i + 1; j < platforms.Count; j++)
            {
                DPhysicsRoundedRect b = platforms[j].GetComponent<DPhysicsRoundedRect>();
                Vec2 delta = platforms[i].GetComponent<BoplBody>().position - platforms[j].GetComponent<BoplBody>().position;
                Vec2 extB = b.CalcExtents();
                if (Fix.Abs(delta.x) <= extA.x + extB.x + a.radius + b.radius + (Fix)3L &&
                    Fix.Abs(delta.y) <= extA.y + extB.y + a.radius + b.radius + (Fix)3L)
                    throw new Exception("Actual spawned island colliders overlap their safety margin");
            }
        }
        foreach (KeyValuePair<AnimateVelocity, Canna.ProceduralMaps.Island> entry in Canna.ProceduralMaps.Plugin.Moving)
            if (entry.Value.drift != 0)
            {
                Vec2 before = entry.Key.GetComponent<BoplBody>().position;
                for (int tick = 0; tick < 30; tick++) Updater.TickSimulation((Fix)1L / (Fix)60L);
                Vec2 after = entry.Key.GetComponent<BoplBody>().position;
                if (before.x == after.x && before.y == after.y) throw new Exception("Generated moving island did not move");
                break;
            }
        File.AppendAllText(Path.Combine(MapAudit.Folder, "physics-audit.txt"), "Scene " + (MapAudit.NextScene - 1) + " initialized, resized and moving islands simulated successfully\n");
    }
}
