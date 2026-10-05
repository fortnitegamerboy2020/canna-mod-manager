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
        yield return AccessTools.Method(typeof(GameSessionHandler), "StartSpawnPlayersRoutine");
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
        // Run the game's real Init and level introduction. Never manually resize or
        // teleport the grounds: those shortcuts hid the preset-sprite rendering bug.
        GameSessionHandler session = UnityEngine.Object.FindObjectOfType<GameSessionHandler>();
        RoutineQueue intro = (RoutineQueue)AccessTools.Field(typeof(GameSessionHandler), "levelAnimationRoutine").GetValue(session);
        for (int tick = 0; tick < 600; tick++)
        { intro.Update((Fix)1L / (Fix)60L); Updater.TickSimulation((Fix)1L / (Fix)60L); }
        Drill[] drills = Resources.FindObjectsOfTypeAll<Drill>();
        if (drills.Length == 0) throw new Exception("Native Drill template unavailable for terrain collision audit");
        LayerMask drillables = (LayerMask)AccessTools.Field(typeof(Drill), "drillables").GetValue(drills[0]);
        foreach (KeyValuePair<AnimateVelocity, Canna.ProceduralMaps.Island> entry in Canna.ProceduralMaps.Plugin.Moving)
        {
            DPhysicsRoundedRect rr = entry.Key.GetComponent<DPhysicsRoundedRect>();
            if (!rr.initHasBeenCalled) throw new Exception("Platform physics failed to initialize");
            SpriteRenderer renderer = entry.Key.GetComponent<SpriteRenderer>();
            if (renderer.sprite == null || renderer.material.shader.name == "Unlit/ResizablePlatform")
                throw new Exception("Generated platform lost its native ground texture/material");
            PhysicsParent[] hits = new PhysicsParent[128];
            Box probe = new Box {
                center = entry.Key.GetComponent<BoplBody>().position,
                right = Vec2.right * ((Fix)1L / (Fix)10L),
                up = Vec2.up * ((Fix)1L / (Fix)10L),
                inverseExtents = new Vec2((Fix)10L, (Fix)10L), layer = 0
            };
            int count = DetPhysics.Get().CollideBox(probe, ref hits, drillables);
            bool drillCanEnter = false;
            for (int hit = 0; hit < count; hit++)
                if (hits[hit].fixTrans == entry.Key.GetComponent<FixTransform>() && hits[hit].fixTrans.GetComponent<StickyRoundedRectangle>() != null)
                    drillCanEnter = true;
            if (!drillCanEnter) throw new Exception("Native Drill terrain query cannot detect generated island " + entry.Key.name);
            if (entry.Key.GetComponent<StickyRoundedRectangle>() == null || entry.Key.GetComponent<BoplBody>() == null)
                throw new Exception("Generated platform lost native drill terrain components");
        }
        AccessTools.Field(typeof(GameSessionHandler), "gameInProgress").SetValue(session, true);
        List<AnimateVelocity> platforms = new List<AnimateVelocity>(Canna.ProceduralMaps.Plugin.Moving.Keys);
        for (int i = 0; i < platforms.Count; i++)
        {
            DPhysicsRoundedRect a = platforms[i].GetComponent<DPhysicsRoundedRect>();
            Canna.ProceduralMaps.Island planned = Canna.ProceduralMaps.Plugin.Moving[platforms[i]];
            Vec2 extA = a.CalcExtents();
            Fix tolerance = (Fix)1L / (Fix)100L;
            if (Fix.Abs(extA.x - (Fix)(long)planned.width / (Fix)100L) > tolerance || Fix.Abs(extA.y - (Fix)(long)planned.height / (Fix)100L) > tolerance)
                throw new Exception("World collider dimensions mismatch on " + platforms[i].name + ": actual=" + extA.x + "," + extA.y + "; expected=" + planned.width + "," + planned.height + "; resizer=" + (platforms[i].GetComponent<ResizablePlatform>() != null));
            for (int j = i + 1; j < platforms.Count; j++)
            {
                DPhysicsRoundedRect b = platforms[j].GetComponent<DPhysicsRoundedRect>();
                Vec2 delta = platforms[i].GetComponent<BoplBody>().position - platforms[j].GetComponent<BoplBody>().position;
                Vec2 extB = b.CalcExtents();
                if (Fix.Abs(delta.x) <= extA.x + extB.x + a.radius + b.radius + (Fix)3L &&
                    Fix.Abs(delta.y) <= extA.y + extB.y + a.radius + b.radius + (Fix)3L)
                    throw new Exception("Actual spawned island colliders overlap their safety margin: a=" + platforms[i].name + " pos=" + platforms[i].GetComponent<BoplBody>().position + " b=" + platforms[j].name + " pos=" + platforms[j].GetComponent<BoplBody>().position + " extA=" + extA + " extB=" + extB);
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



