# Family Thunderstore catalog

20 requested mods plus AntiMatchmaking, mirrored at their latest versions checked at publication. Original authors, manifests, README files and assets are preserved. CustomLocalColorsRedux and ArrowTrajectories include independent Canna add-ons; upstream DLLs remain unchanged. Their extended package versions end in `-canna.1`.

`scripts/Publish-ThunderstoreCatalog.ps1` checks the pinned versions against the live Thunderstore API before publishing, rejects unresolved dependencies and uploads an atomic catalog commit. SHA-256 hashes pin every package. `THUNDERSTORE-SOURCES.json` records upstream versions and download provenance. This is a family mirror, not a promise that upstream authors tested all mods together.

AlmightyPush, ImpactNade and AcidTrip require AntiMatchmaking; Canna 0.2.3 adds and enables that dependency. FourthAbility ships both original slot and stable-repair DLLs. Do not change the base game's ability-count setting while using it. Gameplay-changing mods should match on every participating PC.

AcidTrip has a photosensitivity warning and stays disabled in the example pack. Most gameplay-changing mods are opt-in; adding everything does not imply every combination makes a sensible game.

`Audit.cs` is a development-only harness, never shipped in mod archives. Checks cover native references, combined loading, four-slot spawning, isolated player colors, exact native charged-arrow launch velocity, remote trajectory visibility fixtures and a real private Steam lobby member-data round trip. The level-only test skips menu icon initialization; it does not replace normal-menu or second-PC multiplayer tests.
