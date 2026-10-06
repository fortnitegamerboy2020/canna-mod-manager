# Canna Auto-Hop (L4D2) - 0.2.0 preview

Add Canna Auto-Hop to a Left 4 Dead 2 modpack and choose **Launch modded** in
Canna. Host a local game and hold your bound jump button. It starts automatically
on each map without console commands, keybind changes or enabling sv_cheats.

A native Windows x86 Valve server plugin creates a logic_script entity that
starts the included, readable movement logic. There are no binary patches or
process injection. The plugin requires -insecure and rejects -dedicated; the
movement logic additionally requires a local listen-server host. It applies a
vertical hop impulse on landing while preserving horizontal velocity. Human
survivors, including friends, are eligible; bots, infected players, pinned or
incapacitated survivors, ladders and deep water are excluded.

Automatic local activation and hold-to-hop were verified in L4D2 with sv_cheats
remaining 0. Physical controller input and movement prediction with a second
player remain unverified. Guests do not need this package installed; the host
controls the server movement. This preview is not a speedrun ruleset certification.

Canna manages the VPK, DLL and registration together, removes them for vanilla
launch, and preserves unrelated addons. Changes require the game to be closed.
Removing or unloading the plugin during a running map does not remove movement
logic already running in that map; restart the game to disable the whole addon.

Original Canna code is MIT licensed. ABI declarations match Valve's public
L4D2 SDK interfaces (IServerPluginCallbacks002 and VSERVERTOOLS001):
https://github.com/alliedmodders/hl2sdk/tree/l4d2/public
