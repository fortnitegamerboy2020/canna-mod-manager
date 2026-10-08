// Test-only plugin. Never include in a published Canna Rebound package.
using System;
using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using BepInEx;
using HarmonyLib;
using Photon.Pun;
using UnityEngine;
using UnityEngine.SceneManagement;

[BepInPlugin("canna.ducttapeplusplus.smokechecks", "Canna Rebound controlled smoke checks", "0.0.1")]
[BepInDependency("canna.ducttapeplusplus.networkguard")]
[BepInDependency("com.willis.rounds.unbound")]
public sealed class DuctTapePlusPlusSmokeChecks : BaseUnityPlugin
{
    readonly List<string> report = new List<string>();
    string output;
    bool finished;
    float deadline;
    const string CosmicGuid = "com.XAngelMoonX.rounds.CosmicRounds";
    Player smokePlayer;
    GameObject temporaryCard;
    CardInfo cosmicPrototype;
    CardInfoDisplayer cosmicDisplayer;
    object prototypeLocalizedName, prototypeLocalizedDescription;
    CardInfoStat[] prototypeStats;
    bool prototypeActive;
    int prototypeRegistryCount;

    IEnumerator Start()
    {
        var args = Environment.GetCommandLineArgs();
        int at = Array.IndexOf(args, "--dtpp-smoke");
        if (at < 0 || at + 1 >= args.Length) yield break;
        output = Path.GetFullPath(args[at + 1]);
        deadline = Time.realtimeSinceStartup + 85f;
        report.Add("LIMIT: Controlled local/offline runtime only. No two-client multiplayer synchronization is verified.");
        yield return new WaitForSecondsRealtime(20f);
        SceneDiagnostics("initial");
        if (!HasLoadedNativeMainScene() && !Run("Native loaded intro skip", TrySkipLoadedIntro)) { Finish(); yield break; }
        float sceneDeadline = Time.realtimeSinceStartup + 25f;
        while (!HasLoadedNativeMainScene() && Time.realtimeSinceStartup < sceneDeadline)
            yield return new WaitForSecondsRealtime(.25f);
        SceneDiagnostics("before-gates");
        if (!Run("Native start and readiness gates", CheckNativeGates)) { Finish(); yield break; }
        if (!Run("Real Unbound public sandbox start", StartSandbox)) { Finish(); yield break; }
        float sandboxDeadline = Time.realtimeSinceStartup + 20f;
        while (Time.realtimeSinceStartup < sandboxDeadline && (!PhotonNetwork.OfflineMode || !PhotonNetwork.InRoom || !MapReady()))
            yield return new WaitForSecondsRealtime(.1f);
        report.Add("DIAG Sandbox readiness: offline=" + PhotonNetwork.OfflineMode + "; in-room=" + PhotonNetwork.InRoom + "; map-ready=" + MapReady());
        if (!Check(PhotonNetwork.OfflineMode && PhotonNetwork.InRoom, "Native offline setup creates a local Photon room")) { Finish(); yield break; }
        if (!Check(MapReady(), "Native sandbox map is loaded before AI creation")) { Finish(); yield break; }
        if (!Check(PlayerAssigner.instance && PlayerManager.instance, "Native player assignment and manager are available")) { Finish(); yield break; }
        int before = PlayerManager.instance.players.Count;
        if (!Run("Native offline AI creation request", () => PlayerAssigner.instance.CreatePlayer(null, true))) { Finish(); yield break; }
        float playerDeadline = Time.realtimeSinceStartup + 20f;
        while (Time.realtimeSinceStartup < playerDeadline && !PlayerManager.instance.players.Any(p => p && p.data && p.data.healthHandler))
            yield return new WaitForSecondsRealtime(.1f);
        if (!Check(PlayerManager.instance.players.Count > before, "Native offline player was created")) { Finish(); yield break; }
        yield return new WaitForSecondsRealtime(2f);
        if (!Run("Translated public player getter and damage API", CheckPlayer)) { Finish(); yield break; }
        Check(CardChoice.instance && CardChoice.instance.cards != null && CardChoice.instance.cards.Length > 0,
            "Native card registry is populated (custom-card behavior requires a separate check)");
        if (BepInEx.Bootstrap.Chainloader.PluginInfos.ContainsKey(CosmicGuid))
        {
            if (!Run("Registered CosmicRounds card clone", PrepareCosmicCard)) { Finish(); yield break; }
            yield return null; yield return null;
            if (!Run("Actual CosmicRounds native card draw", DrawCosmicCard)) { Finish(); yield break; }
            yield return null; yield return null;
            if (!Run("Actual CosmicRounds native text output", () => CheckCosmicCardText("Actual CR native draw", false))) { Finish(); yield break; }
            if (!Run("Simulated missing-localized-reference frame draw", DrawLegacyCosmicFrame)) { Finish(); yield break; }
            yield return null; yield return null;
            if (!Run("Simulated legacy frame text output", () => CheckCosmicCardText("Simulated legacy frame fixture", true))) { Finish(); yield break; }
            if (!Run("Native ordinary CosmicRounds card application", ApplyCosmicCard)) { Finish(); yield break; }
            CleanupTemporaryCard(); yield return null; yield return null;
            if (!Run("Disposable CosmicRounds card cleanup", CheckCosmicPrototype)) { Finish(); yield break; }
            report.Add("LIMIT: CosmicRounds test covers one Speed Up card's offline draw and stat application; missing-reference frame is a simulated fixture, not a verified original old asset. No firing, other-card, or multiplayer behavior is certified.");
        }
        else report.Add("LIMIT: CosmicRounds is absent; optional custom-card checks were not run.");
        Finish();
    }

