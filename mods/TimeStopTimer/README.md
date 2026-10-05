# Time Stop Timer: Canna 1.1.2

Includes the original Antimality TimeStopTimer 1.1.0 DLL and a compatibility extension.
Source: https://thunderstore.io/c/bopl-battle/p/Antimality/TimeStopTimer/
The extension replaces the original Harmony hooks, keeping a persistent overlay
that reads the game's actual casting and TimeStop duration fields.

White text has a black outline and no background box. Online players are identified
by their Steam usernames. Local players can set names in
`BepInEx/config/local.canna.timestoptimer.repair.cfg`, under Local names, Player1–Player4.
With no custom name, keyboard/mouse uses the signed-in Steam name; other local slots
use Local 2, Local 3, etc. Names are bounded and shown as literal text.
