# Canna family visual extensions

Independent BepInEx add-ons for the unmodified CustomLocalColorsRedux 1.0.0 and ArrowTrajectories 1.2.2 downloads. Upstream authors and READMEs remain in their packages.

Shared Colors: F8 opens an RGB picker. Lobby member metadata shares your selected color under your Steam identity. Every viewer needs this package. Original local pickers remain available. Each player's palette uses an independent material. No gameplay RNG is consumed.

Friends Trajectories: F9 configures own, teammate and opponent paths independently. Own paths default on; other paths default off. Invisible players remain hidden. Prediction reads native synchronized bow state, uses the exact charged-arrow launch equation and integrates a copied native PhysicsBody. It stops at terrain. Future moving islands, black holes, portals and ArrowWall's additional fan arrows are not predicted.

Build with `./build.ps1` against your installed Bopl Battle. Never distribute game assemblies. The development catalog auditor is not included in the release packages.

Validated against Bopl 2.5.1: member-reference compatibility for all catalog DLLs, combined plugin loading, four-slot native spawn, color/material isolation, a real Steam member-data round trip, native arrow launch velocity and remote visibility fixtures. A second-PC online match and every individual gameplay effect remain unverified. The level-only fixture bypasses the fourth-ability menu icon initialization; test the normal menu flow in multiplayer.
