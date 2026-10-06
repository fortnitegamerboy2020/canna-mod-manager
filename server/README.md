# Canna family server

Rust/Axum service for invitation-only accounts, ZIP uploads, and authenticated
modpack share links, plus a small browser portal. Desktop account and server
catalog integration are still pending; the existing GitHub catalog is retained.

## Host

- Ubuntu 24.04 LTS, public address `165.227.83.76`.
- Key-based administrator: `canna-admin`, with sudo access.
- Application service identity: `canna` (no interactive login).
- Application binaries: `/opt/canna`.
- Configuration: `/etc/canna`.
- Account database and application state: `/var/lib/canna`.
- Uploaded files: `/mnt/canna/uploads`.
- Modpack files: `/mnt/canna/modpacks`.
- Existing 40 GB ext4 volume is mounted by `mnt-canna.mount`; do not format it.
- UFW permits only SSH (22), HTTP (80), and HTTPS (443) inbound.

The administrator private key stays outside this repository at
`C:\Users\t_tra\.ssh\canna_server_ed25519`. Never commit or upload it.

```powershell
ssh -i C:\Users\t_tra\.ssh\canna_server_ed25519 canna-admin@165.227.83.76
```

## DNS

In the registrar's DNS settings, use A records `@` and `api`, both pointing to
`165.227.83.76`. Remove conflicting parking/redirect records at those hosts.
Do not publish the private VPC address in public DNS.

`deploy/Caddyfile` enables automatic HTTPS once DNS resolves to this host and
proxies to the Rust service on loopback port 8787. Shared modpack links require
a family account. Uploaded archives are never extracted or executed on the
server, and downloads require authentication.

## Accounts and uploads

The first administrator invitation is stored inside the encrypted database.
Use the protected invitation helper in Downloads to copy it into the portal.
Choose your own password; it is hashed with Argon2id. The invite works once.
The Owner can create waves of 1–50 seven-day, one-use invitations in the
portal and revoke unused codes from a wave. Every new member receives one
friend invitation; issuing it consumes the allowance transactionally. Invited
friends also receive one invitation. There is no open registration. At most
200 unused invitations can be pending. Invite codes are stored as hashes.
Session tokens persist until sign-out or revocation. Password sign-in requires a one-use email code (ten-minute expiry, five guesses, bound to the requesting browser). Choosing "Trust this device for 30 days" skips the email step on that device for a fixed 30 days; the password is still required. Password resets, bans and role changes revoke device trust. "Forget all trusted devices" also signs out every session. Enabling this upgrade invalidates previous password-only sessions. The browser uses
a Secure, HttpOnly, SameSite=Strict cookie with same-origin mutation checks.
Login/registration is globally limited to 20
attempts per minute, with at most two password computations concurrently.

Uploads are streamed to the secondary volume with a 128 MiB limit per ZIP and
a 30 GiB total storage limit. Family members can download all family uploads;
only the owner or administrator can delete them. Upload metadata and modpack
manifests are stored in SQLCipher on the primary disk. Metadata is not rendered as
HTML. Pack exports containing unbundled local files are rejected; existing
GitHub-backed exports can be shared and imported in Canna. Uploads are not yet
integrated into desktop Discover or installation.

Run `cargo test --manifest-path server/Cargo.toml` and
`cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings`.
Build on Ubuntu with `cargo build --release --locked -j 2`. Install the binary
at `/opt/canna/canna-server`, then install `deploy/canna.service` in
`/etc/systemd/system/`, run `systemctl daemon-reload`, and enable/start `canna`.
The service requires the mounted volume and runs as the unprivileged `canna`
identity with restricted filesystem writes, UMask 0077, a 512 MiB memory ceiling
and a 128-task ceiling.

## Provisioning

`deploy/bootstrap.sh` is for this existing host. It requires `/mnt/canna` to
already be mounted, retains its existing mount configuration, and does not
format disks. It installs Caddy from its official stable package repository.
It grants the `canna-admin` user sudo access and copies the root authorized
keys to that administrator. It does not disable password authentication.

Before any SSH authentication changes, confirm an independent administrator
login and sudo command succeed. Keep the DigitalOcean console available.
Backups should cover both the account database and the uploaded files; the
secondary volume is storage, not a backup.

## Encryption and email

SQLCipher encrypts the complete account database and WAL, including usernames,
emails, password hashes, invitation records, metadata and manifests. Passwords
use salted Argon2id (64 MiB, three iterations, one lane), rather than reversible
password encryption. Uploads use authenticated XChaCha20-Poly1305 STREAM records,
bound to the upload UUID; modified, reordered or truncated ciphertext is rejected.
The database and upload keys are distinct random 32-byte keys outside the data
directories, readable only by root in `/etc/canna/keys`. systemd supplies private
runtime credentials to the unprivileged service. Keys are not embedded in source.

