using System;
using BepInEx;
using BepInEx.Configuration;
using BepInEx.Logging;
using BoplFixedMath;
using HarmonyLib;
using UnityEngine;

namespace Canna.DrillThroughBall
{
    [BepInPlugin("family.canna.drillthroughball", "Drill Through Ball", "1.0.5")]
    public sealed class Plugin : BaseUnityPlugin
    {
        internal static ConfigEntry<bool> Enabled;
        internal static ManualLogSource Log;
        private Harmony harmony;
        private void Awake()
        {
            Log = Logger;
            Enabled = Config.Bind("General", "Enabled", true, "An active drill tip can defeat players transformed into Bounce Ball. Use the same setting on every participant's PC.");
            harmony = new Harmony("family.canna.drillthroughball");
            harmony.PatchAll(typeof(Plugin).Assembly);
            foreach (System.Reflection.MethodBase method in harmony.GetPatchedMethods())
                Logger.LogInfo("Hook registered: " + method.DeclaringType.Name + "." + method.Name);
            Logger.LogInfo("Drill Through Ball 1.0.5 loaded. Rock collision, player collision, killPlayer and Drill.ExitAbility are hooked. Enabled=" + Enabled.Value + ". All players must use the same version and setting.");
        }
        private void OnDestroy()
        {
            // Bopl removes the BepInEx Unity object during scene loading. The managed hooks,
            // configuration and log source must outlive that object for gameplay to work.
            if (harmony != null)
            {
                int count = 0;
                foreach (System.Reflection.MethodBase method in harmony.GetPatchedMethods()) count++;
                Log.LogInfo("Plugin Unity object removed; keeping " + count + " gameplay hooks active until the game process exits.");
            }
        }
    }

    [HarmonyPatch(typeof(Drill), "UpdateSim")]
    internal static class DrillTipPatch
    {
        private static PhysicsParent[] collisions = new PhysicsParent[64];
        private static bool errorReported;
        internal static Box ReadBox(DPhysicsBox hitbox)
        {
            // physicsBox contains the current scaled extents; Box() contains prefab start extents.
            Box box = hitbox.physicsBox;
            FixTransform transform = hitbox.GetComponent<FixTransform>();
            box.SetRotation(transform.rotation);
            box.center = transform.position + Vec2.ComplexMul(transform.offset, transform.right);
            return box;
        }
        // Harmony supplies the drill's existing fields; no replacement physics or floating-point distances.
        private static void Postfix(Drill __instance, Fix SimDeltaTime, bool ___isDrilling, HitboxCombo ___hitboxCombo, Ability ___ability)
        {
            if (!Plugin.Enabled.Value || !___isDrilling || SimDeltaTime == Fix.Zero ||
                __instance == null || __instance.IsDestroyed || !__instance.gameObject.activeInHierarchy ||
                ___hitboxCombo == null || !___hitboxCombo.AreHitboxesActive() || ___ability == null) return;
            try
            {
                DetPhysics physics = DetPhysics.Get();
                if (physics == null) return;
                int attackerId = ___ability.GetPlayerId();
                foreach (DPhysicsBox hitbox in ___hitboxCombo.HitboxesList())
                {
                    if (hitbox == null || hitbox.IsDestroyed || !hitbox.gameObject.activeInHierarchy) continue;
                    // The ball is a physical object, so query all layers and then accept BounceBall only.
                    int count = physics.CollideBox(ReadBox(hitbox), ref collisions, (LayerMask)(-1));
                    for (int i = 0; i < count && i < collisions.Length; i++)
                    {
                        BounceBall ball = DrillRules.FindBall(collisions[i]);
                        if (ball == null || ball.IsDestroyed || !ball.gameObject.activeInHierarchy) continue;
                        DrillRules.CutBall(ball, attackerId);
                    }
                }
            }
            catch (Exception error)
            {
                if (!errorReported) { errorReported = true; Plugin.Log.LogError("Drill ball interaction failed; report this game version and log: " + error); }
            }
        }
    }

