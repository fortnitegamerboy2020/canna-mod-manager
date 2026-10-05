using System;
using System.IO;
using System.Reflection;
using System.Collections.Generic;
using BepInEx;
using BoplFixedMath;
using HarmonyLib;
using UnityEngine;
using UnityEngine.SceneManagement;
[BepInPlugin("family.canna.catalogaudit","Canna Catalog Audit","1.0.0")]
public class CatalogAudit : BaseUnityPlugin
{
 internal static bool Active;
 internal static int Stage;
 internal static float Next=8;
 internal static string Folder;
 internal static Type Colors,Trajectories;
 internal static Steamworks.Data.Lobby TestLobby;
 internal static System.Threading.Tasks.Task<Steamworks.Data.Lobby?> LobbyTask;
 void Awake(){Active=Array.IndexOf(Environment.GetCommandLineArgs(),"--canna-catalog-audit")>=0;if(!Active)return;Folder=Path.Combine(Paths.ConfigPath,"CannaCatalogAudit");Directory.CreateDirectory(Folder);new Harmony("family.canna.catalogaudit").PatchAll(typeof(CatalogAudit).Assembly);}
 internal static void Check(bool c,string message){if(!c)throw new Exception(message);File.AppendAllText(Path.Combine(Folder,"checks.txt"),"PASS "+message+"\n");}
 internal static object Call(Type type,string method,params object[] args){return AccessTools.Method(type,method).Invoke(null,args);}
 internal static void SetConfig(Type type,string name,bool value){object entry=AccessTools.Field(type,name).GetValue(null);entry.GetType().GetProperty("Value").SetValue(entry,value,null);}
}
[HarmonyPatch]
static class CatalogSuppress
{
 static IEnumerable<MethodBase> TargetMethods(){yield return AccessTools.Method(typeof(GameSessionHandler),"StartSpawnPlayersRoutine");yield return AccessTools.Method(typeof(GameSessionHandler),"Update");yield return AccessTools.Method(typeof(GameSessionHandler),"UpdateSim");}
 static bool Prefix(){return !CatalogAudit.Active;}
}
[HarmonyPatch(typeof(SteamManager),"Update")]
static class CatalogDrive
{
 static void Postfix(SteamManager __instance)
 {
  if(!CatalogAudit.Active || Time.unscaledTime<CatalogAudit.Next)return;
  try{
   if(CatalogAudit.Stage==0){
    File.WriteAllText(Path.Combine(CatalogAudit.Folder,"checks.txt"),"");
    CatalogAudit.Colors=AccessTools.TypeByName("Canna.FamilyColors.Plugin");CatalogAudit.Trajectories=AccessTools.TypeByName("Canna.FriendsTrajectories.Plugin");
    CatalogAudit.Check(CatalogAudit.Colors!=null && CatalogAudit.Trajectories!=null,"Both family extensions loaded");
    object[] decode={"24A8E1",Color.clear};bool valid=(bool)CatalogAudit.Call(CatalogAudit.Colors,"Decode",decode);
    CatalogAudit.Check(valid && (string)CatalogAudit.Call(CatalogAudit.Colors,"Encode",decode[1])=="24A8E1","Color wire encoding round trip");
    object[] malformed={"123456789",Color.clear};CatalogAudit.Check(!(bool)CatalogAudit.Call(CatalogAudit.Colors,"Decode",malformed),"Malformed color metadata rejected");
    NamedSpriteList list=(NamedSpriteList)AccessTools.Field(typeof(SteamManager),"abilityIconsFull").GetValue(__instance);
    NamedSprite bow=list.sprites.Find(delegate(NamedSprite e){return e.associatedGameObject!=null && e.associatedGameObject.GetComponent<BowTransform>()!=null;});
    NamedSprite fallback=list.sprites[1];
    List<Player> players=new List<Player>();
    for(int id=1;id<=2;id++){Player p=new Player(id,id-1);p.Scale=Fix.One;p.Color=bow.associatedGameObject.GetComponent<SpriteRenderer>().sharedMaterial;p.Abilities=new List<GameObject>{bow.associatedGameObject,fallback.associatedGameObject,fallback.associatedGameObject,fallback.associatedGameObject};p.AbilityIcons=new List<Sprite>{bow.sprite,fallback.sprite,fallback.sprite,fallback.sprite};p.CanUseAbilities=true;p.ignoreAllInputs=true;p.IsLocalPlayer=false;players.Add(p);}
    PlayerHandler.Get().SetPlayerList(players);AccessTools.Field(typeof(Host),"recordReplay").SetValue(null,false);
    Updater.PreLevelLoad();SceneManager.LoadScene(17);Updater.PostLevelLoad();CatalogAudit.Stage=1;CatalogAudit.Next=Time.unscaledTime+1;return;
   }
   if(CatalogAudit.Stage==1){
    GameTime.PlayerTimeScale=Fix.One;GameSessionHandler.GameIsPaused=false;
    GameSessionHandler session=UnityEngine.Object.FindObjectOfType<GameSessionHandler>();Updater.TickSimulation((Fix)1L/(Fix)60L);
    RoutineQueue routine=(RoutineQueue)AccessTools.Field(typeof(GameSessionHandler),"levelAnimationRoutine").GetValue(session);
    for(int i=0;i<600;i++){routine.Update((Fix)1L/(Fix)60L);Updater.TickSimulation((Fix)1L/(Fix)60L);}
    AccessTools.Method(typeof(GameSessionHandler),"SpawnPlayers").Invoke(session,null);
    Updater.TickSimulation((Fix)1L/(Fix)60L);
    SlimeController[] slimes=(SlimeController[])AccessTools.Field(typeof(GameSessionHandler),"slimeControllers").GetValue(session);
    foreach(SlimeController slime in slimes)if(slime!=null)slime.Spawn();
    CatalogAudit.Check(PlayerHandler.Get().GetPlayer(1).CurrentAbilities.Count==4,"Fourth ability survives native two-player spawning");
    Player first=PlayerHandler.Get().GetPlayer(1),second=PlayerHandler.Get().GetPlayer(2);
    Material source=first.Color;Color original=source.GetColor("_ShadowColor");
    CatalogAudit.Call(CatalogAudit.Colors,"Paint",first,new Color(.2f,.7f,.9f,1));
    CatalogAudit.Call(CatalogAudit.Colors,"Paint",second,new Color(.9f,.3f,.2f,1));
    CatalogAudit.Check(first.Color!=second.Color && first.Color!=source,"Custom colors isolate native materials per player");
    CatalogAudit.Check(source.GetColor("_ShadowColor")==original,"Original shared material remains unchanged");
    CatalogAudit.Check(first.Color.GetColor("_ShadowColor").b>.8f && second.Color.GetColor("_ShadowColor").r>.8f,"Independent player colors applied");
    SlimeController actor=slimes[0];actor.GetComponent<FixTransform>().position=new Vec2(Fix.Zero,(Fix)12L);AccessTools.Field(typeof(PlayerPhysics),"isGrounded").SetValue(actor.GetComponent<PlayerPhysics>(),false);actor.GetComponent<PlayerBody>().selfImposedVelocity=Vec2.zero;actor.GetComponent<PlayerBody>().externalVelocity=Vec2.zero;
    AccessTools.Method(typeof(SlimeController),"EnterAbility").Invoke(actor,new object[]{0,false});
    BowTransform bow=first.CurrentAbilities[0].GetComponent<BowTransform>();
    AccessTools.Field(typeof(BowTransform),"inputVector").SetValue(bow,Vec2.up);AccessTools.Field(typeof(BowTransform),"loadingFrame").SetValue(bow,2);
    object[] prediction={bow,Vec2.zero,new PhysicsBody()};CatalogAudit.Call(CatalogAudit.Trajectories,"Launch",prediction);
    PhysicsBody predicted=(PhysicsBody)prediction[2];
    // Validate the base game's launch equation independently of ArrowWall's intentional fan.
    new Harmony("family.canna.catalogaudit").Unpatch(AccessTools.Method(typeof(BowTransform),"Shoot"),HarmonyPatchType.All,"com.WackyModer.arrowWall");
    AccessTools.Method(typeof(BowTransform),"Shoot").Invoke(bow,new object[]{Vec2.up});
    BoplBody launched=null;foreach(Arrow arrow in UnityEngine.Object.FindObjectsOfType<Arrow>())if(arrow.GetComponent<Projectile>()!=null && arrow.GetComponent<Projectile>().GetPlayerId()==1)launched=arrow.GetComponent<BoplBody>();
    CatalogAudit.Check(launched!=null,"Native bow creates arrow");
    CatalogAudit.Check(launched.StartVelocity.x==predicted.velocity.x && launched.StartVelocity.y==predicted.velocity.y,"Prediction launch velocity exactly matches native charged arrow");
    AccessTools.Field(typeof(BowTransform),"hasFired").SetValue(bow,false);
    GameLobby.isOnlineGame=true;first.IsLocalPlayer=false;second.IsLocalPlayer=true;
    CatalogAudit.SetConfig(CatalogAudit.Trajectories,"Own",true);CatalogAudit.SetConfig(CatalogAudit.Trajectories,"Team",false);CatalogAudit.SetConfig(CatalogAudit.Trajectories,"Other",false);
    CatalogAudit.Check(!(bool)CatalogAudit.Call(CatalogAudit.Trajectories,"Visible",false,false),"Opponent paths hidden by default");
    CatalogAudit.SetConfig(CatalogAudit.Trajectories,"Other",true);AccessTools.Field(CatalogAudit.Trajectories,"Next").SetValue(null,0f);
    CatalogAudit.Call(CatalogAudit.Trajectories,"Draw");
    bool visible=false;foreach(LineRenderer line in UnityEngine.Object.FindObjectsOfType<LineRenderer>())if(line.name.StartsWith("Canna trajectory") && line.enabled && line.positionCount>1)visible=true;
    CatalogAudit.Check(visible,"Remote opponent trajectory renders with online flag and synchronized bow state");
    CatalogAudit.SetConfig(CatalogAudit.Trajectories,"Other",false);AccessTools.Field(CatalogAudit.Trajectories,"Next").SetValue(null,0f);CatalogAudit.Call(CatalogAudit.Trajectories,"Draw");
    visible=false;foreach(LineRenderer line in UnityEngine.Object.FindObjectsOfType<LineRenderer>())if(line.name.StartsWith("Canna trajectory") && line.enabled)visible=true;
    CatalogAudit.Check(!visible,"Disabling opponent visibility hides existing lines immediately");GameLobby.isOnlineGame=false;
    CatalogAudit.Check(__instance.currentLobby.Id.Value==0 || __instance.currentLobby.MemberCount<=1,"Transport test has no other lobby participants");
    CatalogAudit.LobbyTask=Steamworks.SteamMatchmaking.CreateLobbyAsync(4);CatalogAudit.Stage=2;CatalogAudit.Next=Time.unscaledTime+.5f;return;
   }
   if(CatalogAudit.Stage==2){
    if(!CatalogAudit.LobbyTask.IsCompleted){CatalogAudit.Next=Time.unscaledTime+.5f;return;}
    Steamworks.Data.Lobby? lobby=CatalogAudit.LobbyTask.Result;CatalogAudit.Check(lobby.HasValue,"Steam creates isolated private test lobby");CatalogAudit.TestLobby=lobby.Value;CatalogAudit.TestLobby.SetPrivate();CatalogAudit.TestLobby.SetMemberData("canna_color_v1","24A8E1");CatalogAudit.Stage=3;CatalogAudit.Next=Time.unscaledTime+1;return;
   }
   if(CatalogAudit.Stage==3){
    bool received=false;foreach(Steamworks.Friend member in CatalogAudit.TestLobby.Members)if(member.Id==Steamworks.SteamClient.SteamId)received=CatalogAudit.TestLobby.GetMemberData(member,"canna_color_v1")=="24A8E1";
    CatalogAudit.Check(received,"Real Steam lobby round trip preserves color metadata");CatalogAudit.TestLobby.Leave();
    File.WriteAllText(Path.Combine(CatalogAudit.Folder,"complete.txt"),"Catalog compatibility and family visual-extension checks completed. Second-PC multiplayer remains unverified.");Application.Quit();CatalogAudit.Next=float.MaxValue;
   }
  }catch(Exception e){if(CatalogAudit.TestLobby.Id.Value!=0)CatalogAudit.TestLobby.Leave();File.WriteAllText(Path.Combine(CatalogAudit.Folder,"failure.txt"),e.ToString());Application.Quit();CatalogAudit.Next=float.MaxValue;}
 }
}

