# Canna Mod Manager

A native Rust desktop mod library for you and your family. Dark forest colors, Steam library discovery, Bopl Battle as the first supported game, and the private Canna server as the mod catalog. Uploads and external imports are managed on the website.

Version 0.2.4 uses a compact icon sidebar and a borderless window. The yellow
button in the top right minimizes; the red button beside it closes Canna. Drag
the header to move the window, or double-click it to maximize/restore. Hover
sidebar icons for their names. Rounded panels share the green canvas behind
their corners, avoiding black gaps.

## Run

Double-click `dist/Canna Mod Manager.exe` after building, or run:

```powershell
cargo run
```

Build a portable executable with `./build.ps1`. Rust and Windows C++ build tools must already be available. No installer or administrator access is required. Settings are stored at `%APPDATA%/CannaModManager/settings.json`. The executable can be copied to a family member's Windows PC.

## What works now

- Detects Steam from Windows registry and standard locations; follows all `libraryfolders.vdf` libraries, including other drives and legacy library formats.
- Reads installed app manifests and verifies the game directory exists. Only lists Windows Unity games detected from `UnityPlayer.dll` and a corresponding `_Data` folder. Unreal games and other Steam entries are excluded because this version has no framework for them. Search and a family-catalog filter are available.
- Uses cached Steam artwork, with repository artwork taking priority after sync. Bopl Battle remains visible as a starting point if it is not installed.
- Detects BepInEx core assemblies, proxy DLL and Doorstop configuration; distinguishes detected, incomplete, and absent files. This is a file check, not proof the loader works when launched.
- Counts local plugin DLLs, opens the game folder, and launches installed games through Steam.
- Reads game metadata, icons, framework packages and mod listings from the authenticated Canna server on a background thread. Reports expired sessions, malformed metadata and network errors.

Creating a modpack automatically sets up BepInEx. **Add Mods** opens **Discover**, where you can search the family catalog, choose a compatible modpack, and add or update a pinned mod; **Import local mod** adds a DLL or plugin ZIP. **Apply modpack** activates the selection in `BepInEx/plugins/Canna`. **Launch modded** installs the selected pack and launches through Steam. **Launch vanilla** disables Doorstop before launching. **Stop instance** terminates the game process Canna launched, using a retained Windows handle. It appears in the pack, its right-click menu, game details, and navigation while that process runs. Independently launched games are never adopted. The last mode remains selected until changed. Existing plugins outside Canna are preserved and also load in modded mode.

Bopl Battle uses the official Windows x64 BepInEx 5.4.23.5 archive from `bopl-battle/Framework/BepInEx.zip`. Harmony is included upstream; this is not a custom BepInEx fork. Other Unity games need their own compatible Windows BepInEx 5 package at `<game>/Framework/BepInEx.zip`. Existing complete loaders are preserved, and conflicting files are reported rather than overwritten. Close the game before setup, installation or mode changes. Gameplay compatibility still requires testing.

## Connect your private repository

Create a private repository named `manager-uploaded-mods` (or choose another name), and copy the **contents** of `repository-template` to the repository root:

```text
catalog.json
bopl-battle/
  game.json
  icon.jpg
  Mods/
    README.txt
    your-mod.zip
```

Open **Settings** and choose **Sign in & connect account**. Enter the six-character code on the dedicated website verification page. The desktop session is stored encrypted with Windows DPAPI and remains signed in until logout or revocation.

The desktop reads `/api/v1/catalog` and `/api/v1/catalog/file` from Canna. Legacy modpack file paths remain available as server aliases; Bopl artwork, BepInEx and catalog archives have been migrated. The mod GitHub token is no longer embedded in desktop builds. Keep `canna-mod-manager` for application source and release archives; desktop updates are served by Canna.

The website library supports search, game, content type, provider, Minecraft version and loader filters. **Add Mod From External Site** previews a Thunderstore or Modrinth project before importing a selected version. CurseForge needs its server-only API key. Website downloads offer opening Canna, show connection/transfer status, and offer a manual download when no client connects.

Steam catalog metadata and artwork are cached locally for offline browsing. Sessions are never stored in this cache. Historical repository templates below document the migration input, rather than the current transport.

To add another game, add its folder to `catalog.json` and give it a unique Steam app ID in `game.json`. File paths are relative to the game folder. Mod files must be beneath `Mods/`:

