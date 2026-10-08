# Canna Rebound preview

This component prepares an isolated plugin tree for the reviewed Windows public
ROUNDS build. It combines pinned MIT DuctTape/toolkit code, exact upstream curated
ports, checked mechanical IL mappings, asset checks and a multiplayer manifest
guard. It never patches the installed game at startup.

Desktop 0.2.45 offers it as **Settings → Canna Rebound for ROUNDS (preview)**,
off by default. Enable it before Setup, Apply modpack or Launch modded. Ordinary
desktop updates remain enabled; only the separate development Preview.exe disables
updates. With Rebound off, ordinary installs and the official DuctTape workflow
keep their existing behavior. With it enabled, unsupported builds or calls block
preparation before installation. Known old dependency selections remain editable
and their reviewed modern ports are substituted without changing saved choices.

Internal paths and protocol names retain DuctTape++ for the existing preparation
format; the user-facing feature is Canna Rebound. Upstream DuctTape/toolkit
attribution remains part of the source and license notices.

Desktop 0.2.42 and newer use the shipped server-verified Beta workflow, supported
by Community 0.3.58 and newer. Normal debug and release builds, including
`cargo run --release`, contain no support DLL bundle. The account must have the
additional Beta role, verified by the server before support download/cache access
and again before preparation or installation. Beta does not grant administration.
Support is downloaded only through the protected server endpoint after that
authorization. Previously distributed 0.2.41 executables retain their original
bundle. Private test fixtures are excluded from ordinary runtime authorization.

Desktop 0.2.45 also verifies fresh server Beta access before Launch current
dispatches an already-active managed Rebound setup. Applied metadata binds the
authorized support archive, installed compatibility manifest and game assembly
hashes. Older 0.2.44 bindings require one reapply; revoked access, changed support
or altered inputs stop dispatch. This check reads metadata without downloading,
purging support or modifying game files.

Desktop 0.2.44 can receive a protected support refresh without a new desktop
executable. Preparation reads the authorized server manifest and uses a cached ZIP
only when its size and SHA256 match that manifest. A changed support hash selects
the new download; a change after preflight requires preparing the pack again.
After a support release, close ROUNDS and **Apply modpack** again with Rebound
enabled and current Beta access. Already loaded game DLLs do not refresh in a
running session. Setup restores the tracked runtime; Apply modpack or Launch
modded prepares the current pack.

## CR visual and projectile lifecycle refresh

The protected Community 0.3.63 support refresh contains two bounded CR 2.7.0
repairs. Glue removes its two intended explosion components immediately from its
visual prototype before cloning it, using Unity's one-argument cleanup overload
that does not permit asset destruction. The pinned getter's other destruction
calls are unchanged. Satellite Start returns only for an unparented prototype,
matching its existing Update policy; a parented projectile still executes the
original initialization and synchronization body. Unknown versions or changed
method fingerprints are refused before translation writes.

Paired isolated game copies reproduced the Glue effect exception with the prior
support bytes and removed it with the cleanup repair while retaining native ammo,
damage source attribution and reversible surface effects. A separate native
Satellite probe distinguished the unparented template failure from a real bullet
clone with valid movement, synchronization and ownership. These are controlled
local Gun.Attack and explicit native HitInfo checks, with disposable offline AI
input paused in the copy. Natural collision, physical input, full matches and
two-client multiplayer remain unverified. Earlier card-picker effect/audio errors
are historical observations, not a claim that every CR card is now error-free.

## Card-picker compatibility refresh

The protected support refresh for Community 0.3.62 makes bounded repairs
to the reviewed dependency bytes and native game contracts:

- The unique-card dependency resolves the actual picker PlayerID instead of using
  it as a roster index. Its team branch and card eligibility rules are retained.
- The native player-pick application resolves that same identity. Unbound's bar
  rebuild uses roster slots for display ownership, and the runtime binds each
  player to the bar created for that player. Native AddCard selects the bound bar
  while Unbound CardData keeps the actual PlayerID as its key. Rebuilt gameplay
  bindings refuse absent players or changed ownership until bars are rebuilt;
  unbound menu previews retain the native bounded slot API. Players are not
  globally renumbered. The exact ModdingUtils AddCard guard admits only verified
  bound players whose original rejection was its obsolete array-index bound;
  missing owners, unbound previews and other Harmony vetoes remain rejected.
- The exact ModdingUtils dynamic ProjectileInit patches retain their six method
  targets while removing a conflicting class-level Harmony target annotation.

The final isolated game copy passed 204 assertions over six controlled native
local turns using disposable offline AI players with IDs 0/1, 0/2 and 1/2.
Checks cover drawn cards, intended-player application, the bound bar/button,
unchanged other bars, original-ID CardData and completed handoffs; all six
ModdingUtils targets registered. Single-application assertions select offered
cards outside CardManipulation: CR Egg legitimately adds cards in its callback.
Temporary prefab-effect/audio exceptions occurred, and a prior copy had a
startup readiness failure. These synthetic IDs and programmatic picks do not
certify physical input, a full match, firing, online play or two-client multiplayer.

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
runtime configuration is recomputed, excluding only the root loader file
`BepInEx/config/BepInEx.cfg` (case-insensitively). Nested files with that name,
gameplay configuration and changed game assemblies remain checked. Multiplayer
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

Community 0.3.66 recognizes exact reviewed Rebound-supplied dependency releases
when Beta members share ROUNDS packs. Shared original archives retain a preview
requirement; recipients still need their own verified Beta access before support
download/storage and installation. Unknown missing dependency pins remain blocked.
Rebound peer parity uses bound active settings, retains unknown configs, and
ignores config comments/order, two known inactive plugin configs and local mouse
lock. Gameplay values and immutable content still must match. Warnings separate
DLL/support, asset/patcher and gameplay-setting mismatches. Both players must
close ROUNDS and reapply their pack after a support update. Offline/Harmony tests
are not proof of a full live multiplayer match.

The exact archived CR 2.7.0 ZIP (db059e5c...ee5ea7) also records four
legacy patch requirements retired by its reviewed curated adaptation. Only that
archive pin can omit CardThemeLib 1.1.7, GravityPatch 0.0.0, ZeroGBulletPatch
1.1.0 and StopShootingYoureDead 0.0.0. Unknown CR archives remain blocked.
