# Canna Anvil 1.0.7

1.0.6 replaces the circular Rock hull with a native flat-sided box fitted to the
artwork. It can tip and settle on its top, base or side instead of rolling like
a ball. Low restitution and native friction help it stay settled; rotation is
not locked. Standard gravity, inherited momentum, twice Rock's mass,
team-colored HUD circles and the fast entry/exit animations are retained.
A tap completes the roughly 0.067-second entry and lasts five seconds, including it;
release does not cancel it. Native timeout, death and return-to-slime handling remain.
The morph begins with your slime color and fades into steel; exit reverses it.
AbilityScrollBar 1.0.1 is a catalog dependency so the appended ability fits the picker.

Adds **Anvil** at the end of Bopl Battle's ability picker, without replacing Rock.
Turn into a cartoon steel anvil, retain your momentum, and crush opponents using
the game's native contact combat. Twice Rock's mass, standard gravity, native bounce and friction,
five-second duration, and six-second cooldown.

Original 17-frame vector morph artwork plays forward on entry and backward on exit.
Native fixed-point box physics, ownership, scale changes, time stop and
death/exit handling are retained. The rectangular hull approximates the
silhouette rather than tracing its narrow waist. Upright, upside-down and
sideways drops, native contact combat, momentum, rotation and exit were checked
in the native audit. Live family multiplayer and rope behavior still need testing.

Requires Bopl Battle 2.5.1, Windows x64, BepInEx 5.4.23.5 (Harmony included).
Everyone in an online lobby must enable the same version and identical ability mods.
The host checks Anvil version advertisements before starting a round.
Actual multiplayer verification requires a second PC.

Install the published version through Canna Discover, add to your Bopl modpack,
then Launch modded to install the enabled mods while the game is closed.
Disabling/removing the mod requires a game restart. Existing native ability indices
remain unchanged. Remove saved Anvil selections before playing without the mod.

`build.ps1` compiles against your locally installed game; the package contains only
original plugin code. `Audit.cs` is a development-only check and is never packaged.





1.0.7 routes Magnet wall correction through the registered box instead of the circle facade. It preserves native pull, pickup, hold and launch behavior. An offline native-game audit reproduced the old InvalidCastException and passed 70 checks with the fix, including pickup, twelve holding ticks, input-release launch, wall separation through both overloads, ordinary Rock, rotation, landing, combat and exit. The production build contains no audit code; both wall-fix methods match the tested IL. Released Magnet compatibility fix. A full multiplayer match remains unverified.
