# Canna profile cosmetics

The 16 frames and three abstract banners are original Canna vector artwork.

The classic MW2 collection contains 398 unique calling-card images from
https://callofduty.fandom.com/wiki/Calling_Cards/Call_of_Duty:_Modern_Warfare_2.
Lazy-loaded image URLs were resolved to their unscaled originals with
`format=original`, preserving the source PNG/JPEG bytes. Native dimensions are
188×40 (312 images), 240×48 (60), 358×72 (24), and 358×71 (2). This includes
weapon and unused variants shown on that page, not a claim of every regional or
hidden game variant. No generated upscale or HD-original claim is made. The
weed-themed High Command, Joint Ops and Blunt Trauma remain native 188×40 art.

All 296 previously issued MW2 cosmetic IDs remain unchanged. Their old import
provenance is retained under `legacy_source`: the pinned
https://github.com/IcyStarFrost/mw2-callcards-remastered commit
204c7705bccf6bedefaf2b72690b5f80ffb260d9, original Git blob and previous SHA256.
Twenty-seven former 192-pixel thumbnails now use larger true source originals.
Each current catalog item records its original source URL, downloaded asset URL,
native dimensions and local SHA256. Three superseded PNG source files are retained
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