```json
{
  "app_id": 1686940,
  "name": "Bopl Battle",
  "description": "Our family mod collection",
  "icon": "icon.jpg",
  "mods": [
    {
      "name": "Your actual mod name",
      "version": "1.0.0",
      "description": "What this mod does",
      "file": "Mods/your-mod.zip",
      "sha256": "optional SHA-256 checksum verified during installation"
    }
  ]
}
```

The template intentionally has no fictional mods. The included Bopl artwork is copied from Steam's local library cache, for this private family setup.

## Create, export and import modpacks

Right-click modpack cards or their detail header for Open, Add Mods/Edit, local import, Duplicate, Export, Install, launch options, and Delete. The card's three-dot menu exposes the same actions. Delete is also available on the detail page. Deleted local manifests move into `modpacks/deleted/`; **Undo delete** restores the latest deletion during the current session. Deleting a pack preserves installed game files and shared local mod files. Right-click game cards for View, Create modpack, launch options, and folder actions. Right-click a mod's name/file entry to copy its details or remove it from the pack.

Open **Modpacks** in the sidebar, then **Create modpack**. Name the pack, select its game, optionally add a description, and check the mods from the connected catalog. **Save modpack** stores it locally at `%APPDATA%/CannaModManager/modpacks/`. An empty pack can be created before connecting a repository; edit it after your catalog is ready.

The modpack library uses game-art cover cards, sorting and game filters. **New modpack** opens a choice between a custom setup and importing a family pack. Creation and editing use a centered dialog with game, cover color, group, description and mod choices. **New group** creates a named collection and can assign existing packs. Groups and cover colors are preserved during export/import. Opening a card shows its **Content** table and **Pack details**. In the game library, **Create modpack** starts a setup with that game selected.

Saved packs can be edited, duplicated and searched. **Remove** removes a selection; click **Apply modpack** to apply that change. **Export** writes a `.canna.zip` bundle containing the manifest and any imported local mod files. Repository mods remain version/checksum-pinned references. **Import** supports both `.canna.zip` and older `.canna.json` manifests, validates paths and checksums, and assigns a fresh local pack ID. Exports contain no tokens or local game paths. After importing, use **Apply modpack** or **Launch modded**.

The family catalog now includes **Drill Through Ball 1.0.4**, a BepInEx plugin built from `mods/DrillThroughBall/`. Its gameplay and multiplayer behavior need playtesting; all participants should use the same version and setting.

`examples/Family-Bopl-Night.canna.json` is an importable empty starter pack. It contains no fictional mods. Open it with **Import modpack…**, then edit it once your repository has real mods.

**Use this repository** switches to an imported pack's catalog when clicked. Editing preserves existing version pins even when a newer catalog version exists; toggle a mod off/on to choose the current catalog version. Saved pack details identify catalog mismatches rather than silently updating pinned versions. Missing mods remain in the pack until removed.

## Diagnostics and development

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -- --scan
cargo run -- --modpacks
cargo run -- --new-pack
# Optional live read-only GitHub transport check:
cargo test github_public_read -- --ignored
```

Override detection with the Steam location field or `CANNA_STEAM_PATH`. Point it at the folder containing `steamapps`, not the `steamapps` folder itself. Background scans keep the UI responsive; a manual rescan updates loader status after external changes.

For a rendered UI smoke test, set `CANNA_SCREENSHOT` to an absolute PNG path before launching. Canna captures its own rendered window after the scan, saves that image, then exits. Normal launches do not capture screenshots.

GitHub Contents API reference: https://docs.github.com/en/rest/repos/contents
Native UI framework: https://docs.rs/eframe/0.33.3/eframe/


Console shows live Canna setup/launch messages, BepInEx output, Unity player logs and preloader errors. Select a game and log source, filter or copy output, or open its log folder. Launch status tracks the actual game process and fresh BepInEx startup output, then updates when the game closes.

## Automatic updates and mod switches

Canna 0.2.3 adds and enables required catalog libraries through Discover and mod switches. A library cannot be disabled while an enabled mod requires it. The Family Workshop example includes the 20 requested Thunderstore mods plus AntiMatchmaking. Gameplay-changing additions are opt-in; AcidTrip remains disabled and carries its flashing-color warning. Both PCs should use matching gameplay selections. FourthAbility includes its upstream stable repair plugin; keep the game's ability-count setting unchanged.

CustomLocalColorsRedux includes Canna Shared Colors: press F8 to select your lobby color. ArrowTrajectories includes Canna Friends Trajectories: press F9 to configure own, teammate and opponent lines separately. Other players' lines default off. Prediction follows native charged arrow physics; future black holes, moving terrain, portals and additional ArrowWall fan projectiles are not predicted. Second-PC multiplayer verification remains pending.

Canna checks the latest stable release at `https://cannamods.vip/updates/latest` at startup. Windows executables are served by Canna, bounded by size and checked against the published SHA-256 digest. No shared repository credential is embedded. A hidden helper waits for Canna to exit, retains the previous executable, replaces it and restarts. Open pack/group editors, settings and active installations defer the restart. Update failures leave the current app usable; the Console records check errors, and replacement logs are under `%LOCALAPPDATA%/CannaModManager/updates`.