    void Update()
    {
        if (output != null && !finished && Time.realtimeSinceStartup >= deadline)
        { report.Add("FAIL Controlled smoke deadline exceeded"); Finish(); }
    }

    void CheckNativeGates()
    {
        var guardType = AppDomain.CurrentDomain.GetAssemblies().Select(a => a.GetType("Canna.DuctTapePlusPlus.NetworkGuard", false)).FirstOrDefault(t => t != null);
        Require(guardType != null, "Network guard assembly is loaded");
        var guard = guardType.GetField("instance", BindingFlags.Static | BindingFlags.NonPublic).GetValue(null);
        Require(guard != null, "Network guard plugin is active");
        string expectedManifestPath = Path.GetFullPath(Path.Combine(Paths.PluginPath, "Canna", "DuctTapePlusPlus", "compatibility-manifest.json"));
        string readManifestPath = guardType.GetField("manifestReadPath", BindingFlags.Instance | BindingFlags.NonPublic).GetValue(guard) as string;
        report.Add("DIAG Manifest path equals installed Canna path: " + String.Equals(expectedManifestPath, readManifestPath, StringComparison.OrdinalIgnoreCase));
        report.Add("DIAG Manifest exists at installed Canna path: " + File.Exists(expectedManifestPath));
        foreach (string field in new[] { "manifestReadStatus", "contentError", "patchError" })
        {
            string value = guardType.GetField(field, BindingFlags.Instance | BindingFlags.NonPublic).GetValue(guard) as string;
            // Only guard-owned fixed status labels/reasons, bounded without raw exception or log dumps.
            string bounded = value == null ? "none" : new string(value.Take(640).Select(c => c >= ' ' && c <= '~' ? c : '?').ToArray());
            report.Add("DIAG " + field + ": " + bounded);
        }
        Require(guardType.GetField("prepared", BindingFlags.Instance | BindingFlags.NonPublic).GetValue(guard) != null, "Prepared manifest was read from installed Canna root");
        Require(guardType.GetField("local", BindingFlags.Instance | BindingFlags.NonPublic).GetValue(guard) != null &&
            guardType.GetField("contentError", BindingFlags.Instance | BindingFlags.NonPublic).GetValue(guard) == null &&
            guardType.GetField("patchError", BindingFlags.Instance | BindingFlags.NonPublic).GetValue(guard) == null,
            "Installed mod assets and current gameplay configuration validate");
        var library = AppDomain.CurrentDomain.GetAssemblies().Single(a => a.GetName().Name == "UnboundLib");
        var healthPatch = library.GetType("UnboundLib.Patches.HealthBar_Patch_Update", true);
        Require(HasRuntimePrefix(AccessTools.Method(healthPatch, "Postfix"), "UL_HealthBarRespawns_Fix"),
            "Adapted runtime health-bar prefix is installed on actual Unbound method");
        var updateChecker = library.GetType("UnboundLib.Utils.UI.UpdateChecker", true);
        foreach (string target in new[] { "RegisterModUpdateChecker", "CreateUpdateMenu" })
            Require(HasRuntimePrefix(AccessTools.Method(updateChecker, target), "UL_UpdateNotice_Fix"),
                "Adapted runtime update prefix is installed on actual Unbound " + target);
        Require(HasLoadedNativeMainScene(), "Native gameplay scene is loaded before startup-survival certification");
        Require(BepInEx.Bootstrap.Chainloader.ManagerObject &&
            (BepInEx.Bootstrap.Chainloader.ManagerObject.hideFlags & HideFlags.HideAndDontSave) == HideFlags.HideAndDontSave,
            "Shared plugin manager survives the loaded gameplay scene with BepInEx-equivalent hidden flags");
        Require(GameManager.instance && !GameManager.instance.isPlaying, "Controlled copy is at the menu before match start");
        Require(!PhotonNetwork.InRoom || PhotonNetwork.OfflineMode, "Smoke does not interrupt an online room");
        var armsRace = GM_ArmsRace.instance;
        if (!armsRace)
        {
            var candidates = Resources.FindObjectsOfTypeAll<GM_ArmsRace>()
                .Where(mode => mode && mode.gameObject.scene.IsValid() && mode.gameObject.scene.isLoaded).ToArray();
            report.Add("DIAG Loaded native Arms Race candidates with inactive singleton: " + candidates.Length);
            Require(candidates.Length == 1, "Exactly one loaded native Arms Race object exists for denied gate checks");
            armsRace = candidates[0];
        }
        var start = AccessTools.Method(typeof(GM_ArmsRace), "StartGame", Type.EmptyTypes);
        var coroutine = AccessTools.Method(typeof(GM_ArmsRace), "DoStartGame", Type.EmptyTypes);
        Require(HasGate(start) && HasGate(coroutine), "Real native start methods have installed guard prefixes");
        bool savedOffline = PhotonNetwork.OfflineMode;
        GameObject selection = null;
        try
        {
            PhotonNetwork.OfflineMode = false;
            Require(!PhotonNetwork.InRoom, "No-room gate test has no active Photon room");
            bool before = GameManager.instance.isPlaying;
            armsRace.StartGame();
            Require(GameManager.instance.isPlaying == before, "No-room native StartGame leaves game state unchanged");
            var empty = (IEnumerator)coroutine.Invoke(armsRace, null);
            Require(empty != null && !empty.MoveNext(), "No-room native DoStartGame returns an empty coroutine");
            var ready = AccessTools.Method(typeof(CharacterSelectionInstance), "ReadyUp", Type.EmptyTypes);
            Require(HasGate(ready), "Native readiness method has an installed guard prefix");
            selection = new GameObject("Canna Rebound disposable readiness fixture");
            selection.SetActive(false);
            var target = selection.AddComponent<CharacterSelectionInstance>();
            var readyFlag = AccessTools.Field(typeof(CharacterSelectionInstance), "isReady");
            bool previousReady = (bool)readyFlag.GetValue(target);
            target.ReadyUp();
            Require((bool)readyFlag.GetValue(target) == previousReady, "No-room readiness body has no native ready-state side effect");
        }
        finally
        {
            PhotonNetwork.OfflineMode = savedOffline;
            if (selection) UnityEngine.Object.Destroy(selection);
        }
        Require(PhotonNetwork.OfflineMode == savedOffline, "Original Photon offline mode is restored after gate checks");
    }

