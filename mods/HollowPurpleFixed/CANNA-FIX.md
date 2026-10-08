# HollowPurple Fixed 1.8.1

Original mod: **flofl / Hollow Purple contributors**. Compatibility patch: **Canna**.
Original project and author: https://thunderstore.io/c/rounds/p/flofl/HollowPurple/

This is a separate Canna fork of HollowPurple 1.8.0. The original package description,
icon, procedural assets, README and license notices are retained unchanged. Read this
file for the fork's version and installation instructions; historical version labels
in the original README describe the upstream release.

## Fix

The upstream DLL resolves UnityEngine.Input through UnityEngine.CoreModule. Newer
Unity versions move that type to InputLegacyModule. Canna redirects the reference
through the UnityEngine compatibility facade so Unity can resolve its own Input module.
The plugin display name/version become HollowPurple Fixed / 1.8.1. Gameplay method
bodies and embedded resources are unchanged. The patch script accepts only upstream
archive SHA-256 27fcd1c99fb30b24fea84b730d046445c06889ab78b45d5dc46164fcb34a531b.

## Installation and limits

- In Steam, choose ROUNDS → Properties → Betas → **Old ROUNDS for mods**
  (`old-rounds-for-mods`), then wait for Steam to finish updating.
- Use BepInExPack_ROUNDS 5.4.1900, UnboundLib 3.2.14 and MMHook 1.0.0.
- Remove or disable the original HollowPurple before adding this fork. It retains
  the original plugin GUID and DLL path for configuration and dependency compatibility.
  Both packages must not be installed together.
- Every multiplayer participant needs this same fork version and compatible dependencies.
- Metadata checks verify corrected Unity type resolution against the locally installed
  public ROUNDS assemblies, unchanged method bodies/resources and identical assets.
  This does **not** establish public-branch gameplay compatibility. Full gameplay,
  class abilities and multiplayer on the required old branch still need live testing.
- Original opt-in diagnostics (`--hp-smoke`) can write/overwrite files in the chosen
  output directory. They have not been removed or made into telemetry.

Original code remains under its included MIT license; original procedural assets
retain their included CC0 notice. Canna's patch scripts are covered by Canna's MIT license.
