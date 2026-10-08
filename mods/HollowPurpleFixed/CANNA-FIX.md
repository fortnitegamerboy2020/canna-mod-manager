# HollowPurple Fixed 1.8.2 — public ROUNDS

Original mod: **flofl / Hollow Purple contributors**. Public-version port: **Canna**.
Original project and author: https://thunderstore.io/c/rounds/p/flofl/HollowPurple/

This separate Canna fork preserves the original description, icon, procedural assets
and license notices. The bundled upstream README and validation documents describe
the original 1.8.0 release; this file describes the public-version port. The initial
legacy-only Canna 1.8.1 build was withdrawn and is superseded by this port.

## Changes

The pinned original archive is SHA-256
`27fcd1c99fb30b24fea84b730d046445c06889ab78b45d5dc46164fcb34a531b`.
Canna adjusts changed public game IDs, health properties, damage signatures, damage
RPCs, Unity Input resolution and the moved RenderPipelineAsset type. A new narrow
adapter provides this mod's custom-card registration/application, battle hooks and
Photon transport calls without loading old UnboundLib/MMHook. It supports only
HollowPurple's used interfaces, not other UnboundLib mods. The port uses a separate
network protocol channel/version to avoid silently mixing with the original mod.
Original embedded resources and gameplay assets are preserved.

Card-art prototypes now live under an inactive prefab root so their artwork cannot
render at the world origin. Opt-in diagnostics use the native sandbox/menu transition
and dispose of their temporary card clones after each test pick. Normal card picking
keeps the game's own animation and network lifecycle.

## Installation and limits

- In Steam, choose ROUNDS → Properties → Betas → **None** (default public version).
- Create a separate public-version pack with BepInExPack_ROUNDS 5.4.1900. Leave
  legacy UnboundLib 3.2.14 / MMHook 1.0.0 and their dependent mods in an old-branch pack.
- Remove or disable the original HollowPurple before adding this fork. It retains
  the original plugin GUID and DLL path for configuration and dependency compatibility.
  Both packages must not be installed together.
- Every multiplayer participant needs this same fork version and compatible dependencies.
- Public build **21020021**, Unity **2022.3.34f1**: metadata checks resolve all referenced
  type/member references and confirm the identical logo, description and licenses.
  The real-game local/offline smoke test passed **70 checks**, including all 26 cards,
  normal card ownership, charge/projectile collisions, damage, friendly fire,
  cooldown/point cleanup and Gojo, Sukuna, Yhwach and Goku abilities. Four additional
  end-of-run scene checks passed: all card prototypes registered, artwork inactive,
  the native menu closed during gameplay and temporary diagnostic clones destroyed.
  Earlier runs included Card Control 1.1.2; the final cleanup run used only this port
  and the test-only check plugin. The user reported that the final test looked fine.
  Offscreen renders were inspected. This does not establish a complete human match,
  normal online match behavior or two-client synchronization. The final run logged
  one native Sonigon/AudioSource exception during shutdown; no DamageBox collision
  exception or missing game API was logged in that run.
- Future public game updates and other mod combinations need new verification.
  The test machine's older BepInEx 5.4.11 produced a version warning while loading
  the mod; the package dependency remains BepInExPack_ROUNDS 5.4.1900.
- Original opt-in diagnostics (`--hp-smoke`) can write/overwrite files in the chosen
  output directory. They have not been removed or made into telemetry.

No game logs or Steam credentials are bundled. Original code retains its MIT license
and original procedural assets their CC0 notice. Canna's adapter and scripts are MIT
licensed; see CANNA-LICENSE.txt. The normal card picker and original mod key bindings
remain available. Two-client multiplayer is **not verified** yet.
