# Canna family mod catalog

Create this as a private GitHub repository named `manager-uploaded-mods`. Copy this template's contents to the repository root on the `main` branch.

```text
catalog.json
bopl-battle/
  game.json
  icon.jpg
  Mods/
    README.txt
    your-actual-mod.zip
```

`catalog.json` lists game folder names. Each game's `game.json` contains its Steam app ID, name, description, icon filename and mod listings. Upload actual mod files to that game's `Mods/` directory, then add their metadata to its `mods` array. The JSON key `mods` stays lowercase; the directory `Mods` uses a capital M.

Example mod entry (replace with a real mod):

```json
{
  "name": "Your mod",
  "version": "1.0.0",
  "description": "What the mod does",
  "file": "Mods/your-actual-mod.zip",
  "sha256": ""
}
```

Use unique mod file paths. Leave `sha256` empty or set it to the actual file's 64-character SHA-256 checksum. The template has no sample mod binaries.

Keep the visible `Mods/README.txt` so the folder can be uploaded through GitHub's website. It is not counted as a mod. Canna also supports a missing Mods folder, and can discover game folders when catalog.json has not been uploaded yet.

In Canna, set owner to your GitHub username, repository to `manager-uploaded-mods`, branch to `main`, and leave the catalog folder blank. Use a fine-grained GitHub token limited to this repository with Contents read access. Never commit tokens to this repository. Each family member needs access to the private repository.

To add another supported Unity game, create its folder with the same structure and add its folder name to `catalog.json`. Keep the Steam app ID unique.
