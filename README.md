# Canna Mod Manager

A native Rust desktop mod library for you and your family. Dark forest colors, Steam library discovery, Bopl Battle as the first supported game, and a private GitHub repository as the read-only catalog. There is no upload interface.

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
- Reads game metadata, icons and mod listings from GitHub on a background thread. Reports authentication, missing repository, rate limit, malformed metadata and network errors.

Creating a modpack automatically sets up BepInEx. **Add Mods** selects published catalog mods; **Import local mod** adds a DLL or plugin ZIP. **Install Mods** activates the selection in `BepInEx/plugins/Canna`. **Launch modded** installs the selected pack and launches through Steam. **Launch vanilla** disables Doorstop before launching. The last mode remains selected until changed. Existing plugins outside Canna are preserved and also load in modded mode.

Bopl Battle uses the official Windows x64 BepInEx 5.4.23.5 archive from `bopl-battle/Framework/BepInEx.zip`, with the official GitHub release as a fallback. Harmony is included upstream; this is not a custom BepInEx fork. Other Unity games need their own compatible Windows BepInEx 5 package at `<game>/Framework/BepInEx.zip`. Existing complete loaders are preserved, and conflicting files are reported rather than overwritten. Close the game before setup, installation or mode changes. Gameplay compatibility still requires testing.

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

Open **Repository settings** in Canna. Enter the owner, repository and branch. For a private repository, use a GitHub fine-grained personal access token restricted to that repository with **Contents: Read-only**. Family builds embed the repository-only read token from the Git-ignored canna-token.txt file at build time; family members do not need GitHub accounts. Embedded tokens are extractable from the executable. The token field is masked and never saved to settings or logs. It lasts for the app session. Alternatively, provide `CANNA_GITHUB_TOKEN` in the launch environment. Never put tokens in the mod repository.

Suggested values: repository **manager-uploaded-mods**, branch **main**, catalog folder **empty (repository root)**, and game/mod folder **bopl-battle/Mods/**. Alternatively, put the template contents under `games/` and set the catalog folder to `games`; Canna then reads `games/catalog.json`, `games/bopl-battle/game.json` and `games/bopl-battle/icon.jpg`. The folder is always relative to the repository. Existing settings files default to the repository root.

Click **Save, scan & connect**. The app requests `catalog.json`, then each listed folder's `game.json` and icon using GitHub's authenticated Contents API. It only sends GET requests. If you have not made the repository yet, Steam scanning still works.

If `catalog.json` is absent, Canna discovers repository folders that contain `game.json`. A missing `Mods/` folder is allowed and shown in the game's status. To add an otherwise empty directory through GitHub's web uploader, include the visible `Mods/README.txt` supplied by the template; it is not a mod and is not added to the mod list. Actual mods appear only when listed in `game.json`.

Successful catalog syncs save the most recent repository's metadata and artwork to `%LOCALAPPDATA%/CannaModManager/catalog-cache.json`. On the next connection, Canna shows matching cached data while contacting GitHub. If refresh fails, the catalog stays visible with an offline label and its age. The owner, repository, branch and catalog folder must match; changing repositories clears the previous repository's artwork. Tokens are never included in the cache. The cache contains private catalog content as ordinary local files. Up to 16 MiB of artwork is cached; remaining games can fall back to Steam artwork.

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

Saved packs can be edited, duplicated and searched. **Remove** removes a selection; click **Install Mods** to apply that change. **Export** writes a `.canna.zip` bundle containing the manifest and any imported local mod files. Repository mods remain version/checksum-pinned references. **Import** supports both `.canna.zip` and older `.canna.json` manifests, validates paths and checksums, and assigns a fresh local pack ID. Exports contain no tokens or local game paths. After importing, use **Install Mods** or **Launch modded**.

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

## Canna 0.2.0: updates and mod switches

Canna checks the latest stable release in the private `fortnitegamerboy2020/canna-mod-manager` repository at startup. New Windows executables are downloaded using the separate release read token, bounded by size and checked against GitHub's SHA-256 asset digest. A hidden helper waits for Canna to exit, retains the previous executable, replaces it and restarts. Open pack/group editors, settings and active installations defer the restart. Update failures leave the current app usable; the Console records check errors, and replacement logs are under `%LOCALAPPDATA%/CannaModManager/updates`.

This first updater-enabled version must be installed manually once. Future releases use a stable `vMAJOR.MINOR.PATCH` tag and an executable asset named `Canna-Mod-Manager.exe`. `scripts/Publish-AppRelease.ps1` publishes explicitly selected source files and a draft release, verifies uploaded hashes, then makes the release available. The admin token is used only by this publishing script. `canna-token.txt` embeds the original mods read credential; `canna-update-token.txt` embeds the separate application releases read credential. Both are excluded from Git. Environment overrides are `CANNA_GITHUB_TOKEN` and `CANNA_UPDATE_TOKEN` respectively.

Each modpack's Content table has an **Enabled** checkbox; right-click a mod for **Enable mod** / **Disable mod**. Switches save immediately without removing the selection or version pin. Use **Install Mods** or **Launch modded** with the game closed to apply them. Disabled mods are excluded from downloads and from the fresh managed plugin directory, and exports/imports retain the enabled states. Existing manifests default to enabled. Plugins installed outside Canna's managed directory are unaffected.

The catalog includes **Canna Procedural Maps 1.0.0**. Add it to a Bopl pack on every family member's PC. It uses Bopl's shared online round seed, integer layout generation, fixed simulation ticks and Steam lobby member version checks. Generated rounds use six to nine islands, four stable spawn islands, moving upper islands and occasional native space gravity. Native scene data and generated layouts are saved under `BepInEx/config/CannaMaps`. Family multiplayer gameplay still needs a two-PC test; matching layout fingerprints alone do not prove full-game synchronization.