    static bool HasGate(MethodBase method)
    {
        var patches = method == null ? null : Harmony.GetPatchInfo(method);
        return patches != null && patches.Prefixes.Any(p => p.owner == "canna.ducttapeplusplus.networkguard");
    }

    static bool HasLoadedNativeMainScene()
    {
        return GameManager.instance && MainMenuHandler.instance
            && GameManager.instance.gameObject.scene.IsValid() && GameManager.instance.gameObject.scene.isLoaded
            && MainMenuHandler.instance.gameObject.scene.IsValid() && MainMenuHandler.instance.gameObject.scene.isLoaded;
    }

    void TrySkipLoadedIntro()
    {
        var candidates = Resources.FindObjectsOfTypeAll<SkipIntro>()
            .Where(intro => intro && intro.gameObject.activeInHierarchy && intro.gameObject.scene.IsValid() && intro.gameObject.scene.isLoaded).ToArray();
        report.Add("DIAG Active loaded native intro candidates: " + candidates.Length);
        if (candidates.Length != 1) return;
        var shown = AccessTools.Field(typeof(SkipIntro), "hasShown");
        var target = AccessTools.Field(typeof(SkipIntro), "target");
        var menu = AccessTools.Field(typeof(ListMenu), "instance");
        if (shown == null || target == null || menu == null || (bool)shown.GetValue(null)
            || target.GetValue(candidates[0]) == null || !(menu.GetValue(null) as UnityEngine.Object)) return;
        // Actual native Skip only marks hasShown and calls ListMenu.OpenPage(target).
        // The root-controlled test copy is allowed to advance this identified intro.
        candidates[0].Skip();
        Check((bool)shown.GetValue(null), "Native intro skip marks the existing loaded intro as shown");
    }