Resend sends verification and password-reset messages from
`Canna <accounts@cannamods.vip>`. Its send-only API key is the `mail.key` systemd
credential. The domain must be verified in Resend. Registration stays unavailable
without mail configuration. Email ownership must be verified before sign-in.
Codes expire after ten minutes, work once, and allow at most five guesses.
Requests for another code have a one-minute cooldown. A password reset revokes
all existing sessions. Recovery responses do not disclose whether an email is
registered. API calls to Resend require HTTPS; email is not end-to-end encrypted,
and the provider and recipient necessarily receive the address and code.

Encryption at rest protects copied storage when the keys are not also stolen.
It does not protect against root access or compromise of a running service that
can decrypt the data. Root key storage and the encrypted data require separate,
protected backups; losing those keys makes the data unrecoverable. Automated
scans and regression tests are useful checks, not a guarantee of security.

`deploy/encrypted-upgrade.sh` is restricted to the existing pre-account database:
it refuses plaintext migration if users, mods or packs already exist. The
migration retains invitation hashes and moves the bootstrap invitation into
encrypted storage. `deploy/harden-ssh.sh` disables SSH passwords and root login
only after a separate `canna-admin` key login and sudo access have been verified.
The DigitalOcean recovery console remains available.

## Community roles and profiles

| Role | Permissions |
| --- | --- |
| Member | Download and upload mods, share packs, post discussions and replies, manage their profile, rate other members, issue their one friend invite |
| VIP | Member permissions plus publishing guides |
| Admin | Content moderation, source previews, ban/unban Members and VIPs; no invitation or role-management permissions |
| Owner | All permissions, invitation waves, role assignment, moderation audit and atomic ownership transfer |

The bootstrap invitation creates the single Owner. Admins cannot ban another
Admin or the Owner. The Owner cannot ban themselves; ownership must be
transferred first. Bans revoke sessions, recovery codes and unused invitations
issued by the banned member. Role changes require the affected member to sign
in again. Ownership transfer revokes both users' sessions and never leaves two
Owners or zero Owners. Password hashes and email addresses are not exposed in
the admin member list.

The forum supports help, discussion, showcase and guide topics, replies, linked
uploaded mods, pagination, pinning, locking and moderation. Text is rendered
with `textContent`, never executed as HTML. Source previews are restricted to
source files actually included in an uploaded ZIP; DLL-only uploads do not
provide original source. Inspection limits are 32 MiB per archive, 2,000 entries,
100 source files, 1 MiB per source file and 2 MiB total source text. Files are
read in memory and never extracted onto the server filesystem.

Profiles support a status, bio, custom picture, moderated comments, and one
editable 1–5 star vote per other verified member. Self-ratings are rejected and
banned users' votes are excluded. Pictures accept PNG/JPEG/WebP up to 2 MiB and
4,096×4,096 pixels; they are decoded, cropped/resized to 256×256, re-encoded as
PNG without the original metadata, encrypted, and served only to signed-in
members. Profile text, comments and votes are in the encrypted database.

Activity XP is five per retained forum post, five per retained topic, and one
per retained profile comment. Deleting content removes its contribution. Ranks
are Noob (0), Apprentice (25), Modder (100), Expert (250), Master (750),
Grandmaster (1,500), and Pro Hacker (3,000). These labels never grant permissions.

The desktop manager's `#` Community button opens the portal. Desktop account
sync and installing server uploads directly from Discover remain pending; the
existing GitHub catalog continues to work. The website downloads shared pack
exports for import into the desktop manager.

## Forum interface (0.3.2)

The website uses Canna's deep emerald theme and opens on the forums after sign-in.
Category links filter the current topic page; rows show replies, author and activity.
Threads use author panels, timestamps and numbered posts. New discussion opens the
composer; cancelling returns to the list. Existing search, pagination, moderation,
profile, source-preview and email-code flows are retained.

Mods and modpacks appear first in the library. Upload, sharing and invitation
controls expand when needed. The browser loads forum.css from the same origin;
no external theme scripts, fonts or forum service are used. Preview names and
counts in server/security/ui-preview.cjs are synthetic and never deployed.

### Forum sections (0.3.3)
Owners can add sections, rename them, edit descriptions, reorder them, close new discussions, and restrict posting to VIP+. Existing section IDs remain stable so renaming and closing preserve discussions. Admins cannot edit sections. Changes use a server-staged review followed by a separate confirmation; reviews expire after ten minutes, are owner-bound and single-use, and reject stale layout revisions. Applied layouts are saved atomically in the encrypted database and recorded in the owner audit log.


