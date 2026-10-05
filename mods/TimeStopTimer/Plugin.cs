using System;
using System.Collections.Generic;
using BepInEx;
using HarmonyLib;
using UnityEngine;
using UnityEngine.SceneManagement;
using BoplFixedMath;
using BepInEx.Configuration;

namespace CannaTimeStopTimerRepair {
    [BepInPlugin("local.canna.timestoptimer.repair", "TimeStopTimer Compatibility Repair", "1.0.1")]
    [BepInDependency("me.antimality.TimeStopTimer")]
    public class Plugin : BaseUnityPlugin {
        internal static ConfigEntry<string>[] LocalNames=new ConfigEntry<string>[4];
        private void Awake() {
            for(int i=0;i<4;i++)LocalNames[i]=Config.Bind("Local names","Player"+(i+1),"","Optional name for this local player. Empty uses your Steam name for keyboard/mouse, or a controller label.");
            // Keep the original package/profile intact; replace only its hooks.
            new Harmony("me.antimality.TimeStopTimer").UnpatchSelf();
            new Harmony("local.canna.timestoptimer.repair").PatchAll(typeof(Patch));
            TimerDisplay.Ensure();
            Logger.LogInfo("TimeStopTimer Canna 1.1.2: outlined text, online Steam names and configurable local names; native charge/duration, scene-safe hooks.");
        }
        // Bopl can destroy plugin Unity objects when changing scenes. Static
        // gameplay hooks must survive; the dedicated display recreates itself.
        private void OnDestroy() { }
    }

    public class TimerDisplay : MonoBehaviour {
        internal class Entry {
            internal UnityEngine.Object Source;
            internal int Player;
            internal string Phase;
            internal float Seconds;
        }
        private static readonly Dictionary<int, Entry> Entries = new Dictionary<int, Entry>();
        private static TimerDisplay display;
        private string scene;
        private GUIStyle text;
        internal static string PlayerName(int id) {
            Player player=PlayerHandler.Get()==null?null:PlayerHandler.Get().GetPlayer(id);
            if(GameLobby.isOnlineGame && player!=null && player.steamId.Value!=0) {
                string name=new Steamworks.Friend(player.steamId).Name;
                if(!string.IsNullOrEmpty(name))return CleanName(name);
            }
            if(!GameLobby.isOnlineGame && id>=1 && id<=4 && Plugin.LocalNames[id-1]!=null && !string.IsNullOrWhiteSpace(Plugin.LocalNames[id-1].Value))
                return CleanName(Plugin.LocalNames[id-1].Value);
            if(player!=null && player.UsesKeyboardAndMouse) {
                string name=Steamworks.SteamClient.Name;
                return string.IsNullOrEmpty(name)?"Keyboard & Mouse":CleanName(name);
            }
            return "Local "+id;
        }
        internal static string CleanName(string value) {
            var result=new System.Text.StringBuilder();
            foreach(char c in value){if(!char.IsControl(c))result.Append(c);if(result.Length>=32)break;}
            return result.ToString();
        }
        internal static void Ensure() {
            if (display != null) return;
            var root = new GameObject("TimeStopTimerPersistentOverlay");
            UnityEngine.Object.DontDestroyOnLoad(root);
            display = root.AddComponent<TimerDisplay>();
            display.scene = SceneManager.GetActiveScene().name;
        }
        internal static void Set(UnityEngine.Object source, int player, string phase, float seconds) {
            if (source == null || float.IsNaN(seconds) || float.IsInfinity(seconds)) return;
            Ensure();
            if (seconds <= 0f) { Remove(source); return; }
            Entries[source.GetInstanceID()] = new Entry { Source = source, Player = player, Phase = phase, Seconds = seconds };
        }
        internal static void Remove(UnityEngine.Object source) {
            if (source != null) Entries.Remove(source.GetInstanceID());
        }
        private void Update() {
            string next = SceneManager.GetActiveScene().name;
            if (next != scene) { Entries.Clear(); scene = next; }
            var dead = new List<int>();
            foreach (var pair in Entries) if (pair.Value.Source == null) dead.Add(pair.Key);
            foreach (int id in dead) Entries.Remove(id);
        }
        private void OnGUI() {
            if (Entries.Count == 0) return;
            if (text == null) {
                text = new GUIStyle(GUI.skin.label);
                text.richText=false;
                text.alignment = TextAnchor.MiddleCenter;
                text.normal.textColor = Color.white;
            }
            int row = 0;
            var ordered = new List<Entry>(Entries.Values);
            ordered.Sort((a,b) => a.Player != b.Player ? a.Player.CompareTo(b.Player) : string.CompareOrdinal(a.Phase,b.Phase));
            var oldColor = GUI.color;
            GUI.color = Color.white;
            float scale=Mathf.Clamp(Screen.height/1080f,.75f,2f);
            text.fontSize=Mathf.RoundToInt(25*scale);
            foreach (var entry in ordered) {
                string label=PlayerName(entry.Player)+"  "+entry.Phase+"  "+entry.Seconds.ToString("0.0")+"s";
                float width=Mathf.Min(Screen.width-16,text.CalcSize(new GUIContent(label)).x+24*scale);
                var rect = new Rect((Screen.width-width)/2,30*scale+row*42*scale,width,40*scale);
                text.normal.textColor=Color.black;
                float outline=2*scale;
                for(int y=-1;y<=1;y++)for(int x=-1;x<=1;x++)if(x!=0 || y!=0)
                    GUI.Label(new Rect(rect.x+x*outline,rect.y+y*outline,rect.width,rect.height),label,text);
                text.normal.textColor=Color.white;
                GUI.Label(rect,label,text);
                row++;
            }
            GUI.color = oldColor;
        }
        private void OnDestroy() { if (display == this) { display = null; Entries.Clear(); } }
    }

