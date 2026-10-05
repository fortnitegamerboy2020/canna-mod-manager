using System;
using System.IO;
using System.Collections.Generic;
using System.Reflection;
using BepInEx;
using BoplFixedMath;
using HarmonyLib;
using UnityEngine;
using UnityEngine.SceneManagement;
namespace Canna.Anvil
{
    // Development harness. Only compiled when build.ps1 -Audit is explicitly used.
    [BepInPlugin("family.canna.anvilaudit", "Canna Anvil Audit", "1.0.0")]
    public class Audit : BaseUnityPlugin
    {
        internal static bool Active;
        internal static int Stage;
        internal static float Next=8;
        internal static string Folder;
        internal static SlimeController Slime;
        internal static Ability Ability;
        void Awake()
        {
            Active=Array.IndexOf(Environment.GetCommandLineArgs(),"--canna-anvil-audit")>=0;
            if(!Active)return;
            Folder=Path.Combine(Paths.ConfigPath,"CannaAnvilAudit");Directory.CreateDirectory(Folder);
            // Hooks are installed once by the main plugin in this development assembly.
        }
        internal static void Check(bool condition,string message)
        {if(!condition)throw new Exception(message);File.AppendAllText(Path.Combine(Folder,"checks.txt"),"PASS "+message+"\n");}
    }
    [HarmonyPatch]
    static class AuditSuppress
    {
        static IEnumerable<MethodBase> TargetMethods()
        {
            yield return AccessTools.Method(typeof(GameSessionHandler),"StartSpawnPlayersRoutine");
            yield return AccessTools.Method(typeof(GameSessionHandler),"Update");
            yield return AccessTools.Method(typeof(GameSessionHandler),"UpdateSim");
            yield return AccessTools.Method(typeof(AchievementHandler),"OnStartedAGame");
        }
        static bool Prefix(){return !Audit.Active;}
    }
    [HarmonyPatch(typeof(SteamManager),"Update")]
    static class AuditDrive
    {
        static void Postfix(SteamManager __instance)
        {
            if(!Audit.Active || Time.unscaledTime<Audit.Next)return;
            try
            {
                if(Audit.Stage==0)
                {
                    File.WriteAllText(Path.Combine(Audit.Folder,"checks.txt"),"");
                    NamedSpriteList list=(NamedSpriteList)AccessTools.Field(typeof(SteamManager),"abilityIconsFull").GetValue(__instance);
                    Audit.Check(list.IndexOf("Anvil")==31,"Anvil appended at index 31; stock indices preserved");
                    string signature=Plugin.LobbyProtocol(__instance);
                    NamedSprite first=list.sprites[1],second=list.sprites[2];list.sprites[1]=second;list.sprites[2]=first;
                    Audit.Check(Plugin.LobbyProtocol(__instance)!=signature,"Online fingerprint detects mismatched ability order");
                    list.sprites[1]=first;list.sprites[2]=second;
                    Audit.Check(Plugin.LobbyProtocol(__instance)==signature,"Online fingerprint stable for identical ability order");
                    int count=list.sprites.Count;Plugin.Register(list);
                    Audit.Check(list.sprites.Count==count,"Registration is idempotent");
                    Audit.Check(Plugin.Prefab!=null && Plugin.Prefab.GetComponent<AnvilState>()!=null,"Separate marked prefab registered");
                    foreach(AbilityGrid grid in Resources.FindObjectsOfTypeAll<AbilityGrid>())
                        if(grid.gameObject.scene.IsValid())
                        {
                            AbilityGridEntry[] entries=(AbilityGridEntry[])AccessTools.Field(typeof(AbilityGrid),"grid").GetValue(grid);
                            if(entries!=null)Audit.Check(entries.Length==list.sprites.Count-1,"Native ability grid includes appended entry");
                        }
                    for(int i=0;i<17;i++)File.WriteAllBytes(Path.Combine(Audit.Folder,"morph-"+i+".png"),ImageConversion.EncodeToPNG(Art.Frames[i].texture));
                    File.WriteAllBytes(Path.Combine(Audit.Folder,"icon.png"),ImageConversion.EncodeToPNG(Art.Icon.texture));
                    Audit.Check(list.sprites[list.IndexOf("Anvil")].sprite==Art.Icon,"Picker and HUD use transparent artwork independent of gameplay sprite");
                    NamedSprite stock=list.sprites.Find(delegate(NamedSprite e){return e.name!="Anvil" && e.associatedGameObject!=null && e.associatedGameObject.GetComponent<BounceBall>()!=null;});
                    Audit.Check(Math.Abs(Art.Icon.bounds.size.x-stock.sprite.bounds.size.x/1.35f)<.01f,"HUD icon scaled to match measured standard ability size");
                    Audit.Check(Art.Icon.texture.GetPixel(64,18).a==0,"Icon has no baked background or border");
                    Fix nativeMass=(Fix)AccessTools.Field(typeof(BoplBody),"mass").GetValue(stock.associatedGameObject.GetComponent<BoplBody>());
                    Fix anvilMass=(Fix)AccessTools.Field(typeof(BoplBody),"mass").GetValue(Plugin.Prefab.GetComponent<BoplBody>());
                    Audit.Check(anvilMass==nativeMass*(Fix)2L,"Anvil mass halved to twice native Rock mass");
                    Audit.Check((float)AnvilState.MorphDuration<.067f,"Entry morph completes in about 67 milliseconds");
                    int bottom=128;for(int y=0;y<128;y++)for(int x=0;x<128;x++)if(Art.Frames[16].texture.GetPixel(x,y).a>.1f)bottom=Math.Min(bottom,y);
                    float visibleBottom=(60-bottom)/24f;
                    float hullBottom=(float)Plugin.Prefab.GetComponent<DPhysicsCircle>().GetStartRadius();
                    Audit.Check(Math.Abs(visibleBottom-hullBottom)<.03f,"Collider bottom matches opaque Anvil feet within 0.03 native units");
                    Audit.Check(Art.Frames[16].pixelsPerUnit==24,"Gameplay sprite twice the previous size");
                    Player player=new Player(1,0);
                    player.Scale=Fix.One;player.Color=Plugin.Prefab.GetComponent<SpriteRenderer>().sharedMaterial;
                    player.Abilities=new List<GameObject>{Plugin.Prefab,Plugin.Prefab,Plugin.Prefab};
                    player.AbilityIcons=new List<Sprite>{Art.Icon,Art.Icon,Art.Icon};
                    player.CanUseAbilities=true;player.ignoreAllInputs=true;player.IsLocalPlayer=false;
                    Player victim=new Player(2,1);victim.Scale=Fix.One;victim.Color=player.Color;
                    victim.Abilities=new List<GameObject>(player.Abilities);victim.AbilityIcons=new List<Sprite>(player.AbilityIcons);victim.ignoreAllInputs=true;victim.IsLocalPlayer=false;
                    PlayerHandler.Get().SetPlayerList(new List<Player>{player,victim});
                    AccessTools.Field(typeof(Host),"recordReplay").SetValue(null,false);
                    Updater.PreLevelLoad();SceneManager.LoadScene(17);Updater.PostLevelLoad();
                    Audit.Stage=1;Audit.Next=Time.unscaledTime+1;return;
                }
                if(Audit.Stage==1)
                {
                    GameTime.PlayerTimeScale=Fix.One;GameSessionHandler.GameIsPaused=false;
                    GameSessionHandler session=UnityEngine.Object.FindObjectOfType<GameSessionHandler>();
                    Updater.TickSimulation((Fix)1L/(Fix)60L);
                    RoutineQueue routine=(RoutineQueue)AccessTools.Field(typeof(GameSessionHandler),"levelAnimationRoutine").GetValue(session);
                    for(int tick=0;tick<600;tick++){routine.Update((Fix)1L/(Fix)60L);Updater.TickSimulation((Fix)1L/(Fix)60L);}
                    AccessTools.Method(typeof(GameSessionHandler),"SpawnPlayers").Invoke(session,null);
                    for(int tick=0;tick<2;tick++)Updater.TickSimulation((Fix)1L/(Fix)60L);
                    SlimeController[] slimes=(SlimeController[])AccessTools.Field(typeof(GameSessionHandler),"slimeControllers").GetValue(session);
                    Audit.Slime=slimes[0];
                    AbilityReadyIndicator[] indicators=(AbilityReadyIndicator[])AccessTools.Field(typeof(SlimeController),"AbilityReadyIndicators").GetValue(Audit.Slime);
                    AnvilHudBadge badge=indicators[0].GetComponent<AnvilHudBadge>();
                    Audit.Check(badge!=null && badge.Fill.enabled && badge.Border.enabled,"Native HUD creates a visible Anvil circle and border");
                    Color teamFill=badge.Artwork.material.GetColor("_CircleColor");
                    Audit.Check(badge.Fill.color==teamFill,"HUD background matches native team fill");
                    bool matches=false;
                    foreach(TeamColors palette in Resources.FindObjectsOfTypeAll<TeamColors>())foreach(TeamColor color in palette.teamColors)
                        if(color.fill==teamFill && color.border==badge.Border.color)matches=true;
                    Audit.Check(matches,"HUD border uses the actual native team border color");
                    indicators[0].SetSprite(Art.Frames[16],false);
                    Audit.Check(!badge.Fill.enabled && !badge.Border.enabled,"HUD badge hides when ability icon changes");
                    indicators[0].SetSprite(Art.Icon,false);
                    Audit.Check(badge.Fill.enabled && badge.Border.enabled,"HUD badge returns with Anvil icon");
                    Audit.Slime.Spawn();slimes[1].Spawn();
                    PlayerHandler.Get().GetPlayer(1).CanUseAbilities=true;
                    Audit.Slime.GetComponent<FixTransform>().position=new Vec2(Fix.Zero,(Fix)12L);
                    AccessTools.Field(typeof(PlayerPhysics),"isGrounded").SetValue(Audit.Slime.GetComponent<PlayerPhysics>(),false);
                    Audit.Slime.GetComponent<PlayerBody>().selfImposedVelocity=new Vec2((Fix)4L,Fix.Zero);
                    Audit.Slime.GetComponent<PlayerBody>().externalVelocity=new Vec2((Fix)3L,(Fix)2L);
                    AccessTools.Method(typeof(SlimeController),"EnterAbility").Invoke(Audit.Slime,new object[]{0,false});
                    Audit.Ability=PlayerHandler.Get().GetPlayer(1).CurrentAbilities[0].GetComponent<Ability>();
                    Audit.Check(Audit.Ability.gameObject.activeInHierarchy,"Native slime EnterAbility activates Anvil");
                    Audit.Check(Audit.Ability.GetPlayerId()==1,"Native player ownership retained");
                    BoplBody body=Audit.Ability.GetComponent<BoplBody>();
                    Audit.Check(body.velocity.x==(Fix)7L && body.velocity.y==(Fix)2L,"Native entry inherits momentum without a downward impulse");
                    Audit.Check(body.angularVelocity!=Fix.Zero,"Native entry retains momentum-derived spin");
                    Audit.Check(body.PhysicsBody().gravityScale==Fix.One,"Standard gravity reaches native physics body");

                    Audit.Check(Audit.Ability.GetCooldown()==(Fix)6L,"Six second native cooldown");
                    Audit.Check(Audit.Ability.GetComponent<SpriteRenderer>().sprite==Art.Frames[0],"Entry begins as slime, not final anvil");
                    Audit.Check((Fix)AccessTools.Field(typeof(BounceBall),"maxDuration").GetValue(Audit.Ability.GetComponent<BounceBall>())==(Fix)5L,"Five second native Anvil duration");
                    for(int tick=0;tick<3;tick++){body.position=new Vec2(Fix.Zero,(Fix)12L);body.velocity=Vec2.zero;Updater.TickSimulation((Fix)1L/(Fix)60L);}
                    Sprite intermediate=Audit.Ability.GetComponent<SpriteRenderer>().sprite;
                    Audit.Check(intermediate!=Art.Frames[0] && intermediate!=Art.Frames[16],"Native simulation displays intermediate morph frames");
                    body.angularVelocity=(Fix)2L;
                    Fix beforeRotation=body.rotation;
                    body.position=new Vec2(Fix.Zero,(Fix)12L);
                    Updater.TickSimulation((Fix)1L/(Fix)60L);
                    Audit.Check(body.rotation!=beforeRotation && body.angularVelocity!=Fix.Zero,"Native simulation rotates freely without frame-by-frame locks");
                    Audit.Check(!(bool)AccessTools.Field(typeof(BounceBall),"IsExiting").GetValue(Audit.Ability.GetComponent<BounceBall>()),"Released input cannot cancel the entry morph immediately");
                    for(int tick=0;tick<4;tick++){body.position=new Vec2(Fix.Zero,(Fix)12L);body.velocity=Vec2.zero;Updater.TickSimulation((Fix)1L/(Fix)60L);}
                    Audit.Check(Audit.Ability.GetComponent<SpriteRenderer>().sprite==Art.Frames[16],"Entry morph reaches steel anvil sprite");
                    foreach(PlayerAverageCamera c in UnityEngine.Object.FindObjectsOfType<PlayerAverageCamera>())c.enabled=false;
                    Camera.main.transform.position=new Vector3(0,0,-10);Camera.main.orthographicSize=18;
                    // Hold the preview in view without changing shipped gameplay.
                    body.position=new Vec2(Fix.Zero,(Fix)5L);body.velocity=Vec2.zero;
                    Audit.Stage=2;Audit.Next=Time.unscaledTime+.1f;return;
                }
                if(Audit.Stage==2)
                {
                    ScreenCapture.CaptureScreenshot(Path.Combine(Audit.Folder,"anvil-game.png"));
                    Player victim=PlayerHandler.Get().GetPlayer(2);
                    SlimeController victimSlime=UnityEngine.Object.FindObjectsOfType<SlimeController>()[0];
                    foreach(SlimeController s in UnityEngine.Object.FindObjectsOfType<SlimeController>())if(s.GetPlayerId()==2)victimSlime=s;
                    PlayerCollision collisionHandler=victimSlime.GetPlayerCollision();
                    AccessTools.Field(typeof(PlayerCollision),"isInvulnerableMask").SetValue(collisionHandler,0u);
                    DPhysicsCircle anvilHull=Audit.Ability.GetComponent<DPhysicsCircle>();
                    CollisionInformation contact=new CollisionInformation();contact.colliderPP=anvilHull.GetPhysicsParent();contact.layer=LayerMask.NameToLayer("Player");contact.normal=Vec2.up;
                    Audit.Check(victim.IsAlive,"Opponent alive before contact fixture");
                    collisionHandler.OnCollide(contact);
                    Audit.Check(!victim.IsAlive,"Native contact combat kills opponent");
                    Audit.Stage=3;Audit.Next=Time.unscaledTime+.1f;return;
                }
                if(Audit.Stage==3)
                {
                    BounceBall ball=Audit.Ability.GetComponent<BounceBall>();
                    AccessTools.Field(typeof(BounceBall),"IsExiting").SetValue(ball,true);
                    AccessTools.Field(typeof(BounceBall),"timeSinceExitStarted").SetValue(ball,(Fix)AccessTools.Field(typeof(BounceBall),"exitTime").GetValue(ball)/(Fix)2L);
                    Audit.Ability.GetComponent<AnvilState>().Paint();
                    Audit.Check(Audit.Ability.GetComponent<SpriteRenderer>().sprite==Art.Frames[8],"Exit morph reverses to halfway frame");
                    AccessTools.Field(typeof(BounceBall),"timeSinceExitStarted").SetValue(ball,(Fix)1L);
                    ball.LateUpdateSim((Fix)1L/(Fix)60L);
                    Audit.Check(!Audit.Ability.gameObject.activeSelf,"Native exit deactivates Anvil");
                    Audit.Check(!((bool)AccessTools.Field(typeof(SlimeController),"isInAbility").GetValue(Audit.Slime)),"Native exit returns player to slime");
                    File.WriteAllText(Path.Combine(Audit.Folder,"complete.txt"),"Anvil registration, native entry, physics, animation and exit verified.");
                    Audit.Stage=4;Audit.Next=Time.unscaledTime+.3f;return;
                }
                Application.Quit();Audit.Next=float.MaxValue;
            }
            catch(Exception error)
            {File.WriteAllText(Path.Combine(Audit.Folder,"failure.txt"),error.ToString());Application.Quit();Audit.Next=float.MaxValue;}
        }
    }
}







