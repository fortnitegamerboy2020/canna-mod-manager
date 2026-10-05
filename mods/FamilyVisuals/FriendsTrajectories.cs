using System;
using System.Collections.Generic;
using System.Reflection;
using BepInEx;
using BepInEx.Configuration;
using BoplFixedMath;
using HarmonyLib;
using UnityEngine;
using UnityEngine.SceneManagement;
using UnityEngine.InputSystem;
namespace Canna.FriendsTrajectories
{
    [BepInPlugin("family.canna.friendstrajectories", "Canna Friends Trajectories", "1.0.0")]
    [BepInDependency("com.Obelous.ArrowTrajectories")]
    public class Plugin : BaseUnityPlugin
    {
        internal static ConfigEntry<bool> Own,Team,Other;
        internal static ConfigFile Settings;
        internal static bool Open;
        internal static Dictionary<BowTransform,LineRenderer> Lines=new Dictionary<BowTransform,LineRenderer>();
        internal static FieldInfo Fired=AccessTools.Field(typeof(BowTransform),"hasFired");
        internal static FieldInfo Body=AccessTools.Field(typeof(BowTransform),"body");
        internal static FieldInfo Aim=AccessTools.Field(typeof(BowTransform),"inputVector");
        internal static FieldInfo Charge=AccessTools.Field(typeof(BowTransform),"loadingFrame");
        internal static FieldInfo Arrow=AccessTools.Field(typeof(BowTransform),"Arrow");
        internal static FieldInfo Speed=AccessTools.Field(typeof(BowTransform),"ArrowSpeed");
        internal delegate Vec2 Integrator(ref PhysicsBody body,Fix delta);
        internal static Integrator Integrate;
        internal static DetPhysics Physics;
        internal static float Next;
        void Awake()
        {
            Settings=Config;
            Own=Config.Bind("Visibility","OwnArrows",true,"Show your own trajectory, offline or in a private family lobby.");
            Team=Config.Bind("Visibility","TeammateArrows",false,"Show teammates' synchronized bow trajectories on this PC. F9 changes visibility.");
            Other=Config.Bind("Visibility","OpponentArrows",false,"Show other players' synchronized bow trajectories on this PC.");
            Harmony harmony=new Harmony("family.canna.friendstrajectories");
            harmony.Patch(AccessTools.Method(AccessTools.TypeByName("ArrowTrajectories.Plugin"),"Update"),new HarmonyMethod(typeof(Plugin),"SuppressOriginal"));
            harmony.PatchAll(typeof(Plugin).Assembly);
            SceneManager.sceneLoaded+=delegate(Scene scene,LoadSceneMode mode){Clear();InstallPanel();};InstallPanel();
            Logger.LogInfo("Friends Trajectories loaded. F9 configures own, teammate and opponent visibility. Native charge, launch velocity, gravity and integration; cosmetic prediction only.");
        }
        static bool SuppressOriginal(){return false;}
        static void InstallPanel(){if(UnityEngine.Object.FindObjectOfType<TrajectoryPanel>()==null)new GameObject("Canna trajectory panel").AddComponent<TrajectoryPanel>();}
        internal static bool Visible(bool local,bool teammate){return local?Own.Value:teammate?Team.Value:Other.Value;}
        internal static void Clear()
        {
            foreach(LineRenderer line in Lines.Values)if(line!=null){UnityEngine.Object.Destroy(line.sharedMaterial);UnityEngine.Object.Destroy(line.gameObject);}
            Lines.Clear();Physics=null;Integrate=null;
        }
        internal static void Launch(BowTransform bow,out Vec2 origin,out PhysicsBody physics)
        {
            PlayerBody body=(PlayerBody)Body.GetValue(bow);
            Fix scale=body.fixtrans.Scale;
            Fix speedScale=Fix.One+(scale-Fix.One)/(Fix)2L;
            origin=body.position+bow.FirepointOffset.x*body.right+bow.FirepointOffset.y*body.up;
            physics=((BoplBody)Arrow.GetValue(bow)).NewPhysicsBody(scale);
            physics.velocity=(Vec2)Aim.GetValue(bow)*((Fix)(long)(int)Charge.GetValue(bow)+Fix.One)*(Fix)Speed.GetValue(bow)*speedScale+body.selfImposedVelocity;
        }
        internal static void Draw()
        {
            if(Time.unscaledTime<Next)return;Next=Time.unscaledTime+1f/30f;
            DetPhysics current=DetPhysics.Get();if(current==null)return;
            if(current!=Physics)
            {
                Physics=current;
                Integrate=(Integrator)Delegate.CreateDelegate(typeof(Integrator),current,AccessTools.Method(typeof(DetPhysics),"IntegrateBody"));
            }
            List<BowTransform> removed=new List<BowTransform>();
            foreach(KeyValuePair<BowTransform,LineRenderer> pair in Lines)
                if(pair.Key==null || pair.Key.IsDestroyed || !pair.Key.gameObject.activeInHierarchy){if(pair.Value!=null){UnityEngine.Object.Destroy(pair.Value.sharedMaterial);UnityEngine.Object.Destroy(pair.Value.gameObject);}removed.Add(pair.Key);}
            foreach(BowTransform key in removed)Lines.Remove(key);
            List<Player> players=PlayerHandler.Get().PlayerList();
            foreach(BowTransform bow in UnityEngine.Object.FindObjectsOfType<BowTransform>())
            {
                Player owner=PlayerHandler.Get().GetPlayer(bow.GetComponent<Ability>().GetPlayerId());
                if(owner==null)continue;
                bool local=!GameLobby.isOnlineGame || owner.IsLocalPlayer;
                bool teammate=false;
                foreach(Player viewer in players)if(viewer.IsLocalPlayer && viewer.Team==owner.Team)teammate=true;
                LineRenderer line;
                bool visible=Visible(local,teammate) && !(bool)Fired.GetValue(bow) && !owner.isInvisible;
                if(!visible){if(Lines.TryGetValue(bow,out line) && line!=null)line.enabled=false;continue;}
                if(!Lines.TryGetValue(bow,out line) || line==null)
                {
                    line=new GameObject("Canna trajectory player "+owner.Id).AddComponent<LineRenderer>();
                    line.sharedMaterial=new Material(Shader.Find("Sprites/Default"));line.widthMultiplier=.075f;line.useWorldSpace=true;line.sortingOrder=30;Lines[bow]=line;
                }
                line.enabled=true;Color color=owner.Color!=null && owner.Color.HasProperty("_ShadowColor")?owner.Color.GetColor("_ShadowColor"):Color.white;
                color.a=.75f;line.startColor=color;color.a=.15f;line.endColor=color;
                Vec2 position;PhysicsBody body;Launch(bow,out position,out body);
                List<Vector3> points=new List<Vector3>();points.Add((Vector3)position);
                Fix delta=(Fix)1L/(Fix)60L;
                // Integrate a COPY using the game's exact fixed-point integrator. No spawned
                // projectiles, gameplay RNG, collisions or synchronized simulation writes.
                for(int i=0;i<180;i++)
                {
                    Vec2 offset=Integrate(ref body,delta);Fix distance=Vec2.Magnitude(offset);
                    if(distance>Fix.Zero)
                    {
                        RaycastInformation hit=current.RaycastToClosest(position,offset/distance,distance,LayerMask.GetMask("wall","terrain","lethalTerrain"));
                        if(hit.pp.fixTrans!=null){points.Add((Vector3)hit.nearPos);break;}
                    }
                    position+=offset;if(i%3==2)points.Add((Vector3)position);
                }
                line.positionCount=points.Count;line.SetPositions(points.ToArray());
            }
        }
    }
    public class TrajectoryPanel : MonoBehaviour
    {
        void Update(){if(Keyboard.current!=null && Keyboard.current.f9Key.wasPressedThisFrame)Plugin.Open=!Plugin.Open;}
        void OnGUI()
        {
            if(!Plugin.Open)return;
            GUILayout.BeginArea(new Rect(Screen.width-350,80,330,190),GUI.skin.box);
            GUILayout.Label("Arrow trajectories · F9 to close");
            Plugin.Own.Value=GUILayout.Toggle(Plugin.Own.Value,"Show my arrows");
            Plugin.Team.Value=GUILayout.Toggle(Plugin.Team.Value,"Show teammates' arrows");
            Plugin.Other.Value=GUILayout.Toggle(Plugin.Other.Value,"Show opponents' arrows");
            GUILayout.Label("Visibility is your local preference.\nMoving platforms, portals and black holes can\nchange the eventual path after release.");
            if(GUILayout.Button("Save"))Plugin.Settings.Save();GUILayout.EndArea();
        }
    }
    [HarmonyPatch(typeof(SteamManager),"Update")]
    static class Render
    {
        static void Postfix(){Plugin.Draw();}
    }
}

