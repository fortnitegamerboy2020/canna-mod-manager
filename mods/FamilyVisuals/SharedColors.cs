using System;
using System.Globalization;
using System.Reflection;
using System.Collections.Generic;
using BepInEx;
using BepInEx.Configuration;
using BoplFixedMath;
using HarmonyLib;
using UnityEngine;
using UnityEngine.SceneManagement;
using UnityEngine.InputSystem;
namespace Canna.FamilyColors
{
    [BepInPlugin("family.canna.sharedcolors", "Canna Shared Colors", "1.0.0")]
    [BepInDependency("com.thepro.CustomLocalColors")]
    public class Plugin : BaseUnityPlugin
    {
        internal static ConfigEntry<string> Hex;
        internal static ConfigFile Settings;
        internal static Color[] Local = new Color[4];
        internal static Type Original;
        internal static bool Open;
        internal static float Next;
        internal static float R=.9f,G=.65f,B=.25f;
        internal static Dictionary<Player,Material> Materials=new Dictionary<Player,Material>();
        void Awake()
        {
            Settings=Config;
            Hex=Config.Bind("Colors","OnlineColor","","Your online color as #RRGGBB. Empty uses the existing local color picker or your normal game color. F8 opens a color panel.");
            Original=AccessTools.TypeByName("CustomLocalColors.Plugin");
            // Upstream changes shared native materials and indexes online colors by local slot.
            // Replace only that hook; keep its mouse/controller color picker and original assets.
            Harmony harmony=new Harmony("family.canna.sharedcolors");
            harmony.Unpatch(AccessTools.Method(typeof(GameSessionHandler),"SpawnPlayers"),HarmonyPatchType.Prefix,"com.thepro.CustomLocalColors");
            foreach(MethodBase method in new MethodBase[] {
                AccessTools.Method(typeof(SlimeController),"Spawn"),
                AccessTools.Method(typeof(PlayerCollision),"SpawnClone"),
                AccessTools.Method(typeof(PlayerCollision),"SpawnAbilityLessClone"),
                AccessTools.PropertySetter(typeof(Player),"Color") })
                harmony.Unpatch(method,HarmonyPatchType.All,"com.thepro.CustomLocalColors");
            harmony.PatchAll(typeof(Plugin).Assembly);
            SceneManager.sceneLoaded+=delegate(Scene scene,LoadSceneMode mode){InstallPanel();};
            InstallPanel();
            Logger.LogInfo("Shared Colors loaded. F8: online color panel. Steam lobby member data shares cosmetic colors; every viewer needs this mod.");
        }
        static void InstallPanel()
        {
            if(UnityEngine.Object.FindObjectOfType<ColorPanel>()==null)new GameObject("Canna color panel").AddComponent<ColorPanel>();
        }
        internal static string Encode(Color color)
        {return ColorUtility.ToHtmlStringRGB(color);}
        internal static bool Decode(string hex,out Color color)
        {
            color=Color.clear;
            return !string.IsNullOrEmpty(hex) && hex.Length<=7 && ColorUtility.TryParseHtmlString(hex.StartsWith("#")?hex:"#"+hex,out color);
        }
        internal static void SamplePicker()
        {
            if(Original==null)return;
            Array pickers=AccessTools.Field(Original,"colorPickers").GetValue(null) as Array;
            Color[] colors=AccessTools.Field(Original,"playerColors").GetValue(null) as Color[];
            for(int i=0;i<4;i++)
            {
                Component picker=pickers==null?null:pickers.GetValue(i) as Component;
                if(picker!=null && picker.gameObject.activeInHierarchy)
                {
                    Color c=(Color)AccessTools.Method(picker.GetType(),"GetColor").Invoke(picker,null);
                    Local[i]=c;if(colors!=null)colors[i]=c;
                }
                else if(colors!=null && colors[i].a>0)Local[i]=colors[i];
            }
        }
        internal static void Paint(Player player,Color color)
        {
            if(player==null || player.Color==null || color.a<=0)return;
            Material material;
            if(!Materials.TryGetValue(player,out material) || material==null)
            {
                material=new Material(player.Color); material.name="Canna shared color "+player.Id;
                Materials[player]=material;player.Color=material;
            }
            // Native slime shader's palette property, as used by the upstream picker.
            material.SetColor("_ShadowColor",color);
            player.Color=material;
            // A packet can arrive after spawning. Refresh live renderers as well as the
            // material that native future abilities/clones inherit from their player.
            foreach(SlimeController slime in UnityEngine.Object.FindObjectsOfType<SlimeController>())
                if(slime.GetPlayerId()==player.Id && slime.GetPlayerSprite()!=null && slime.GetPlayerSprite().material.HasProperty("_ShadowColor"))
                    slime.GetPlayerSprite().material.SetColor("_ShadowColor",color);
            foreach(Ability ability in UnityEngine.Object.FindObjectsOfType<Ability>())
                if(ability.GetPlayerId()==player.Id)
                {
                    SpriteRenderer renderer=ability.GetComponent<SpriteRenderer>();
                    if(renderer!=null && renderer.material.HasProperty("_ShadowColor"))renderer.material.SetColor("_ShadowColor",color);
                }
        }
        internal static void Apply(SteamManager manager)
        {
            foreach(Player player in PlayerHandler.Get().PlayerList())
            {
                Color color;
                if(GameLobby.isOnlineGame)
                {
                    if(manager==null || manager.currentLobby.Id.Value==0)continue;
                    foreach(Steamworks.Friend member in manager.currentLobby.Members)
                        if(member.Id.Value==player.steamId.Value && Decode(manager.currentLobby.GetMemberData(member,"canna_color_v1"),out color))Paint(player,color);
                }
                else if(player.Id>=1 && player.Id<=4)
                {
                    color=Local[player.Id-1];
                    bool[] random=Original==null?null:AccessTools.Field(Original,"playerRandoms").GetValue(null) as bool[];
                    if(random!=null && random[player.Id-1])
                    {
                        // Cosmetic randomness cannot consume Bopl's synchronized gameplay RNG.
                        Material previous;
                        if(Materials.TryGetValue(player,out previous) && previous!=null)color=previous.GetColor("_ShadowColor");
                        else
                        {
                            uint seed=unchecked((uint)Updater.SimulationTicks*2654435761u+(uint)player.Id*2246822519u);
                            color=Color.HSVToRGB((seed%10000)/10000f,.7f,.95f);
                        }
                    }
                    Paint(player,color);
                }
            }
        }
    }
    public class ColorPanel : MonoBehaviour
    {
        void Update(){if(Keyboard.current!=null && Keyboard.current.f8Key.wasPressedThisFrame)Plugin.Open=!Plugin.Open;}
        void OnGUI()
        {
            if(!Plugin.Open)return;
            GUILayout.BeginArea(new Rect(20,80,320,280),GUI.skin.box);
            GUILayout.Label("Family color · F8 to close");
            GUILayout.Label("Your color is shared with modded lobby members.");
            Plugin.R=GUILayout.HorizontalSlider(Plugin.R,0,1);GUILayout.Label("Red");
            Plugin.G=GUILayout.HorizontalSlider(Plugin.G,0,1);GUILayout.Label("Green");
            Plugin.B=GUILayout.HorizontalSlider(Plugin.B,0,1);GUILayout.Label("Blue");
            Color color=new Color(Plugin.R,Plugin.G,Plugin.B,1);
            GUI.color=color;GUILayout.Label("●  #"+Plugin.Encode(color));GUI.color=Color.white;
            if(GUILayout.Button("Use this color")){Plugin.Hex.Value=Plugin.Encode(color);Plugin.Settings.Save();Plugin.Next=0;}
            if(GUILayout.Button("Use normal color / local picker")){Plugin.Hex.Value="";Plugin.Settings.Save();Plugin.Next=0;}
            GUILayout.EndArea();
        }
    }
    [HarmonyPatch(typeof(SteamManager),"Update")]
    static class Share
    {
        internal static SteamManager Manager;
        static void Postfix(SteamManager __instance)
        {
            Manager=__instance;
            if(Time.unscaledTime<Plugin.Next)return;
            Plugin.Next=Time.unscaledTime+.5f;
            Plugin.SamplePicker();
            if(__instance.currentLobby.Id.Value!=0)
            {
                Color color;string value="";
                if(Plugin.Decode(Plugin.Hex.Value,out color))value=Plugin.Encode(color);
                else if(Plugin.Local[0].a>0)value=Plugin.Encode(Plugin.Local[0]);
                __instance.currentLobby.SetMemberData("canna_color_v1",value);
            }
            Plugin.Apply(__instance);
        }
    }
    [HarmonyPatch(typeof(GameSessionHandler),"SpawnPlayers")]
    static class Spawn
    {
        [HarmonyPriority(Priority.First)]
        static void Prefix(){Plugin.SamplePicker();Plugin.Apply(Share.Manager);}
    }
}

