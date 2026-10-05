# Canna Anvil 1.0.5

1.0.5 removes the forced downward impulse and rotation locks. Standard gravity
and native angular physics drive movement, bounce and free rotation. Mass stays
at twice Rock's, half of version 1.0.3. The team-colored HUD circle, faster entry
and exit animations, and collider aligned with the drawn feet are retained.
A tap completes the roughly 0.067-second entry and lasts five seconds, including it;
release does not cancel it. Native timeout, death and return-to-slime handling remain.
The morph begins with your slime color and fades into steel; exit reverses it.
AbilityScrollBar 1.0.1 is a catalog dependency so the appended ability fits the picker.

Adds **Anvil** at the end of Bopl Battle's ability picker, without replacing Rock.
Turn into a cartoon steel anvil, retain your momentum, and crush opponents using
the game's native contact combat. Twice Rock's mass, standard gravity, native bounce and friction,
five-second duration, and six-second cooldown.

Original 17-frame vector morph artwork plays forward on entry and backward on exit.
Native fixed-point physics, rounded Rock collision hull, ownership, scale changes,
rope support, time stop, and death/exit handling are retained. This first version
uses the game's circular Rock hull rather than a concave anvil-shaped collider.

Requires Bopl Battle 2.5.1, Windows x64, BepInEx 5.4.23.5 (Harmony included).
Everyone in an online lobby must enable the same version and identical ability mods.
The host checks Anvil version advertisements before starting a round.
Actual multiplayer verification requires a second PC.

Install through Canna Discover, add to your Bopl modpack, then Apply modpack.
Disabling/removing the mod requires a game restart. Existing native ability indices
remain unchanged. Remove saved Anvil selections before playing without the mod.

`build.ps1` compiles against your locally installed game; the package contains only
original plugin code. `Audit.cs` is a development-only check and is never packaged.