### Admin invitation controls (0.3.4)
Owner invitation controls appear at the top of Administration and include both single-invite and wave generation. Results and errors appear beside the controls; buttons are disabled during requests. The top-right username opens the signed-in user's profile. Admin-role accounts retain their existing no-invite policy.


### Server-gated pages (0.3.5)
Anonymous and expired sessions receive a separate login document containing no forum, profile, upload or administration markup. Community scripts require a valid verified, unbanned session. Successful authentication reloads the current route, preserving modpack links. Private documents and scripts use no-store and vary by Cookie/Authorization; API authorization remains enforced independently. Restoring a community page from browser back/forward cache triggers a server reload.


### Security hardening (0.3.11)
Authentication requests are limited per client (30/minute), password sign-in per account (20/minute), and authenticated writes per member (60/minute), with bounded global overload protection. Only a loopback proxy may supply the client IP, and Caddy overwrites forwarded IP headers. Sessions persist until revoked; device trust remains 30 days.

Uploaded-mod quotas are 2 GiB/member, 5 GiB/VIP, 10 GiB/admin and 30 GiB/owner, subject to the 30 GiB shared limit and 500 archives/account. Posts and profile comments share a 5,000/account lifetime quota. Deleting content frees quota. New archives/imports require administrator approval; the owner-staged existing catalog is retained. Pending mods are excluded from the desktop catalog and rejected by direct and ticket downloads. Approval is logged and does not certify an archive harmless.

`deploy/backup.sh` creates a root-only, consistent recovery copy of encrypted state, the secondary volume, keys and service configuration on the primary disk. It briefly pauses the app while copying. The daily timer runs at 09:00 UTC with up to five minutes jitter. Staging/hardlinks retain the last complete copy until isolated database/catalog verification succeeds. It preserves 5 GiB free disk space and fails rather than filling the primary disk. DigitalOcean Droplet backups exclude Volumes; this recovery copy must be included in a subsequently completed Droplet backup. This is not an independent off-provider backup. Provider backup completion and a full Droplet restore must be checked in DigitalOcean.

Recovery: preserve `/var/backups/canna-current` (or a complete `canna-previous` after an interrupted rename). Restore onto an isolated Droplet first; the state, volume, keys and configuration must match. Keep credentials private. Validate `--check-storage` and `--audit-catalog` with restored paths before redirecting DNS. Do not overwrite live data to test restoration.

Account connection uses an authenticated dedicated `/connect` page. The desktop copies an independent six-character code to the clipboard; typing or pasting it verifies automatically. Codes expire after five minutes and five wrong submissions permanently lock that request with a `pairing-bruteforce-blocked` audit event. Client and account rate limits restrict repeated guesses. Codes are hashed in the encrypted database and excluded from page URLs; native proof and single-use session claiming remain required. Legacy code-less account ticket creation is disabled.

Logged-in devices are private to their account, named from browser/OS or native PC name, and support rename, individual logout and logout-other-devices. Active recently means an authenticated request within 120 seconds; idle sessions remain signed in. Browser cookies use a rolling 400-day Max-Age (browser limit), refreshed by authenticated traffic; native credentials have no clock expiry. Valid legacy sessions migrate once without reviving expired ones. Revocation deletes associated email trust; password resets and bans continue to invalidate sessions.


### CurseForge credentials
Use approved CurseForge for Studios third-party project/download API keys, not author upload tokens. Keep one distinct key per line in `/etc/canna/keys/curseforge.keys` (root:root, mode 0600), then add `LoadCredential=curseforge.keys:/etc/canna/keys/curseforge.keys` to the service, daemon-reload and restart. Never embed them in desktop builds or logs. The legacy `curseforge.key` credential remains supported. Each 429 response sets a per-key cooldown using Retry-After (seconds or HTTP date), default 60 seconds. A request tries each available key at most once; all-limited pools return 429 until cooldown. Keys are not rotated on 401/403 or author restrictions. Provider account-wide limits may apply to all keys.

### Application updates
Public `/updates/latest` serves a bounded manifest. `/updates/vMAJOR.MINOR.PATCH` streams only fixed versioned executable files under `/opt/canna/releases`; it cannot read arbitrary paths. Publisher scripts mirror public executable/hash metadata with SSH. No repository token is sent to clients. Keep the private GitHub source/release archive. Desktop login credentials are user-bound DPAPI files, not shared provider keys.

