# Canna Rebound runtime source adaptations

Upstream DuctTape runtime source remains pinned to commit
`02e1b3d561f1a3dee321a893f111934ab3a2e1df` under its MIT license.
The original checkout is unchanged and included in `source/DuctTape.zip`.

The task-local build copy changes two Harmony prefixes in UnboundLibFixes.cs for
the reviewed BepInEx 5.4.11 HarmonyX core, which does not support generic `__args`
injection:

- The health-bar prefix uses its known CharacterData argument at index 1 and
  retains the non-null player/stat checks.
- The update-notice prefix uses its known object argument at index 0, retains
  `__originalMethod`, and retains the exact version/repository skip conditions.

`source/runtime-source-adaptations.json` records original/adapted source SHA256
and changes. `source/runtime-adapted-source.zip` includes the exact build sources.
Unexpected original source shapes or any remaining generic `__args` injection
fail the build. Canna's adaptation code is MIT; upstream notices remain required.
