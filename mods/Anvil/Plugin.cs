using System;
using System.Collections.Generic;
using System.Reflection;
using BepInEx;
using BepInEx.Logging;
using BoplFixedMath;
using HarmonyLib;
using UnityEngine;

namespace Canna.Anvil
{
    [BepInPlugin("family.canna.anvil", "Canna Anvil", "1.0.7")]
    public sealed class Plugin : BaseUnityPlugin
    {
        internal static ManualLogSource Log;
        internal static GameObject Prefab;
        internal static Material Steel;
        internal const string Protocol = "canna-anvil-1.0.7";
        void Awake()
        {
            Log = Logger;
            Art.Create();
            Steel = new Material(Shader.Find("Sprites/Default"));
            UnityEngine.Object.DontDestroyOnLoad(Steel);
            new Harmony("family.canna.anvil").PatchAll(typeof(Plugin).Assembly);
            Logger.LogInfo("Anvil 1.0.7 loaded: separate selectable ability, native Rock lifecycle, native gravity and free rotation and 17-frame morph. All online players need the same mod.");
        }
        internal static void Register(NamedSpriteList list)
        {
            if (list == null || list.sprites == null || list.IndexOf("Anvil") >= 0) return;
            NamedSprite source = list.sprites.Find(delegate(NamedSprite entry) {
                return entry.associatedGameObject != null && entry.associatedGameObject.GetComponent<BounceBall>() != null;
            });
            // Do not expose a locked DLC/demo ability or change an unrelated sprite list.
            if (source.associatedGameObject == null) return;
            Art.CreateIcon(source.sprite);
            if (Prefab == null)
            {
                GameObject root = new GameObject("Canna Anvil prefab container");
                root.SetActive(false);
                UnityEngine.Object.DontDestroyOnLoad(root);
                Prefab = UnityEngine.Object.Instantiate(source.associatedGameObject, root.transform);
                Prefab.name = "Anvil";
                Prefab.AddComponent<AnvilState>();
                Prefab.SetActive(true); // Inactive parent prevents native Awake on the template.
                BoplBody body = Prefab.GetComponent<BoplBody>();
                FieldInfo mass = AccessTools.Field(typeof(BoplBody), "mass");
                mass.SetValue(body, (Fix)mass.GetValue(body) * (Fix)2L);
                body.gravityScale = Fix.One;
                body.bounciness = (Fix)1L/(Fix)10L;
                body.friction = (Fix)4L/(Fix)5L;
                body.dynamicFriction = (Fix)3L/(Fix)5L;
                // Retain native friction, bounce and angular physics. Mass remains
                // twice Rock's; gravity must not add an artificial acceleration boost.
                DPhysicsCircle hull=Prefab.GetComponent<DPhysicsCircle>();
                FieldInfo startRadius=AccessTools.Field(typeof(DPhysicsCircle),"startRadius");
                // Artwork bounds: x=11..107, y=22..90, centered at (59,56).
                // Only the box is registered; the circle is a native ability API facade.
                startRadius.SetValue(hull,(Fix)17L/(Fix)12L);
                DPhysicsBox box=Prefab.AddComponent<DPhysicsBox>();
                Prefab.GetComponent<FixTransform>().offset=Vec2.zero;
                AccessTools.Field(typeof(DPhysicsBox),"startExtents").SetValue(box,new Vec2((Fix)2L,(Fix)17L/(Fix)12L));
                AccessTools.Field(typeof(DPhysicsBox),"manuallyCallInit").SetValue(box,true);
                box.MinScale=hull.MinScale;box.MaxScale=hull.MaxScale;
                Fix boxMass=(Fix)mass.GetValue(body);
                AccessTools.Field(typeof(BoplBody),"momentOfInertia").SetValue(body,boxMass*((Fix)4L+(Fix)289L/(Fix)144L)/(Fix)3L);
                Prefab.GetComponent<Ability>().Cooldown = (Fix)6L;
                AccessTools.Field(typeof(BounceBall), "maxDuration").SetValue(Prefab.GetComponent<BounceBall>(), (Fix)5L);
                FieldInfo exitTime = AccessTools.Field(typeof(BounceBall), "exitTime");
                BounceBall ball = Prefab.GetComponent<BounceBall>();
                exitTime.SetValue(ball, (Fix)exitTime.GetValue(ball) * (Fix)2L / (Fix)3L);
                AccessTools.Field(typeof(BounceBall), "playerCollisionBounce").SetValue(Prefab.GetComponent<BounceBall>(), (Fix)2L);
                Prefab.GetComponent<SpriteRenderer>().sprite = Art.Frames[16];
            }
            // Native menus, pickups, random abilities and network packets all use this list.
            // Append only: existing native ability indices remain unchanged.
            list.sprites.Add(new NamedSprite("Anvil", Art.Icon, Prefab, true));
            Log.LogInfo("Registered Anvil in " + list.name + " at native ability index " + (list.sprites.Count - 1));
        }
        internal static void RegisterFrom(object owner)
        {
            foreach (FieldInfo field in AccessTools.GetDeclaredFields(owner.GetType()))
                if (field.FieldType == typeof(NamedSpriteList)) Register((NamedSpriteList)field.GetValue(owner));
        }
        internal static string LobbyProtocol(SteamManager manager)
        {
            NamedSpriteList list = (NamedSpriteList)AccessTools.Field(typeof(SteamManager), "abilityIconsFull").GetValue(manager);
            uint hash = 2166136261;
            // Native packets transmit byte indices. Matching mod versions alone is insufficient
            // when another ability mod changes list order on one participant's machine.
            foreach (NamedSprite entry in list.sprites)
            {
                foreach (char c in entry.name) hash = unchecked((hash ^ c) * 16777619);
                hash = unchecked((hash ^ 0) * 16777619);
            }
            return Protocol + ":" + hash.ToString("x8");
        }
    }
    public sealed class AnvilState : MonoBehaviour
    {
        static readonly FieldInfo Exiting = AccessTools.Field(typeof(BounceBall), "IsExiting");
        static readonly FieldInfo EnterTime = AccessTools.Field(typeof(BounceBall), "timeSinceActivation");
        static readonly FieldInfo ExitTime = AccessTools.Field(typeof(BounceBall), "timeSinceExitStarted");
        static readonly FieldInfo ExitDuration = AccessTools.Field(typeof(BounceBall), "exitTime");
        internal BounceBall Ball;
        internal SpriteRenderer Renderer;
        internal Color SlimeColor = new Color(.8f, .9f, .6f, 1);
        internal static readonly Fix MorphDuration=(Fix)1L/(Fix)15L;
        internal void Paint()
        {
            if (Ball == null) Ball = GetComponent<BounceBall>();
            if (Renderer == null) Renderer = GetComponent<SpriteRenderer>();
            bool exiting = (bool)Exiting.GetValue(Ball);
            Fix time = (Fix)(exiting ? ExitTime : EnterTime).GetValue(Ball);
            Fix duration = exiting ? (Fix)ExitDuration.GetValue(Ball) : MorphDuration;
            int frame = Mathf.Clamp((int)((float)(time / duration) * 16), 0, 16);
            if (exiting) frame = 16 - frame;
            Renderer.sharedMaterial = Plugin.Steel;
            Renderer.sprite = Art.Frames[frame];
            Renderer.color = Color.Lerp(SlimeColor, Color.white, frame / 16f);
        }
    }
    [HarmonyPatch]
    static class RegisterAbility
    {
        static IEnumerable<MethodBase> TargetMethods()
        {
            foreach (Type type in new Type[] { typeof(SteamManager), typeof(SelectAbility), typeof(AbilityGrid), typeof(MidGameAbilitySelect), typeof(SlimeController), typeof(RandomAbility), typeof(DynamicAbilityPickup) })
                yield return AccessTools.Method(type, "Awake");
        }
        [HarmonyPriority(Priority.First)]
        static void Prefix(object __instance) { Plugin.RegisterFrom(__instance); }
    }
    [HarmonyPatch(typeof(BounceBall), "OnEnterAbility")]
    static class EnterAnvil
    {
        static void Postfix(BounceBall __instance, PlayerInfo ___playerInfo)
        {
            AnvilState state = __instance.GetComponent<AnvilState>();
            if (state == null) return;
            Material material = ___playerInfo.playerMaterial;
            if (material != null && material.HasProperty("_ShadowColor")) state.SlimeColor = material.GetColor("_ShadowColor");
            else if (material != null && material.HasProperty("_Color")) state.SlimeColor = material.color;
            state.Paint();
        }
    }
    [HarmonyPatch(typeof(BounceBall), "LateUpdateSim")]
    static class AnimateAnvil
    {
        // Timed transformation: releasing the activation button must not shorten
        // the requested five-second duration. Native timeout/death still exit it.
        static void Prefix(BounceBall __instance, Fix ___timeSinceActivation, ref bool ___IsCancellable)
        {
            if(__instance.GetComponent<AnvilState>()!=null)
                ___IsCancellable=false;
        }
        static void Postfix(BounceBall __instance)
        {
            AnvilState state = __instance.GetComponent<AnvilState>();
            if (state == null || !__instance.gameObject.activeInHierarchy) return;
            state.Paint();
        }
    }
    [HarmonyPatch(typeof(SpriteAnimator), "UpdateAnimations")]
    static class KeepAnvilSprite
    {
        static bool Prefix(SpriteAnimator __instance) { return __instance.GetComponent<AnvilState>() == null; }
    }
    [HarmonyPatch(typeof(LocalizationTable), "GetText", new Type[] { typeof(string), typeof(Language) })]
    static class AnvilName
    {
        static bool Prefix(string enText, ref string __result)
        {
            if (enText != "Anvil") return true;
            __result = "Anvil"; return false;
        }
    }
    [HarmonyPatch(typeof(SteamManager), "Update")]
    static class Advertise
    {
        static float next;
        static void Postfix(SteamManager __instance)
        {
            if (Time.unscaledTime < next || __instance.currentLobby.Id.Value == 0) return;
            next = Time.unscaledTime + 1;
            __instance.currentLobby.SetMemberData("canna_anvil", Plugin.LobbyProtocol(__instance));
        }
    }
    [HarmonyPatch]
    static class MatchingLobbies
    {
        static IEnumerable<MethodBase> TargetMethods()
        {
            yield return AccessTools.Method(typeof(SteamManager), "HostGame");
            yield return AccessTools.Method(typeof(SteamManager), "HostNextLevel");
        }
        static bool Prefix(SteamManager __instance)
        {
            if (__instance.currentLobby.Id.Value == 0 || __instance.currentLobby.MemberCount < 2) return true;
            string expected = Plugin.LobbyProtocol(__instance);
            __instance.currentLobby.SetMemberData("canna_anvil", expected);
            foreach (Steamworks.Friend member in __instance.currentLobby.Members)
                if (__instance.currentLobby.GetMemberData(member, "canna_anvil") != expected)
                {
                    Plugin.Log.LogWarning("Online round blocked: " + member.Name + " needs Canna Anvil 1.0.7 and the same ability list/order. Wait a moment after joining, then retry.");
                    return false;
                }
            return true;
        }
    }
}