    internal static class DrillRules
    {
        private static readonly System.Reflection.FieldInfo drillingField = AccessTools.Field(typeof(Drill), "isDrilling");
        private static readonly System.Reflection.FieldInfo protectionField = AccessTools.Field(typeof(PlayerCollision), "isInvulnerableMask");
        private static readonly System.Reflection.FieldInfo hitboxOwnerField = AccessTools.Field(typeof(Hitbox), "hitboxHandler");
        private static readonly System.Reflection.FieldInfo killedField = AccessTools.Field(typeof(PlayerCollision), "wasKilledThisFrame");
        [ThreadStatic] internal static BounceBall ContactBall;
        internal static bool IsPierceable(BounceBall ball)
        {
            if(ball==null)return false;
            // Anvil deliberately reuses BounceBall's lifecycle, but steel is not
            // Rock. No hard assembly dependency: Anvil remains an optional mod.
            foreach(Component component in ball.GetComponents<Component>())
                if(component!=null && component.GetType().FullName=="Canna.Anvil.AnvilState")return false;
            return true;
        }
        internal static BounceBall FindBall(PhysicsParent parent)
        {
            BounceBall ball = FindBall(parent.monobehaviourCollider as Component);
            if (ball == null) ball = FindBall(parent.collisionCallback as Component);
            if (ball == null) ball = FindBall(parent.fixTrans);
            return ball;
        }
        private static BounceBall FindBall(Component component)
        {
            BounceBall ball=component == null ? null : component.GetComponentInParent<BounceBall>();
            return IsPierceable(ball)?ball:null;
        }
        internal static Drill FindDrill(PhysicsParent parent)
        {
            Drill drill = FindDrill(parent.monobehaviourCollider as Component);
            if (!IsDrilling(drill)) drill = FindDrill(parent.collisionCallback as Component);
            if (!IsDrilling(drill)) drill = FindDrill(parent.fixTrans);
            return drill;
        }
        internal static bool IsDrilling(Drill drill)
        {
            return Plugin.Enabled.Value && drill != null && !drill.IsDestroyed && drill.gameObject.activeInHierarchy && (bool)drillingField.GetValue(drill);
        }

        internal static Drill FindDrill(Component collision)
        {
            if (collision == null) return null;
            Drill drill = collision.GetComponentInParent<Drill>();
            if (IsDrilling(drill)) return drill;
            Hitbox hitbox = collision.GetComponent<Hitbox>();
            HitboxCombo handler = hitbox == null ? null : hitboxOwnerField.GetValue(hitbox) as HitboxCombo;
            if (handler != null) { drill = handler.GetComponent<Drill>(); if (IsDrilling(drill)) return drill; }
            IPlayerIdHolder owner = collision.GetComponent<IPlayerIdHolder>();
            Player player = owner == null ? null : PlayerHandler.Get().GetPlayer(owner.GetPlayerId());
            if (player == null || player.CurrentAbilities == null) return null;
            // A collision callback may belong to the player's body rather than the active ability object.
            foreach (GameObject current in player.CurrentAbilities)
            {
                if (current == null || !current.activeInHierarchy) continue;
                drill = current.GetComponent<Drill>();
                if (IsDrilling(drill)) return drill;
            }
            return null;
        }

