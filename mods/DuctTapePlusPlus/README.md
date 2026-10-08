# Canna Rebound preview

This component prepares an isolated plugin tree for the reviewed Windows public
ROUNDS build. It combines pinned MIT DuctTape/toolkit code, exact upstream curated
ports, checked mechanical IL mappings, asset checks and a multiplayer manifest
guard. It never patches the installed game at startup.

Desktop 0.2.41 includes it as **Settings → Canna Rebound for ROUNDS (preview)**,
off by default. Enable it before Setup, Apply modpack or Launch modded. Ordinary
desktop updates remain enabled; only the separate development Preview.exe disables
updates. With Rebound off, ordinary installs and the official DuctTape workflow
keep their existing behavior. With it enabled, unsupported builds or calls block
preparation before installation. Known old dependency selections remain editable
and their reviewed modern ports are substituted without changing saved choices.

Internal paths and protocol names retain DuctTape++ for the existing preparation
format; the user-facing feature is Canna Rebound. Upstream DuctTape/toolkit
attribution remains part of the source and license notices.

The current unpublished desktop source changes distribution: normal debug and
release builds contain no support DLL bundle. The account must have the additional
Beta role, verified by the server before support download/cache access and again
before preparation or installation. Beta does not grant administration. The
protected server endpoints and support artifact must be deployed before this
workflow is usable. Previously distributed 0.2.41 executables retain their original
bundle. Private test fixtures are excluded from ordinary runtime authorization.

The current profile is `rounds-public-1.1.2`, protocol `canna.ducttape++/1`, with
Assembly-CSharp SHA256
`20451cc7090908cd1d125f75f06584645d25e898ec234de0a0c2f154e2900668`.
Other game builds require a reviewed profile. This one game assembly fingerprint
does not attest every Unity, Photon, loader or operating-system component.

`build.py --dotnet <portable SDK> --game <read-only ROUNDS references>` produces
`target/ducttape-plus-plus/support.zip`. `--core`, `--output`, `--upstream-source`,
`--toolkit-source`, `--unbound-source` and `--friends-source` are optional.
The builder requires clean pinned source trees or downloads them under its output.
It keeps NuGet and SDK state there. It does not build the UnboundLib project,
whose upstream postbuild target copies into a game installation.

The net472 helper runs without a user .NET SDK:

```text
helper/Canna.DuctTapePlusPlus.Translate.exe --request request.json --report report.json
```

The JSON request contains absolute `game_root`, `plugins`, `core`, optional
`config` and `patchers` directories, and `declared_dependencies` strings. Plugins
must be a temporary directory outside the game and core, with a
`.canna-ducttape-staging` file containing the protocol. Config is an isolated
snapshot; patcher content is unsupported in this preview. Reports must be outside
plugins/game/core and within the preparation's parent directory.

The helper checks source bytes, works in a sibling copy, resolves only referenced
or exact declared supported dependency closure, applies exact curated changes and
mechanical mappings, and validates the complete result with a fresh resolver.
Only then does it replace the temporary input tree. It retains the original
temporary tree for the caller's recovery/cleanup. Unknown replacement library
bytes, unsupported APIs, unresolved hard plugin dependencies or minimum versions,
duplicate assembly/plugin identities, unknown assets/preloaders and unsigned
rewrite of a strong-named assembly block preparation. Display names alone do not
authorize omission of selected content.

Providers must be concrete classes derived from the actual BaseUnityPlugin with
valid loader GUID/version metadata. Inherited hard dependencies, process filters
and incompatibilities are checked: one matching reviewed ROUNDS process filter
permits loading; incompatible present providers block preparation. The installed
loader's extension handling is case-sensitive before case-insensitive process
comparison, so `ROUNDS.exe` and `rounds` match while `ROUNDS.EXE` does not.
Foreign attributes imitating loader metadata block as ambiguous, because this
loader's Cecil reader matches attribute names without checking assembly scope.

`--prepare-payloads --game <references> --core <references> --report <output>` is a
build-only command. It fixes/scans the pinned payload set before the builder
records its final hashes. `payloads/index.json` records source hashes and
normalization steps; `upstream.lock.json` pins original sources. The support
manifest binds every bundled file, including source archives and notices.

Reports contain `ok`, `required`, `protocol`, `profile`, `game_sha256`,
`manifest_sha256`, `fingerprint`, `translated`, `replacements`, `warnings` and
`errors`. Output file paths are relative to plugins. Review warnings are visible
limitations, including uncertain dynamic reflection and asset behavior; a clean
static result does not prove full gameplay or multiplayer behavior.

The prepared manifest binds managed assembly identity/hash and immutable assets;
runtime configuration is recomputed, excluding only `BepInEx.cfg`. Multiplayer
guard checks compare prepared bytes and effective config across peers and gate
supported readiness/start paths. This detects prepared-content mismatches; it
does not prove that every mod's synchronization or arbitrary custom game mode
is correct. Fixture and isolated-game smoke checks must be reported separately
from real multi-client gameplay.

All bundles are currently marked
`preview-redistribution-unverified`. DuctTape/toolkit are MIT, several
patched components or changes are GPL-3.0, Odin is Apache-2.0 and Octokit is MIT.
UnboundLib/MMHook and some other source projects have no blanket downstream
license established here. The source/notice archives are retained, but publication
of replacement binaries has unresolved inherited-code distribution scope. Bknibb
has said he is personally fine with redistribution of source he wrote while
flagging the original inherited UnboundLib code; this is not a blanket license:
https://github.com/Bknibb/UnboundLib/issues/2#issuecomment-6052691146
Credits and source notices do not assert broader permission. Test-only
SmokeChecks sources in the Canna source archive are not active runtime payloads.
