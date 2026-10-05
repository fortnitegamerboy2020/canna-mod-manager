using System;
using System.IO;
using System.Reflection;
using System.Collections;
using BepInEx;
using HarmonyLib;
using UnityEngine;
[BepInPlugin("family.canna.menuaudit","Canna Menu Audit","1.0.0")]
public class MenuAudit : BaseUnityPlugin {
 internal static bool Active;internal static float Next=7;internal static int Stage=-1;internal static string Folder;
 void Awake(){Active=Array.IndexOf(Environment.GetCommandLineArgs(),"--canna-menu-audit")>=0;Folder=Path.Combine(Paths.ConfigPath,"CannaAnvilAudit");}
}
[HarmonyPatch(typeof(SteamManager),"Update")]
static class MenuDrive {
 static void Postfix(){
  if(!MenuAudit.Active || Time.unscaledTime<MenuAudit.Next)return;
  try {
   if(MenuAudit.Stage==-1){
    for(int i=0;i<UnityEngine.SceneManagement.SceneManager.sceneCountInBuildSettings;i++){
     string path=UnityEngine.SceneManagement.SceneUtility.GetScenePathByBuildIndex(i);
     File.AppendAllText(Path.Combine(MenuAudit.Folder,"menu-scenes.txt"),i+" "+path+"\n");
     if(path.ToLowerInvariant().Contains("characterselect")){UnityEngine.SceneManagement.SceneManager.LoadScene(i);MenuAudit.Stage=0;MenuAudit.Next=Time.unscaledTime+2;return;}
    }
    throw new Exception("No character select scene found");
   }
   if(MenuAudit.Stage==0){
    int count=0;
    foreach(AbilityGrid grid in Resources.FindObjectsOfTypeAll<AbilityGrid>())if(grid.gameObject.scene.IsValid()){
     Transform t=grid.transform;while(t!=null){t.gameObject.SetActive(true);t=t.parent;}
     count++;
    }
    File.WriteAllText(Path.Combine(MenuAudit.Folder,"menu-checks.txt"),"Activated "+count+" native grids\n");
    if(count==0)throw new Exception("No scene ability grids found");
    MenuAudit.Stage=1;MenuAudit.Next=Time.unscaledTime+1;return;
   }
   Type tracker=AccessTools.TypeByName("AbilityScrollBar.ScrollTracker");
   IDictionary states=(IDictionary)AccessTools.Field(tracker,"States").GetValue(null);
   if(states.Count==0)throw new Exception("Scroll mod did not initialize for overflowing grid");
   foreach(AbilityGrid grid in Resources.FindObjectsOfTypeAll<AbilityGrid>())if(grid.gameObject.scene.IsValid() && states.Contains(grid.gameObject.GetInstanceID())){
    AbilityGridEntry[] entries=(AbilityGridEntry[])AccessTools.Field(typeof(AbilityGrid),"grid").GetValue(grid);
    AccessTools.Property(typeof(AbilityGrid),"SelectedIcon").SetValue(grid,entries.Length-1,null);
    object state=states[grid.gameObject.GetInstanceID()];
    MethodInfo update=AccessTools.Method(AccessTools.TypeByName("AbilityScrollBar.UpdatePatch"),"Postfix");
    for(int i=0;i<100;i++)update.Invoke(null,new object[]{grid});
    float offset=(float)AccessTools.Field(state.GetType(),"scrollOffset").GetValue(state);
    Rect rect=(Rect)AccessTools.Field(state.GetType(),"visibleRect").GetValue(state);
    float size=(float)AccessTools.Field(state.GetType(),"entrySize").GetValue(state);
    float y=entries[entries.Length-1].rectTrans.anchoredPosition.y;
    if(offset<=0 || y-size*.5f<rect.yMin-.5f || y+size*.5f>rect.yMax+.5f)throw new Exception("Last ability does not fit scrolled viewport");
    File.AppendAllText(Path.Combine(MenuAudit.Folder,"menu-checks.txt"),"PASS Last entry scrolls inside native viewport; entries="+entries.Length+", offset="+offset+"\n");
   }
   File.AppendAllText(Path.Combine(MenuAudit.Folder,"menu-checks.txt"),"PASS Water height="+Constants.WATER_HEIGHT+"\n");
   Application.Quit();MenuAudit.Next=float.MaxValue;
  }catch(Exception e){File.WriteAllText(Path.Combine(MenuAudit.Folder,"menu-failure.txt"),e.ToString());Application.Quit();MenuAudit.Next=float.MaxValue;}
 }
}