    void SceneDiagnostics(string stage)
    {
        var active = SceneManager.GetActiveScene();
        report.Add("DIAG " + stage + " active scene=" + SceneName(active.name) + "; build-index=" + active.buildIndex
            + "; valid=" + active.IsValid() + "; loaded=" + active.isLoaded + "; scene-count=" + SceneManager.sceneCount);
        for (int i = 0; i < Math.Min(SceneManager.sceneCount, 16); i++)
        {
            var scene = SceneManager.GetSceneAt(i);
            report.Add("DIAG " + stage + " loaded scene=" + SceneName(scene.name) + "; build-index=" + scene.buildIndex
                + "; valid=" + scene.IsValid() + "; loaded=" + scene.isLoaded);
        }
        report.Add("DIAG " + stage + " GameManager=" + (bool)GameManager.instance + "; ArmsRace=" + (bool)GM_ArmsRace.instance
            + "; MainMenu=" + (bool)MainMenuHandler.instance + "; isPlaying=" + (GameManager.instance ? GameManager.instance.isPlaying.ToString() : "unavailable")
            + "; native-main-scene-ready=" + HasLoadedNativeMainScene());
    }
    static string SceneName(string name)
    { return name == null ? "none" : new string(name.Take(96).Select(c => c >= ' ' && c <= '~' ? c : '?').ToArray()); }

    static bool HasRuntimePrefix(MethodBase method, string typeName)
    {
        var patches = method == null ? null : Harmony.GetPatchInfo(method);
        return patches != null && patches.Prefixes.Any(p => p.owner.StartsWith("rounds-port.runtime.", StringComparison.Ordinal)
            && p.PatchMethod.DeclaringType != null && p.PatchMethod.DeclaringType.Name == typeName
            && p.PatchMethod.DeclaringType.Assembly.GetName().Name == "rounds-port.Runtime");
    }

