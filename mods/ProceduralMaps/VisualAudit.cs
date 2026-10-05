using System;
using System.IO;
using System.Reflection;
using System.Collections.Generic;
using BepInEx;
using BoplFixedMath;
using HarmonyLib;
using UnityEngine;
using UnityEngine.SceneManagement;
[BepInPlugin("family.canna.visualaudit","Canna Visual Audit","1.0.0")]
public class VisualAudit : BaseUnityPlugin {
 internal static bool Active;
 internal static int Stage;
 internal static float Next=5;
 internal static string Folder;
 void Awake() {
  Active=Array.IndexOf(Environment.GetCommandLineArgs(),"--canna-visual-audit")>=0;
  if(!Active)return;
  Folder=Path.Combine(Paths.ConfigPath,"CannaMaps");
  // Compiled with Plugin.cs: the production plugin installs this assembly's hooks once.
 }
}
[HarmonyPatch]
static class VisualSuppress {
 static IEnumerable<MethodBase> TargetMethods() {
  yield return AccessTools.Method(typeof(GameSessionHandler),"StartSpawnPlayersRoutine");
  yield return AccessTools.Method(typeof(GameSessionHandler),"Update");
  yield return AccessTools.Method(typeof(GameSessionHandler),"UpdateSim");
  yield return AccessTools.Method(typeof(AchievementHandler),"OnStartedAGame");
 }
 static bool Prefix(){return !VisualAudit.Active;}
}
[HarmonyPatch(typeof(SteamManager),"Update")]
static class VisualDrive {
 static void Postfix() {
  if(!VisualAudit.Active || Time.unscaledTime<VisualAudit.Next)return;
  try {
   if(VisualAudit.Stage==0) {
    int scene=6; foreach(string arg in Environment.GetCommandLineArgs())if(arg.StartsWith("--canna-visual-scene="))scene=int.Parse(arg.Substring("--canna-visual-scene=".Length));
    Updater.PreLevelLoad(); SceneManager.LoadScene(scene); Updater.PostLevelLoad();
    VisualAudit.Stage=1;VisualAudit.Next=Time.unscaledTime+1;return;
   }
   if(VisualAudit.Stage==1) {
    AccessTools.Field(typeof(Host),"recordReplay").SetValue(null,false);
    GameTime.PlayerTimeScale=Fix.One;GameSessionHandler.GameIsPaused=false;
    GameSessionHandler session=UnityEngine.Object.FindObjectOfType<GameSessionHandler>();
    Updater.TickSimulation((Fix)1L/(Fix)60L);
    RoutineQueue routine=(RoutineQueue)AccessTools.Field(typeof(GameSessionHandler),"levelAnimationRoutine").GetValue(session);
    for(int tick=0;tick<600;tick++) {routine.Update((Fix)1L/(Fix)60L);Updater.TickSimulation((Fix)1L/(Fix)60L);}
    string report="";
    foreach(PlatformTransform t in Resources.FindObjectsOfTypeAll<PlatformTransform>()) {
     StickyRoundedRectangle prefab=(StickyRoundedRectangle)AccessTools.Field(typeof(PlatformTransform),"platformPrefab").GetValue(t);
     if(prefab!=null)report+="PREFAB "+prefab.name+" shader="+prefab.GetComponent<SpriteRenderer>().sharedMaterial.shader.name+" sprite="+prefab.GetComponent<SpriteRenderer>().sprite.name+"\n";
    }
    foreach(KeyValuePair<AnimateVelocity,Canna.ProceduralMaps.Island> pair in Canna.ProceduralMaps.Plugin.Moving) {
     SpriteRenderer s=pair.Key.GetComponent<SpriteRenderer>();Material m=s.material;
     DPhysicsRoundedRect rr=pair.Key.GetComponent<DPhysicsRoundedRect>();
     Canna.ProceduralMaps.Island p=pair.Value;
     if(p.width+p.radius<390)throw new Exception("Generated platform remains too small");
     if(Math.Abs((float)(rr.CalcExtents().x+rr.radius)-(p.width+p.radius)/100f)>.02f)throw new Exception("Generated width differs from native collision geometry");
     foreach(Canna.ProceduralMaps.Island q in Canna.ProceduralMaps.Plugin.Map.islands)
      if(!object.ReferenceEquals(p,q) && Math.Abs(p.x-q.x)<=p.width+q.width+p.radius+q.radius+p.drift+q.drift+200 && Math.Abs(p.y-q.y)<=p.height+q.height+p.radius+q.radius+200)throw new Exception("Native geometry or motion envelopes overlap");
     report+="PHYS scale="+rr.Scale+" base="+pair.Key.GetComponent<StickyRoundedRectangle>().baseScaleForPlatform+" start="+AccessTools.Field(typeof(DPhysicsRoundedRect),"startExtents").GetValue(rr)+" actual="+rr.CalcExtents()+" radius="+rr.radius+"\n";
     foreach(Component component in pair.Key.GetComponents<Component>())report+=component.GetType().Name+",";report+="\n";
     pair.Key.GetComponent<FixTransform>().SyncTransform();
     report+=pair.Key.name+" pos="+pair.Key.GetComponent<FixTransform>().position+" bounds="+s.bounds+" scale="+s.transform.localScale+" color="+s.color+" shader="+m.shader.name+" sprite="+s.sprite.rect+" tex="+s.sprite.texture.width+","+s.sprite.texture.height+" ppu="+s.sprite.pixelsPerUnit+"\n";
     for(int propIndex=0;propIndex<m.shader.GetPropertyCount();propIndex++) {
      string prop=m.shader.GetPropertyName(propIndex);UnityEngine.Rendering.ShaderPropertyType type=m.shader.GetPropertyType(propIndex);
      report+=prop+" "+type+"="+(type==UnityEngine.Rendering.ShaderPropertyType.Texture ? (m.GetTexture(prop)==null?"null":m.GetTexture(prop).name) : (type==UnityEngine.Rendering.ShaderPropertyType.Color || type==UnityEngine.Rendering.ShaderPropertyType.Vector ? m.GetVector(prop).ToString():m.GetFloat(prop).ToString()))+"\n";
     }
     foreach(string prop in new string[]{"_Scale","_RWidth","_RHeight","_BevelRadius","_Alpha"})if(m.HasProperty(prop))report+=prop+"="+m.GetFloat(prop)+" ";report+="\n";
    }
    File.WriteAllText(Path.Combine(VisualAudit.Folder,"visual-audit.txt"),report);
    foreach(PlayerAverageCamera c in UnityEngine.Object.FindObjectsOfType<PlayerAverageCamera>())c.enabled=false;
    Camera camera=Camera.main;camera.transform.position=new Vector3(0,0,-10);camera.orthographicSize=18;
    VisualAudit.Stage=2;VisualAudit.Next=Time.unscaledTime+1;return;
   }
   if(VisualAudit.Stage==2){
    Camera camera=Camera.main;RenderTexture target=new RenderTexture(1600,900,24);
    RenderTexture old=RenderTexture.active;camera.targetTexture=target;camera.Render();RenderTexture.active=target;
    Texture2D pixels=new Texture2D(1600,900,TextureFormat.RGB24,false);pixels.ReadPixels(new Rect(0,0,1600,900),0,0);pixels.Apply();
    File.WriteAllBytes(Path.Combine(VisualAudit.Folder,"visual-audit.png"),ImageConversion.EncodeToPNG(pixels));
    camera.targetTexture=null;RenderTexture.active=old;UnityEngine.Object.Destroy(target);UnityEngine.Object.Destroy(pixels);
    VisualAudit.Stage=3;VisualAudit.Next=Time.unscaledTime+2;return;
   }
   Application.Quit();VisualAudit.Next=float.MaxValue;
  }catch(Exception error){File.WriteAllText(Path.Combine(VisualAudit.Folder,"visual-failure.txt"),error.ToString());Application.Quit();VisualAudit.Next=float.MaxValue;}
 }
}