This first updater-enabled version must be installed manually once. Future releases use a stable `vMAJOR.MINOR.PATCH` tag and an executable asset named `Canna-Mod-Manager.exe`. `scripts/Publish-AppRelease.ps1` publishes explicitly selected source files and a draft release, verifies uploaded hashes, then makes the release available. The admin token is used only by this publishing script. The publisher also mirrors release metadata and the executable to Canna’s server. Older desktop versions contain a shared read token; replace them with version 0.2.15 or later. Mod access uses the Canna account session.

Each modpack's Content table has an **Enabled** checkbox; right-click a mod for **Enable mod** / **Disable mod**. Switches save immediately without removing the selection or version pin. Use **Apply modpack** or **Launch modded** with the game closed to apply them. Disabled mods are excluded from downloads and from the fresh managed plugin directory, and exports/imports retain the enabled states. Existing manifests default to enabled. Plugins installed outside Canna's managed directory are unaffected.

The catalog includes **Canna Procedural Maps 1.1.1**. Add it to a Bopl pack on every family member's PC. It uses Bopl's shared online round seed for native scene selection, varied layouts, fixed simulation movement and satellite spin, with Steam lobby version checks. Rounds have one to nine islands, including large continents and twins. About one third load actual native space scenes with moons and optional rotating satellite panels. Native scene data and generated layouts are saved under `BepInEx/config/CannaMaps`. The 124-scene native audit and 10,000 deterministic seed checks passed; family multiplayer gameplay still needs a two-PC test.

**Canna Anvil 1.0.6** uses a native flat-sided box hull fitted to its artwork, with low bounce and settling friction. Native drop tests from upright, upside-down and sideways orientations settle on flat faces. The hull approximates the outline rather than tracing the narrow waist. Momentum, unlocked rotation, team-colored HUD circles and the five-second transformation remain.



### Minecraft work in progress

The debug desktop includes Minecraft instances, Microsoft device sign-in, managed Java, loader installation paths for Vanilla/Fabric/Forge/NeoForge/Quilt, process stop controls, and local skin import/export/account application. Microsoft sign-in requires Canna's registered public client ID. Login and live Minecraft launches have not been verified yet; content dependency installation, Minecraft modpack sharing, and complete launcher parity are still pending. The Windows releases include these preview features. Microsoft sign-in and live Minecraft launches still require verification with the registered Canna client ID.

### Downloads (0.2.6)

The sidebar uses the supplied Minecraft, website and download icons. Downloads is a full page with All, Unassigned and one tab per Steam modpack. It includes downloaded mods and BepInEx archives, plus imported local mod files. Pack tabs use stable pack IDs and matching content hashes so renamed packs and imported bundles stay grouped correctly. Files from the old website-downloads directory remain available. Select a pack tab to import a local DLL/ZIP, or add a compatible downloaded file to another pack. Frameworks are tracked separately from mods.

### Account connection and skin discovery (0.2.7)

Settings → Sign in & connect account starts a five-minute browser approval request. Sign into the Canna website, compare the six-character code with the desktop, and click Approve Canna. The waiting desktop connects and syncs automatically; this flow does not depend on Windows opening a custom protocol link. Connection proofs stay out of browser URLs, approval requires a verified active account, and claims are single-use. Cancel stops desktop polling. The legacy website download links remain supported.

Skins → Discover skins searches MinecraftSkins.net and SkinsMC together or individually, displays PNG previews, preserves attribution and original links, and saves skins for classic/slim preview, export or application to a signed-in Minecraft account. Skindex is also attempted; sites that block automated requests show a source status and a browser search link. Skin responses have size, dimensions, format and redirect-host checks.