    void StartSandbox()
    {
        var library = AppDomain.CurrentDomain.GetAssemblies().Single(a => a.GetName().Name == "UnboundLib");
        var manager = library.GetType("UnboundLib.GameModes.GameModeManager", true);
        string sandbox = (string)manager.GetField("SandBoxID", BindingFlags.Public | BindingFlags.Static).GetRawConstantValue();
        manager.GetMethod("SetGameMode", new[] { typeof(string) }).Invoke(null, new object[] { sandbox });
        var handler = manager.GetProperty("CurrentHandler", BindingFlags.Public | BindingFlags.Static).GetValue(null, null);
        Require(handler != null, "Real Unbound sandbox handler exists");
        Require(handler.GetType().FullName == "UnboundLib.GameModes.SandboxHandler", "Actual Unbound current handler is the public sandbox handler");
        // Setting OfflineMode alone does not create a Photon room. Use the same native
        // setup called by MainMenuHandler.PlaySandbox: SetOffline joins a local room.
        var offline = UnityEngine.Object.FindObjectOfType<SetOfflineMode>(true);
        Require(offline && offline.gameObject.scene.IsValid() && offline.gameObject.scene.isLoaded,
            "Native offline setup object exists in the loaded gameplay scene");
        offline.SetOffline();
        MainMenuHandler.instance.Close();
        handler.GetType().GetMethod("StartGame", Type.EmptyTypes).Invoke(handler, null);
        Require(PhotonNetwork.OfflineMode, "Offline sandbox path is allowed by the guard");
    }

    static bool MapReady()
    { return MapManager.instance && MapManager.instance.currentMap != null && MapManager.instance.currentMap.Map; }

    void CheckPlayer()
    {
        var player = PlayerManager.instance.players.First(p => p && p.data && p.data.healthHandler);
        smokePlayer = player;
        var fixture = AppDomain.CurrentDomain.GetAssemblies().Select(a => a.GetType("Fixtures.LegacyCalls", false)).FirstOrDefault(t => t != null);
        if (fixture == null)
        {
            // This dependency-free fixture intentionally has no BepInPlugin entry point.
            // Load only the exact reviewed test assembly from the prepared Canna subtree.
            var files = Directory.GetFiles(Path.Combine(Paths.PluginPath, "Canna"), "DependencyFreeLegacy.dll", SearchOption.AllDirectories);
            Require(files.Length == 1, "Exactly one prepared translated legacy fixture is present");
            fixture = Assembly.LoadFrom(files[0]).GetType("Fixtures.LegacyCalls", true);
        }
        Require(fixture != null, "Translated dependency-free legacy fixture is loaded");
        var read = fixture.GetMethod("Read", BindingFlags.Public | BindingFlags.Static, null, new[] { typeof(Player) }, null);
        Require(read != null && (int)read.Invoke(null, new object[] { player }) == player.PlayerID, "Translated legacy playerID access matches native PlayerID");
        Require(player.data.MaxHealth > 0, "Current public max-health property is accessible");
        float health = player.data.health;
        ((Damagable)player.data.healthHandler).TakeDamage(Vector2.zero, player.transform.position, null, player, false, false, HealthHandler.DamageSource.Player);
        Require(player.data.health == health, "Current damage-source signature accepts zero damage without changing health");
    }