CurseForge import coverage (server 0.3.16): Minecraft mc-mods, texture/resource packs, shaders, data packs and modpack archives, plus whitelisted Steam/Unity games. Available versions paginate in 50-item pages up to the provider 10,000-result limit. Loader labels are normalized to lowercase, separated from game versions; dependency relation metadata remains intact. A missing file download URL is resolved through the official file download-url endpoint at import time; no CDN URL is guessed. Distribution-disabled authors remain blocked. Modpack archives can be catalogued/downloaded but are not automatically installed as Minecraft instances. Recursive dependency installation and full Minecraft content installation are not verified. Live CurseForge preview/import awaits an approved project API key.


Server 0.3.17: Owner uploads/imports publish automatically; members and admins submit to the dedicated mod review queue. Approval traverses imported dependency IDs transactionally. Import metadata is resolved before downloads, deduplicates shared dependencies, honors required file IDs and Minecraft version/loader constraints, optionally includes optional dependencies, and rejects conflicts, cycles and graphs over 128 projects. Imported archives still follow existing Minecraft installer limitations; importing a provider modpack does not install its internal manifest. BepInEx framework dependencies are catalogued, but desktop pack dependency names omit the framework already managed by Canna.

Administration uses Overview, Mod reviews, Members and Support tickets tabs; Owners also have Forum sections, Invitations and Audit. Forum widths adapt up to 2200 px. Staff can inspect source, approve/reject mods, search members and revoke subordinate devices. Admins cannot issue invitations.

Public /support accepts help, bug, invite and general tickets from guests or members. Tickets and messages are in the SQLCipher database and backups. Guest access requires a 256-bit private tracking secret, stored only as a digest; signed-in authors and staff have server-authorized access. Replies are on-site, not an inbox or inbound email service. Hidden honeypots silently discard filled traps; a single-use IP-bound 10-minute challenge, SHA-256 proof of work (three leading hexadecimal zeroes), minimum two-second age, same-origin checks, 3 tickets/client/hour, 200 tickets/day globally and a 10,000-ticket storage cap limit abuse. Limits reduce spam; they cannot guarantee that sophisticated bots never submit. No paid CAPTCHA, email hosting or new API key is required. Resend remains send-only.

Public discovery: /robots.txt and /sitemap.xml advertise the login/about entry page and /help. Community APIs and tickets remain private. BingSiteAuth.xml is an operator-provided public verification file under /srv/canna-public served by Caddy; it is not committed to source. Indexing requires the owner to verify in webmaster tools and submit the sitemap. No ranking or indexing date is guaranteed.

## Static mod review and community additions (0.3.18)

The private review workspace at `/review/mods/<id>` is restricted to Admin/Owner accounts. `CANNA_REVIEW_JOBS` enables automatic scan gating: downloads and approval require completed analysis and recorded decisions for all findings, including dependencies. Reports and reconstructed source are stored in the encrypted database. Plaintext handoff jobs are temporary and must not be included in backups.

The separate `canna-review` worker has no access to production keys, database, uploads store or network. Install .NET 8, ILSpyCmd 9.1.0.7988, Java 17, CFR 0.152, ClamAV with freshclam, and Detect It Easy 3.21 from their official distributions. Place ILSpyCmd under `/opt/canna-review/tools/ilspycmd` and CFR under `/opt/canna-review/tools/cfr.jar`; the other tools use `/usr/bin`. Create `/var/lib/canna-review/jobs` as root:canna-review, mode 2770; install `deploy/review-worker.py` under `/opt/canna-review/` and `deploy/canna-review.service` under systemd. The main service has supplementary group canna-review and read/write access only to the job spool. Each job directory must retain mode 2770 (setgid); changing it to 0770 causes new input archives to use the API primary group and makes them unreadable to the isolated worker. Run `test-review-permissions.py` as the API user after provisioning to verify the actual archive/source/result handoff. ClamAV signature updates run separately with network access. DiE rules ship with its pinned release.

Analysis is bounded and static: no submitted code is run, native code is not reconstructed, failed or oversized analysis is explicit, and packing/obfuscation cannot always be identified or unpacked. Findings can be false positives. Test the service using the harmless `deploy/test-review-live.py` fixture before deployment; it verifies .NET decompilation, code findings, ClamAV EICAR detection, and DiE availability.

Member-only chat expires after 24 hours in active storage; backups rotate separately. CannaBot coins and cosmetic badges have no monetary value and never confer permissions. Daily rewards and game limits are enforced transactionally in the encrypted database. Category removal requires Owner review/confirmation and reassigns existing discussions rather than deleting their posts. Submission decision history expires after seven days; private notifications expire after 30 days and are capped at 500 per recipient.