        internal static BounceBall BallOwnedBy(int id)
        {
            Player player = PlayerHandler.Get().GetPlayer(id);
            if (player == null || player.CurrentAbilities == null) return null;
            foreach (GameObject current in player.CurrentAbilities)
            {
                if (current == null || !current.activeInHierarchy) continue;
                BounceBall ball = current.GetComponent<BounceBall>();
                if (IsPierceable(ball) && !ball.IsDestroyed) return ball;
            }
            return null;
        }
        internal static bool TouchesBall(Drill drill, BounceBall ball)
        {
            DPhysicsCircle circle = ball.GetComponent<DPhysicsCircle>();
            DPhysicsBox body = drill.GetComponent<DPhysicsBox>();
            if (circle == null || body == null) return false;
            Circle shape = new Circle(); shape.center = circle.position; shape.radius = circle.radius;
            if (PhysTools.CollisionTest(shape, DrillTipPatch.ReadBox(body))) return true;
            HitboxCombo combo = drill.GetComponent<HitboxCombo>();
            if (combo == null || !combo.AreHitboxesActive()) return false;
            foreach (DPhysicsBox hitbox in combo.HitboxesList())
                if (hitbox != null && !hitbox.IsDestroyed && hitbox.gameObject.activeInHierarchy && PhysTools.CollisionTest(shape, DrillTipPatch.ReadBox(hitbox))) return true;
            return false;
        }
        internal static bool StopRockDeath(Drill drill, int killerId, CauseOfDeath cause, string entry)
        {
            if (!IsDrilling(drill) || cause != CauseOfDeath.Other) return false;
            Ability attacker = drill.GetComponent<Ability>();
            if (attacker == null || attacker.GetPlayerId() == killerId) return false;
            BounceBall ball = ContactBall;
            if(ball!=null && !IsPierceable(ball))return false;
            bool confirmed = ball != null && ball.GetComponent<Ability>().GetPlayerId() == killerId;
            if (!confirmed) { ball = BallOwnedBy(killerId); if (ball == null || !TouchesBall(drill, ball)) return false; }
            Plugin.Log.LogInfo("Blocked Rock death at " + entry + ": driller=" + attacker.GetPlayerId() + ", rock=" + killerId + ", tick=" + Updater.SimulationTicks);
            CutBall(ball, attacker.GetPlayerId());
            PlayerCollision collision = attacker.GetPlayerCollision();
            if (collision != null) killedField.SetValue(collision, false);
            return true;
        }

        internal static bool CutBall(BounceBall ball, int attackerId)
        {
            if (!IsPierceable(ball) || ball.IsDestroyed || !ball.gameObject.activeInHierarchy) return false;
            Player target = ball.getPlayer();
            if (target == null || target.Id == attackerId || !target.IsAlive) return false;
            Ability ability = ball.GetComponent<Ability>();
            PlayerCollision protection = ability == null ? null : ability.GetPlayerCollision();
            // Ball's blocking protection is precisely what Drill should pierce; preserve spawn/zone safety.
            if (protection != null && ((uint)protectionField.GetValue(protection) & (PlayerCollision.Mask_InInvincibilityZone | PlayerCollision.Mask_OnSpawnBuff)) != 0) return false;
            AbilityExitInfo exit = new AbilityExitInfo();
            exit.isDead = true; exit.killerId = attackerId; exit.causeOfDeath = CauseOfDeath.Other;
            ball.ExitAbility(exit);
            Plugin.Log.LogInfo("Drill cut Bounce Ball: player " + attackerId + " -> player " + target.Id + " at simulation tick " + Updater.SimulationTicks);
            return true;
        }
    }

    [HarmonyPatch(typeof(PlayerCollision), "KillPlayerOnCollision")]
    internal static class BallContactPriorityPatch
    {
        private static bool errorReported;
        private static int contactsLogged;
        private static bool Prefix(PlayerCollision __instance, CollisionInformation collision, ref bool __result)
        {
            try
            {
                Drill drill = DrillRules.FindDrill(__instance);
                bool spinning = DrillRules.IsDrilling(drill);
                BounceBall ball = DrillRules.FindBall(collision.colliderPP);
                if (ball == null)
                {
                    if (spinning && collision.layer == LayerMask.NameToLayer("Player") && contactsLogged++ < 32)
                        Plugin.Log.LogInfo("Active drill player contact without BounceBall component: tick=" + Updater.SimulationTicks);
                    return true;
                }
                if (contactsLogged++ < 32)
                    Plugin.Log.LogInfo("Ball contact: body=" + __instance.name + ", drill=" + (drill == null ? "none" : drill.name) + ", spinning=" + spinning + ", tick=" + Updater.SimulationTicks);
                if (spinning)
                {
                    Ability ability = drill.GetComponent<Ability>();
                    if (ability == null) return true;
                    // The engine has already confirmed contact. Requiring another, smaller tip overlap
                    // here allowed the lethal body collision to win before the tip reached the ball.
                    // An immune ball survives CutBall, but still cannot kill an actively drilling player.
                    DrillRules.CutBall(ball, ability.GetPlayerId());
                    __result = false;
                    return false;
                }
            }
            catch (Exception error)
            {
                if (!errorReported) { errorReported = true; Plugin.Log.LogError("Drill ball collision priority failed: " + error); }
            }
            return true;
        }
    }

