# Canna Procedural Maps 1.0.0

Bopl Battle 2.5.1 / Windows Mono / BepInEx 5.4.23.5.

Generates new island positions, sizes and four safe team spawn locations before native level initialization. Reuses Bopl's platform assets and creates additional islands on sparse maps. Some rounds use native space gravity. Upper islands move through the native platform physics controller using deterministic simulation ticks. Sudden death and player control of islands retain native behavior.

Every participant must install and enable this version. Steam lobby member metadata prevents the modded host starting a round with a missing or mismatched generator. Bopl's existing host start packet supplies the shared seed; the generator does not send a separate map packet or use Unity's random generator. Seed and layout fingerprint appear in BepInEx output and Canna's Console tab.

Native scene audits and the latest generated map are written to `BepInEx/config/CannaMaps`. Each round has six to nine islands independent of the original map's platform count; excess native islands are removed through Bopl's destruction queue. Scenes with no platform template and replays retain native maps. This initial version requires family multiplayer testing; deterministic layout tests do not establish that the whole game remains synchronized. Keep this mod disabled for public matchmaking.

Build using `build.ps1` with locally installed game assemblies. Game code and assets are not included in the package. `TestLayout.cs` exercises 10,000 seeds, spawn safety, identical layout fingerprints, platform separation across movement ranges, and motion periodicity.
