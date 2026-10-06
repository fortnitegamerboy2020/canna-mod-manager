# Canna Auto-Hop (L4D2) - 0.1.0 preview

Hold your bound jump action to repeat hops when landing. The script reads L4D2's
jump action, shared by keyboard, controller and Steam Input mappings. It applies
a vertical hop impulse while preserving horizontal velocity.

Install through a Canna L4D2 modpack and choose **Launch modded** (`-insecure`).
Host a local map. In the host developer console, enter `sv_cheats 1`, then
`script_execute canna_autohop`. Repeat the script command after map changes.
This enables local practice cheats, so use a local practice session only.
The script does not enable cheats or change keybinds automatically.

The host script processes all human survivors, including friends joining the
local server. Bots, infected players, pinned or incapacitated survivors, ladders
and deep water are excluded. Guests do not need the script to receive server
movement changes. Dedicated servers and joining a remote host are unsupported.

Run `script CannaAutoHop.enabled = false` on the host to disable auto-hop; set it
to `true` to resume. Reloading replaces only this mod's own tick handler.

This original Canna code is MIT licensed. Repeated hops were tested in a running
local L4D2 map with VAC disabled, and the tester confirmed holding jump works.
Physical controller input and multiplayer prediction with a second player have
not been verified. This is a preview, not a speedrun ruleset certification.
