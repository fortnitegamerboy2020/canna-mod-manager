# Canna Anvil 1.0.1

1.0.1 separates the padded, eyeless menu icon from a gameplay sprite twice as large.
The native circular collision radius is doubled too. A tap now completes the
0.55-second morph and holds the finished pose briefly before release cancels it.
The morph begins with your slime color and fades into steel; exit reverses it.
AbilityScrollBar 1.0.1 is a catalog dependency so the appended ability fits the picker.

Adds **Anvil** at the end of Bopl Battle's ability picker, without replacing Rock.
Turn into a cartoon steel anvil, slam downward in midair, and crush opponents using
the game's native contact combat. Four times Rock's mass, 2.5 times its gravity,
low bounce, four-second duration, and six-second cooldown. Press the ability again
to cancel using the native return-to-slime flow.

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