For Microsoft login, register Canna in Microsoft Entra → App registrations → New registration with Personal Microsoft accounts. Under Authentication, enable Allow public client flows. Copy the public Application (client) ID from Overview into Canna → Minecraft → Microsoft account and Save client ID. Do not create a client secret. Device-code login does not require a redirect URI. A registered client ID alone does not guarantee Minecraft API access; any required Minecraft/Xbox application approval must also be completed. Real Microsoft login remains pending the ID and a test with a Minecraft-owning account.

Official registration guide: https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-register-app
Public client configuration: https://learn.microsoft.com/en-us/entra/identity-platform/scenario-desktop-app-configuration

### Microsoft sign-in popup (0.2.8)

Minecraft → Microsoft account → Sign in with Microsoft opens a separate popup and automatically opens Microsoft's verification page once the device code arrives. The popup offers a large code, Copy code, countdown, reopen-page button and cancellation. Closing or cancelling prevents the login from being saved. Canna's registered public client ID is the default; saved overrides are ignored as of 0.2.9.

Errors identify the failing Microsoft, Xbox, Minecraft API login, entitlement or profile step. A Minecraft API login 403 explains the possible application approval requirement. Successful device-code issuance alone does not verify a full Minecraft login. Tokens and raw authentication response bodies are not included in displayed errors.

### Public help and FAQ

The public guide at https://cannamods.vip/help is linked prominently from both the login page and member header. It describes Canna, setup, current features, preview/planned work, and troubleshooting. It contains no private member or catalog data. When adding or changing a user-facing feature, update server/web/help.html and its relevant FAQs in the same task; AGENTS.md records this maintenance requirement. Label verification limits accurately, especially Minecraft API approval, launches and multiplayer.

### Discover and navigation (0.2.9)

Discover opens a game browser. Selecting a game shows its server catalog; back or pressing Discover while already on that page returns home. Switching to other pages retains the game selection. The private catalog requires a separate desktop connection approved on the website; the empty state offers that connection directly. Minecraft content is included using a reserved game ID and filtered by Mods, Shaders, Resource packs and Data packs. Minecraft downloads still use the website download flow and require manual placement in the instance folder.

Minecraft is first in the game library, with Create instance. Its Microsoft application ID is compiled into the app and cannot be overridden by settings or client-id.txt. Full Minecraft account login/launch remains unverified while API approval is pending.

Sidebar order is Library, Modpacks, Discover, Console, Minecraft, Skins, Downloads, Settings, with a larger Website button at the bottom. Sidebar/status surfaces have square corners. Header icons show warnings/errors and the status bar uses larger text.

Skins opens friendly, original Canna robot designs. Home and external searches load additional results as the scroll reaches the end. External results are deduplicated; repeated/end pages stop loading. The title filter is not image moderation and cannot guarantee external search results are safe. Home does not fetch unreviewed external uploads.

Account connection uses an authenticated dedicated `/connect` page. The desktop copies an independent six-character code to the clipboard; typing or pasting it verifies automatically. Codes expire after five minutes and five wrong submissions permanently lock that request with a `pairing-bruteforce-blocked` audit event. Client and account rate limits restrict repeated guesses. Codes are hashed in the encrypted database and excluded from page URLs; native proof and single-use session claiming remain required. Legacy code-less account ticket creation is disabled.

Manage sessions under **My profile → Logged-in devices** on the website, or **Manage logged-in devices** in desktop Settings. Device names can be renamed; individual logout and logout-every-other-device are supported. The desktop checks access once a minute and clears a remotely revoked session. Active recently is based on server requests in the past two minutes; idle devices stay signed in.

The website library has a prominent **Add mod from external site** button; the same button in desktop Discover opens that importer directly. Paste a provider project URL, choose a version and import into encrypted server storage and the mods database. Imports remain pending until reviewed. Dependency references are displayed for separate import; CurseForge needs its server-only API credential.


## Open source and authentication

Original Canna code is available under the MIT license; see LICENSE and SECURITY.md. Public source does not make the community public: the server checks every private request, device session and role. Each device has its own session. Logging out one device leaves other devices signed in; logging out other devices keeps the current one signed in. Password reset revokes existing sessions. A revoked device must sign in again.

The desktop and server source can be reviewed without production keys. To run your own server, generate your own protected credentials and use your own domain. Do not copy production databases, backups, private content or user sessions into a fork. Game artwork, logos, third-party packages and dependencies retain their own rights and licenses.