    internal static class Patch {
        private static bool IsTimeStop(GameObject spell) {
            return spell != null && (spell.GetComponent<TimeStop>() != null || spell.name.IndexOf("TimeStop",StringComparison.OrdinalIgnoreCase) >= 0);
        }
        [HarmonyPatch(typeof(CastSpell), "UpdateSim")]
        [HarmonyPostfix]
        private static void Charge(CastSpell __instance, GameObject ___spell, Fix ___castTime,
                Fix ___timeSinceActivation, PlayerInfo ___playerInfo, bool ___isCastingSpell, bool ___hasFired) {
            if (!IsTimeStop(___spell) || !___isCastingSpell || ___hasFired) {
                TimerDisplay.Remove(__instance); return;
            }
            TimerDisplay.Set(__instance, ___playerInfo.playerId, "Charging", (float)(___castTime - ___timeSinceActivation));
        }
        [HarmonyPatch(typeof(CastSpell), "ExitAbility", new Type[] { })]
        [HarmonyPostfix]
        private static void Exit(CastSpell __instance) { TimerDisplay.Remove(__instance); }
        [HarmonyPatch(typeof(CastSpell), "ExitAbility", new Type[] { typeof(AbilityExitInfo) })]
        [HarmonyPostfix]
        private static void ExitWithInfo(CastSpell __instance) { TimerDisplay.Remove(__instance); }
        [HarmonyPatch(typeof(TimeStop), "Init")]
        [HarmonyPostfix]
        private static void Start(TimeStop __instance, int ___casterId, float ___duration, float ___secondsElapsed) {
            TimerDisplay.Set(__instance, ___casterId, "Time stop", ___duration - ___secondsElapsed);
        }
        [HarmonyPatch(typeof(TimeStop), "UpdateSim")]
        [HarmonyPostfix]
        private static void Active(TimeStop __instance, int ___casterId, float ___duration, float ___secondsElapsed) {
            TimerDisplay.Set(__instance, ___casterId, "Time stop", ___duration - ___secondsElapsed);
        }
        [HarmonyPatch(typeof(TimeStop), "End")]
        [HarmonyPostfix]
        private static void End(TimeStop __instance) { TimerDisplay.Remove(__instance); }
    }
}