    void PrepareCosmicCard()
    {
        var plugin = BepInEx.Bootstrap.Chainloader.PluginInfos[CosmicGuid];
        Require(plugin.Instance && plugin.Metadata.Version.ToString() == "2.7.0", "Actual CosmicRounds GUID is loaded at reviewed version 2.7.0");
        var assembly = plugin.Instance.GetType().Assembly;
        Require(assembly.GetName().Name == "CosmicRounds", "CosmicRounds plugin resolves to its actual assembly");
        var library = AppDomain.CurrentDomain.GetAssemblies().Single(a => a.GetName().Name == "UnboundLib");
        var custom = library.GetType("UnboundLib.Cards.CustomCard", true);
        var speed = assembly.GetType("CR.Cards.SpeedUpCard", true);
        Require(custom.IsAssignableFrom(speed), "Reviewed CR Speed Up type derives from actual Unbound CustomCard");
        var cards = CardChoice.instance.cards.Where(card => card && card.GetComponent(custom)
            && card.GetComponent(custom).GetType().Assembly == assembly).ToArray();
        Require(cards.Length > 0, "Actual CosmicRounds custom cards exist in the native registered card array");
        var matches = cards.Where(card => card.GetComponent(speed)).ToArray();
        Require(matches.Length == 1, "Exactly one reviewed CR Speed Up prototype is registered");
        cosmicPrototype = matches[0];
        Require(cosmicPrototype.CardName == "Speed Up" && !String.IsNullOrEmpty(cosmicPrototype.CardDescription),
            "Registered CosmicRounds title and description resolve through current localized getters");
        prototypeLocalizedName = AccessTools.Property(typeof(CardInfo), "LocalizedCardName").GetValue(cosmicPrototype, null);
        prototypeLocalizedDescription = AccessTools.Property(typeof(CardInfo), "LocalizedCardDescription").GetValue(cosmicPrototype, null);
        prototypeStats = cosmicPrototype.cardStats; prototypeActive = cosmicPrototype.gameObject.activeSelf;
        prototypeRegistryCount = CardChoice.instance.cards.Length;
        Require(prototypeLocalizedName != null && prototypeLocalizedDescription != null && prototypeStats.Length == 3,
            "CR Speed Up prototype has current localized data and three reviewed stat rows");
        temporaryCard = CardChoice.instance.AddCardVisual(cosmicPrototype, new Vector3(0f, 1000f, 0f));
        Require(temporaryCard && temporaryCard != cosmicPrototype.gameObject, "Native AddCardVisual creates a disposable CR clone away from gameplay");
        var info = temporaryCard.GetComponent<CardInfo>();
        Require(info && info.GetComponent(speed), "Disposable native card clone retains the real CR custom-card component");
        info.sourceCard = cosmicPrototype;
        temporaryCard.SetActive(true);
    }

    void DrawCosmicCard()
    {
        var displays = temporaryCard.GetComponentsInChildren<CardInfoDisplayer>(true);
        Require(displays.Length == 1, "Disposable CR card owns exactly one native card displayer");
        cosmicDisplayer = displays[0];
        report.Add("DIAG Actual CR frame localized references: name="
            + (AccessTools.Field(typeof(CardInfoDisplayer), "m_localizedNameText").GetValue(cosmicDisplayer) != null)
            + "; description=" + (AccessTools.Field(typeof(CardInfoDisplayer), "m_localizedEffectText").GetValue(cosmicDisplayer) != null));
        Require(cosmicDisplayer.transform.IsChildOf(temporaryCard.transform), "Native card displayer belongs to the disposable clone");
        Require(HasRuntimePrefix(AccessTools.Method(typeof(CardInfoDisplayer), "DrawCard"), "OldCardFrame_Fix"),
            "Upstream legacy-frame prefix is installed on actual native DrawCard");
        InvokeCosmicDraw();
        Require(true, "Actual registered CosmicRounds clone completes current native DrawCard");
    }

    void InvokeCosmicDraw()
    {
        var info = temporaryCard.GetComponent<CardInfo>();
        var method = AccessTools.Method(typeof(CardInfoDisplayer), "DrawCard");
        Require(method != null && method.GetParameters().Length == 5, "Current native DrawCard has the inspected five-argument contract");
        method.Invoke(cosmicDisplayer, new object[] { info.cardStats,
            AccessTools.Property(typeof(CardInfo), "LocalizedCardName").GetValue(info, null),
            AccessTools.Property(typeof(CardInfo), "LocalizedCardDescription").GetValue(info, null), null, false });
    }

