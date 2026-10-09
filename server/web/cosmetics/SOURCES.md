# Canna profile cosmetics

The 16 frames and three abstract banners are original Canna vector artwork.

The classic MW2 collection contains 398 unique calling-card images from
https://callofduty.fandom.com/wiki/Calling_Cards/Call_of_Duty:_Modern_Warfare_2.
Lazy-loaded image URLs were resolved to their unscaled originals with
`format=original`. The original inputs were 188×40 (312 images), 240×48 (60),
358×72 (24), and 358×71 (2), including weapon and unused variants on that page.
This is not a claim of every regional or hidden game variant.

Community 0.3.64 cleans the baked screenshot surround in 260 of the 188×40
JPEGs, including Stuck on You, High Command, Joint Ops and Blunt Trauma.
Reviewed boundary-color masks preserve the central card body and the gray
metal/lens bodies where applicable. Only alpha is changed; retained RGB values
match the decoded source exactly. The three weed cards also remove isolated
neutral matte debris outside their protected title region. Fully transparent
padding is then cropped and the result stored as lossless PNG, without resizing.
Each derivative records its original source SHA256/dimensions, exact crop box,
mask class and protected body rectangle. Stuck on You becomes 181×38 after a
transparent-only crop; 3,201 of its 7,520 source pixels become transparent.

Fifty-two JPEGs with real or ambiguous gray smoke, sky, portrait or patterned
artwork remain byte-for-byte unchanged, avoiding damaged art or invented cutouts.
The 86 already transparent PNGs remain unchanged. Conservative cleanup can retain
one-pixel JPEG fringes or similar gray shades in legitimate art; it does not claim
to remove every gray pixel. Native detail remains limited by the source. No
upscale, generated replacement, HD-original or native game-texture claim is made.
MW2 artwork is displayed at at most twice its resulting pixel width and shrinks
on narrow screens; BO2 animations and Canna banners retain their existing layout.

All 296 previously issued MW2 cosmetic IDs remain unchanged. Their old import
provenance is retained under `legacy_source`: the pinned
https://github.com/IcyStarFrost/mw2-callcards-remastered commit
204c7705bccf6bedefaf2b72690b5f80ffb260d9, original Git blob and previous SHA256.
Twenty-seven former 192-pixel thumbnails now use larger true source originals.
Each current catalog item records its original source URL, downloaded asset URL,
current dimensions and output SHA256; cleaned derivatives additionally record the
original source SHA256 and dimensions. Three superseded PNG source files are retained
locally without an asset route; their replacement JPEGs use the same cosmetic IDs.

The BO2 collection comes from the user-supplied `bo2_calling_cards.rar`, containing
Volkz Calling Cards Pack. Archive SHA256:
`25207506de5bddc136f04fc3ce753d633e3ba31325590f8bd8708b63ce198f2b`.
This is custom BO2 replacement artwork, including anime designs and flag slots;
it is not represented as official BO2 originals or the complete BO2 collection.
The 224 static DDS textures were decoded to lossless PNG without resizing: 223
are 256×64 and one is 420×100. The 13 native vertical DDS animation sheets contain
32 frames (12 sheets) or 16 frames (one), each frame 256×64. They were encoded as
lossless animated WebP, preserving displayed pixels and alpha, with an authenticated
first-frame PNG poster for reduced-motion display. Duplicate adjacent frames may
coalesce; `frame_count` describes the encoded animation and `source_frame_count`
describes the original texture. Every source frame's displayed pixels and the
complete loop timeline were checked against the resulting WebP.

The archive supplies no frame-timing metadata. Playback uses a documented browser
display convention of 100 milliseconds per native source frame; it is not claimed
to reproduce the game's original animation speed. Catalog entries retain the
source archive/entry names, source DDS hash, output/poster hashes, dimensions,
animation metadata and conversion description. Original portrait/square GIFs and
texture atlases remain in the isolated audit folder, not in the cosmetic catalog.
IWI counterparts, configuration files and executables were not imported.

Original MW2 game artwork belongs to Infinity Ward/Activision. BO2 game artwork
and the pack's individual custom art retain their original owners' rights.
Source availability and attribution do not assert a blanket license. No external
messages were sent. Cosmetic and poster routes require the existing authenticated
asset API; posters are not separate crate prizes.

## Community 0.3.65 availability

MW2 calling cards are paused due to insufficient source image quality. Their
files, provenance, IDs, odds weights, ownership and stored selections are retained
for a future resume. The MW2 crate and new equips are disabled, mixed-case drops
exclude them, and the collection/profile UI hides them. BO2 and avatar frames
remain active. No asset pixels changed in this pause.

## Local preview: user-supplied emblem packs and username effects

`mw2-callcards-remastered-main.zip` (SHA256 `0af2c3dfa473e652760c159892376be4750a1df3adbea2f599433bfcc11d7337`) supplies 205 PNG emblems. Its 296 small calling-card images do not resolve the previously reported quality problem; MW2 calling cards remain paused. `Call-of-Duty-Rank-Emblems-main.zip` (SHA256 `1572137cd5209ca966008a2195507afb9d62e302e476a00f91c9937249bf8a76`) supplies 123 PNG rank/prestige emblems across its named games. Original image bytes and transparency are preserved; duplicate DDS versions and Backups, Lua addon code, fonts, sounds and VTF files are not imported. Each item records its archive member path and exact image digest. Game artwork remains owned by its respective creators. Six gradient preview SVGs and matching CSS animations are original Canna artwork. Run `scripts/Import-CosmeticPacks.py` to regenerate asset routing idempotently from the supplied archives and catalog.
