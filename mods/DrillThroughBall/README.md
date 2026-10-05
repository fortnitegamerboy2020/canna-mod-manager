# Drill Through Ball 1.0.4

A Canna family Bopl Battle mod. While Drill is actively drilling, its scaled deterministic attack hitboxes can defeat another player who has transformed into Bounce Ball. Normal terrain, loose boulders, other abilities, and the drill's inactive animation are unchanged. Self-hits, spawn immunity and invincibility zones are excluded. Confirmed contact with Bounce Ball favors an actively spinning drill, including body contact. Entering or exiting the drill animation provides no protection. Spawn and zone immunity still prevent killing the ball. The normal ability-death exit path handles kill credit and revival.

Requires BepInEx 5 with its included Harmony libraries. Install the ZIP using Canna's Import local mod or select it from the family catalog. The ZIP contains only this plugin and its README; no game assemblies are redistributed.

Configuration: `BepInEx/config/family.canna.drillthroughball.cfg`, `[General] Enabled = true`.

All participants in an online match need the same version and setting because this changes deterministic gameplay. Compiled against the owner's installed Bopl Battle assemblies. In-game collision, timing, revival and multiplayer behavior require playtesting; compilation is not gameplay verification.

Build with `./build.ps1`, optionally supplying `-GamePath` and `-FrameworkZip`. Build references remain local.

Version 1.0.4 hooks Rock's BounceBall.OnCollide callback, PlayerCollision.OnCollide and killPlayer, and Drill.ExitAbility before death cleanup. It also hooks startDrill and the existing drill hitbox query. Collider resolution checks the collider component, callback component and FixTransform. Death fallback requires Rock's killer ID and confirmed contact or a deterministic body/tip overlap; other death causes continue normally. Startup reports all registered hooks, and activation/contact/death events appear in BepInEx output.

Bopl removes the plugin Unity object during initial scene loading. Hooks intentionally remain installed until the game process exits; destroying that object must not unpatch gameplay methods. Version 1.0.4 fixes this lifetime issue.