    void CheckCosmicCardText(string phase, bool legacyFixture)
    {
        Require(temporaryCard && cosmicDisplayer, phase + " retains the disposable card and displayer");
        var name = FrameText("nameText", "m_localizedNameText");
        var description = FrameText("effectText", "m_localizedEffectText");
        string expectedName = legacyFixture ? cosmicPrototype.CardName.ToUpper() : cosmicPrototype.CardName;
        report.Add("DIAG " + phase + " public title actual=" + BoundedCardText(ReadText(name)) + "; expected=" + BoundedCardText(expectedName));
        report.Add("DIAG " + phase + " public description actual=" + BoundedCardText(ReadText(description))
            + "; expected=" + BoundedCardText(cosmicPrototype.CardDescription));
        report.Add("DIAG " + phase + " plain title=" + BoundedCardText(ReadText(AccessTools.Field(typeof(CardInfoDisplayer), "nameText").GetValue(cosmicDisplayer) as Component))
            + "; localized title=" + BoundedCardText(ReadText(LocalizedFrameText("m_localizedNameText"))));
        Require(ReadText(name) == expectedName, phase + " shows the registered title using the inspected text-case contract");
        Require(ReadText(description) == cosmicPrototype.CardDescription, phase + " shows the registered description");
        var grid = AccessTools.Field(typeof(CardInfoDisplayer), "grid").GetValue(cosmicDisplayer) as GameObject;
        var template = AccessTools.Field(typeof(CardInfoDisplayer), "statObject").GetValue(cosmicDisplayer) as GameObject;
        Require(grid && template && grid.transform.IsChildOf(temporaryCard.transform) && template.transform.IsChildOf(temporaryCard.transform),
            phase + " stat grid and template belong to the disposable clone");
        var rows = grid.transform.Cast<Transform>().Where(row => row.name == template.name + "(Clone)").ToArray();
        Require(rows.Length >= prototypeStats.Length, phase + " creates all reviewed stat rows");
        var latest = rows.Skip(rows.Length - prototypeStats.Length).ToArray();
        for (int i = 0; i < latest.Length; i++)
        {
            var text = latest[i].GetChild(0).GetComponents<Component>().FirstOrDefault(component => component && component.GetType().FullName == "TMPro.TextMeshProUGUI");
            Require(ReadText(text) == prototypeStats[i].stat, phase + " retains reviewed stat label " + (i + 1));
        }
    }

    Component FrameText(string plain, string localized)
    {
        // Modern DrawCard writes UILocalizedString.Text, while the forced legacy
        // fixture writes plain fields. Select the actual writer, not a stale field.
        var text = LocalizedFrameText(localized);
        if (text) return text;
        return AccessTools.Field(typeof(CardInfoDisplayer), plain).GetValue(cosmicDisplayer) as Component;
    }
    Component LocalizedFrameText(string localized)
    {
        var reference = AccessTools.Field(typeof(CardInfoDisplayer), localized).GetValue(cosmicDisplayer) as Component;
        return reference == null ? null : AccessTools.Property(reference.GetType(), "Text").GetValue(reference, null) as Component;
    }
    static string BoundedCardText(string text)
    { return text == null ? "<none>" : "\"" + new string(text.Take(160).Select(c => c >= ' ' && c <= '~' ? c : '?').ToArray()) + "\""; }
    static string ReadText(Component component)
    {
        if (!component) return null;
        var property = AccessTools.Property(component.GetType(), "text");
        return property == null ? null : property.GetValue(component, null) as string;
    }

    void DrawLegacyCosmicFrame()
    {
        var name = FrameText("nameText", "m_localizedNameText");
        var effect = FrameText("effectText", "m_localizedEffectText");
        Require(name && effect && name.transform.IsChildOf(temporaryCard.transform) && effect.transform.IsChildOf(temporaryCard.transform),
            "Simulated legacy frame changes only disposable clone text references");
        AccessTools.Field(typeof(CardInfoDisplayer), "nameText").SetValue(cosmicDisplayer, name);
        AccessTools.Field(typeof(CardInfoDisplayer), "effectText").SetValue(cosmicDisplayer, effect);
        AccessTools.Field(typeof(CardInfoDisplayer), "m_localizedNameText").SetValue(cosmicDisplayer, null);
        AccessTools.Field(typeof(CardInfoDisplayer), "m_localizedEffectText").SetValue(cosmicDisplayer, null);
        InvokeCosmicDraw();
        Require(true, "Simulated missing-localized-reference frame completes actual patched DrawCard");
    }

