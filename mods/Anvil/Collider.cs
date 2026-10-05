using System;
using System.Collections.Generic;
using System.Reflection;
using BoplFixedMath;
using HarmonyLib;
using UnityEngine;

namespace Canna.Anvil
{
    // BounceBall's ability lifecycle expects a DPhysicsCircle component. Keep that
    // API as a facade, but register only a real native Box in deterministic physics.
    // Other Rock/BounceBall objects retain their original circle implementation.
    static class AnvilHull
    {
        internal static DPhysicsBox Box(DPhysicsCircle circle)
        { return circle.GetComponent<AnvilState>() == null ? null : circle.GetComponent<DPhysicsBox>(); }
        internal static object Forward(DPhysicsBox box, MethodBase method, object[] args)
        {
            MethodInfo target = AccessTools.Method(typeof(DPhysicsBox), method.Name);
            return target.Invoke(box,args);
        }
        internal static IEnumerable<MethodBase> Methods(Type result, bool setters)
        {
            foreach(MethodInfo method in AccessTools.GetDeclaredMethods(typeof(DPhysicsCircle)))
            {
                if(method.ReturnType!=result)continue;
                MethodInfo target=typeof(DPhysicsBox).GetMethod(method.Name,BindingFlags.Public|BindingFlags.NonPublic|BindingFlags.Instance);
                if(target==null || target.ReturnType!=result)continue;
                if(setters ? method.Name.StartsWith("set_") || method.Name.StartsWith("AddForce") || method.Name.StartsWith("SetRelative")
                           : method.Name.StartsWith("get_") || method.Name=="GetPhysicsParent" || method.Name=="GetBody" || method.Name=="IsComposite")
                    yield return method;
            }
        }
    }
    [HarmonyPatch(typeof(BoplBody),"Awake")]
    static class BindAnvilBox
    {
        static void Postfix(BoplBody __instance)
        {
            if(__instance.GetComponent<AnvilState>()!=null)
                AccessTools.Field(typeof(BoplBody),"physicsCollider").SetValue(__instance,__instance.GetComponent<DPhysicsBox>());
        }
    }
    [HarmonyPatch(typeof(DPhysicsCircle),"Initialize")]
    static class InitializeAnvilBox
    {
        static bool Prefix(DPhysicsCircle __instance)
        {
            DPhysicsBox box=AnvilHull.Box(__instance);if(box==null)return true;
            if(!box.initHasBeenCalled)
            {
                box.ManualInit();
                box.RegisterCollisionCallback(__instance);
            }
            return false;
        }
    }
    [HarmonyPatch]
    static class AnvilHullFix
    {
        static IEnumerable<MethodBase> TargetMethods(){return AnvilHull.Methods(typeof(Fix),false);}
        static bool Prefix(DPhysicsCircle __instance,MethodBase __originalMethod,ref Fix __result)
        {DPhysicsBox box=AnvilHull.Box(__instance);if(box==null)return true;__result=(Fix)AnvilHull.Forward(box,__originalMethod,new object[0]);return false;}
    }
    [HarmonyPatch]
    static class AnvilHullVec
    {
        static IEnumerable<MethodBase> TargetMethods(){return AnvilHull.Methods(typeof(Vec2),false);}
        static bool Prefix(DPhysicsCircle __instance,MethodBase __originalMethod,ref Vec2 __result)
        {DPhysicsBox box=AnvilHull.Box(__instance);if(box==null)return true;__result=(Vec2)AnvilHull.Forward(box,__originalMethod,new object[0]);return false;}
    }
    [HarmonyPatch]
    static class AnvilHullParent
    {
        static IEnumerable<MethodBase> TargetMethods(){return AnvilHull.Methods(typeof(PhysicsParent),false);}
        static bool Prefix(DPhysicsCircle __instance,MethodBase __originalMethod,ref PhysicsParent __result)
        {DPhysicsBox box=AnvilHull.Box(__instance);if(box==null)return true;__result=(PhysicsParent)AnvilHull.Forward(box,__originalMethod,new object[0]);return false;}
    }
    [HarmonyPatch]
    static class AnvilHullBody
    {
        static IEnumerable<MethodBase> TargetMethods(){return AnvilHull.Methods(typeof(PhysicsBody),false);}
        static bool Prefix(DPhysicsCircle __instance,MethodBase __originalMethod,ref PhysicsBody __result)
        {DPhysicsBox box=AnvilHull.Box(__instance);if(box==null)return true;__result=(PhysicsBody)AnvilHull.Forward(box,__originalMethod,new object[0]);return false;}
    }
    [HarmonyPatch]
    static class AnvilHullShape
    {
        static IEnumerable<MethodBase> TargetMethods(){return AnvilHull.Methods(typeof(Shape),false);}
        static bool Prefix(DPhysicsCircle __instance,MethodBase __originalMethod,ref Shape __result)
        {DPhysicsBox box=AnvilHull.Box(__instance);if(box==null)return true;__result=(Shape)AnvilHull.Forward(box,__originalMethod,new object[0]);return false;}
    }
    [HarmonyPatch]
    static class AnvilHullBool
    {
        static IEnumerable<MethodBase> TargetMethods(){return AnvilHull.Methods(typeof(bool),false);}
        static bool Prefix(DPhysicsCircle __instance,MethodBase __originalMethod,ref bool __result)
        {DPhysicsBox box=AnvilHull.Box(__instance);if(box==null)return true;__result=(bool)AnvilHull.Forward(box,__originalMethod,new object[0]);return false;}
    }
    [HarmonyPatch]
    static class AnvilHullSetters
    {
        static IEnumerable<MethodBase> TargetMethods(){return AnvilHull.Methods(typeof(void),true);}
        static bool Prefix(DPhysicsCircle __instance,MethodBase __originalMethod,object[] __args)
        {DPhysicsBox box=AnvilHull.Box(__instance);if(box==null)return true;AnvilHull.Forward(box,__originalMethod,__args);return false;}
    }
    [HarmonyPatch(typeof(DPhysicsCircle),"get_radius")]
    static class AnvilHullRadius
    {
        static bool Prefix(DPhysicsCircle __instance,ref Fix __result)
        {DPhysicsBox box=AnvilHull.Box(__instance);if(box==null)return true;__result=box.CalcExtents().y;return false;}
    }
    [HarmonyPatch(typeof(DPhysicsCircle),"Circle")]
    static class AnvilHullCircleQuery
    {
        static bool Prefix(DPhysicsCircle __instance,ref Circle __result)
        {
            DPhysicsBox box=AnvilHull.Box(__instance);if(box==null)return true;
            Vec2 extents=box.CalcExtents();
            __result=new Circle{center=box.position,radius=Fix.Max(extents.x,extents.y),layer=box.gameObject.layer};
            return false;
        }
    }
    [HarmonyPatch(typeof(DPhysicsCircle),"IPhysicsCollider.set_enabled")]
    static class AnvilHullEnabled
    {
        static void Postfix(DPhysicsCircle __instance,bool __0)
        {DPhysicsBox box=AnvilHull.Box(__instance);if(box!=null)box.enabled=__0;}
    }
    [HarmonyPatch(typeof(DPhysicsCircle),"set_radius")]
    static class AnvilHullResizeRadius
    {
        static bool Prefix(DPhysicsCircle __instance,Fix __0)
        {DPhysicsBox box=AnvilHull.Box(__instance);if(box==null)return true;box.Scale*=__0/box.CalcExtents().y;return false;}
    }
    [HarmonyPatch(typeof(DPhysicsCircle),"UpdatePhysicsPositions")]
    static class AnvilHullPositions
    {
        static bool Prefix(DPhysicsCircle __instance)
        {DPhysicsBox box=AnvilHull.Box(__instance);if(box==null)return true;box.UpdatePhysicsPositions();return false;}
    }
    [HarmonyPatch(typeof(DPhysicsCircle),"UpdateLayer")]
    static class AnvilHullLayer
    {
        static bool Prefix(DPhysicsCircle __instance)
        {DPhysicsBox box=AnvilHull.Box(__instance);if(box==null)return true;Box shape=box.physicsBox;shape.layer=box.gameObject.layer;box.physicsBox=shape;return false;}
    }
}