    [HarmonyPatch(typeof(Drill), "startDrill")]
    internal static class DrillStartedPatch
    {
        private static void Postfix(Drill __instance)
        {
            Plugin.Log.LogInfo("Drill spinning: player=" + __instance.GetComponent<Ability>().GetPlayerId() + ", tick=" + Updater.SimulationTicks);
        }
    }

    [HarmonyPatch(typeof(BounceBall), "OnCollide")]
    internal static class RockCollisionPatch
    {
        private static bool errorReported;
        private static bool Prefix(BounceBall __instance, CollisionInformation collision)
        {
            if(!DrillRules.IsPierceable(__instance))return true;
            try
            {
                Drill drill = DrillRules.FindDrill(collision.colliderPP);
                if (!DrillRules.IsDrilling(drill)) return true;
                Plugin.Log.LogInfo("Rock.OnCollide intercepted drill at tick " + Updater.SimulationTicks);
                DrillRules.CutBall(__instance, drill.GetComponent<Ability>().GetPlayerId());
                return false;
            }
            catch (Exception error) { if (!errorReported) { errorReported = true; Plugin.Log.LogError("Rock callback hook failed: " + error); } return true; }
        }
    }

    [HarmonyPatch(typeof(PlayerCollision), "OnCollide")]
    internal static class PlayerContactPatch
    {
        private static void Prefix(CollisionInformation collision, out BounceBall __state)
        {
            __state = DrillRules.ContactBall;
            DrillRules.ContactBall = DrillRules.FindBall(collision.colliderPP);
        }
        private static Exception Finalizer(Exception __exception, BounceBall __state)
        {
            DrillRules.ContactBall = __state;
            return __exception;
        }
    }

    [HarmonyPatch(typeof(PlayerCollision), "killPlayer")]
    internal static class KillPlayerPatch
    {
        private static bool errorReported;
        private static bool Prefix(PlayerCollision __instance, int killerId, CauseOfDeath causeOfDeath, ref bool __result)
        {
            try
            {
                if (!DrillRules.StopRockDeath(DrillRules.FindDrill(__instance), killerId, causeOfDeath, "PlayerCollision.killPlayer")) return true;
                __result = false;
                return false;
            }
            catch (Exception error) { if (!errorReported) { errorReported = true; Plugin.Log.LogError("killPlayer hook failed: " + error); } return true; }
        }
    }

    [HarmonyPatch(typeof(Drill), "ExitAbility")]
    internal static class DrillDeathPatch
    {
        private static bool errorReported;
        private static bool Prefix(Drill __instance, AbilityExitInfo info)
        {
            if (!info.isDead) return true;
            try
            {
                Plugin.Log.LogInfo("Drill.ExitAbility death request: killer=" + info.killerId + ", cause=" + info.causeOfDeath + ", spinning=" + DrillRules.IsDrilling(__instance) + ", tick=" + Updater.SimulationTicks);
                return !DrillRules.StopRockDeath(__instance, info.killerId, info.causeOfDeath, "Drill.ExitAbility");
            }
            catch (Exception error) { if (!errorReported) { errorReported = true; Plugin.Log.LogError("Drill death hook failed: " + error); } return true; }
        }
    }
}