    void ApplyCosmicCard()
    {
        Require(PhotonNetwork.OfflineMode && PhotonNetwork.InRoom && smokePlayer && smokePlayer.data && smokePlayer.data.stats,
            "Ordinary CR card applies only to the disposable offline player");
        var ammo = smokePlayer.data.weaponHandler.gun.GetComponentInChildren<GunAmmo>();
        var apply = temporaryCard.GetComponent<ApplyCardStats>();
        Require(ammo && apply && temporaryCard.GetComponent<CardInfo>().sourceCard == cosmicPrototype,
            "Disposable CR pick has native stats, ammo and canonical source ownership");
        float movement = smokePlayer.data.stats.movementSpeed, reload = ammo.reloadTimeMultiplier;
        int count = smokePlayer.data.currentCards.Count;
        apply.OFFLINE_Pick(new[] { smokePlayer });
        Require(smokePlayer.data.currentCards.Count == count + 1 && smokePlayer.data.currentCards.Contains(cosmicPrototype),
            "Native CR OFFLINE_Pick adds the canonical card to the disposable player's current cards");
        Require(Mathf.Approximately(smokePlayer.data.stats.movementSpeed, movement * 1.35f),
            "Native CR Speed Up pick applies its reviewed movement multiplier");
        Require(Mathf.Approximately(ammo.reloadTimeMultiplier, reload * .8f),
            "Real CR OnAddCard callback applies its reviewed reload multiplier");
        Require(smokePlayer && smokePlayer.data.health > 0f && !Single.IsNaN(smokePlayer.data.MaxHealth) && !Single.IsInfinity(smokePlayer.data.MaxHealth),
            "Disposable offline player remains alive with valid health after the ordinary CR pick");
    }

    void CleanupTemporaryCard()
    { if (temporaryCard) { temporaryCard.SetActive(false); UnityEngine.Object.Destroy(temporaryCard); } }
    void CheckCosmicPrototype()
    {
        Require(!temporaryCard, "Disposable CR card clone is destroyed after draw and application");
        Require(cosmicPrototype && CardChoice.instance.cards.Length == prototypeRegistryCount && CardChoice.instance.cards.Contains(cosmicPrototype)
            && cosmicPrototype.gameObject.activeSelf == prototypeActive && ReferenceEquals(cosmicPrototype.cardStats, prototypeStats)
            && ReferenceEquals(AccessTools.Property(typeof(CardInfo), "LocalizedCardName").GetValue(cosmicPrototype, null), prototypeLocalizedName)
            && ReferenceEquals(AccessTools.Property(typeof(CardInfo), "LocalizedCardDescription").GetValue(cosmicPrototype, null), prototypeLocalizedDescription),
            "Canonical CR prototype, registry, active state and localized references remain untouched after cleanup");
    }

    bool Run(string name, Action action)
    {
        try { action(); return true; }
        catch (Exception ex)
        {
            var cause = ex is TargetInvocationException && ex.InnerException != null ? ex.InnerException : ex;
            report.Add("FAIL " + name + " (" + cause.GetType().Name + ")");
            report.Add("DIAG Failure HRESULT: " + cause.HResult.ToString("x8"));
            var frames = new System.Diagnostics.StackTrace(cause, false).GetFrames();
            if (frames != null)
                foreach (var frame in frames.Take(8))
                {
                    var method = frame.GetMethod();
                    if (method == null) continue;
                    string label = (method.DeclaringType == null ? "unknown" : method.DeclaringType.FullName) + "." + method.Name;
                    report.Add("DIAG Failure method: " + new string(label.Take(160).Select(c => c >= ' ' && c <= '~' ? c : '?').ToArray()));
                }
            Logger.LogError("Canna Rebound smoke failed: " + name + " (" + cause.GetType().Name + ")");
            return false;
        }
    }

    void Require(bool condition, string name) { if (!Check(condition, name)) throw new InvalidOperationException(name); }
    bool Check(bool condition, string name) { report.Add((condition ? "PASS " : "FAIL ") + name); return condition; }
    void Finish()
    {
        if (finished) return;
        finished = true;
        CleanupTemporaryCard();
        Directory.CreateDirectory(output);
        File.WriteAllLines(Path.Combine(output, "report.txt"), report);
        Logger.LogInfo("Canna Rebound smoke complete: " + report.Count(r => r.StartsWith("PASS ")) + " assertions; " + report.Count(r => r.StartsWith("FAIL ")) + " failures.");
        Application.Quit();
    }
}
