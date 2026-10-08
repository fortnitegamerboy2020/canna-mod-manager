# Canna workflow verification

This is a regression and fault-testing matrix, not an aviation certification or a
claim that every possible user action has been proved correct. Unknown/live results
remain explicit. Checks use isolated directories/databases and mock provider responses;
they do not sign in users, wager live Kash, approve real uploads or launch real games.

## Reproduce

Install no dependencies automatically. Use an existing Rust/Windows toolchain, Node,
Python and Playwright with Edge available. From the repository:

```powershell
./scripts/Test-Workflows.ps1 -Node node -Python python -PlaywrightModule '<existing Playwright module>'
```

`-Packaging` also tests the built `dist` artifacts with the existing Inno Setup compiler.
`-Server user@host -Identity <key>` runs Rust tests/lint against the synchronized server
checkout in isolated SQLCipher fixtures; it does not deploy. The server requires its
SQLCipher development libraries. A constrained production server may need its review
worker paused during compilation. Always resume it, including after failures.
Use `CARGO_TARGET_DIR` for a separate build tree if a running EXE locks the default tree.

The runner stops on the first failure and writes a local summary only on success.
Live-provider/game tests marked ignored in Rust stay excluded. Run them explicitly
only after reviewing their comments and scope.

## Coverage

| User workflow | Automated evidence | Failure/privacy checks |
|---|---|---|
| Navigate forums, discussions, members, admin and Play Lab | Test-CommunityNavigation.cjs, Test-ModBrowserPage.cjs | Startup JS, back routing, hidden panels, desktop/390px layouts |
| Browse/filter/page providers, pick a release and subscribe | provider_browser tests; Test-ProviderBrowser, Test-ModBrowserPage, Test-CurseForge | Metadata-only browsing, source attribution, provider failure isolation, version/loader mismatch, account switch, review gating |
| Create/edit/import/export/duplicate/remove a pack | modpacks and pack_ui tests | Archive corruption, path traversal, pin preservation, dependency enable/removal, target isolation, undo |
| Discover installed or uninstalled games | Steam, profile and native UI tests | Secondary libraries, invalid VDF, missing Source content, branch/build vs release numbers, disabled launch |
| Apply/switch Unity and Source content | runtime/source_addons tests | Preserve unrelated plugins/configs, DLL digest/architecture, managed registration changes, interrupted activation rollback, unsafe ZIP/VPK |
| Install managed Minecraft content | native_content_tests and minecraft::play tests | Recursive dependencies, digest/loader/version rejection before writes, instance confinement, exact JAR restore, configs copied, worlds untouched |
| Create/join/update/leave/close a readiness room | play endpoint tests and Test-PlayLab.cjs | Authentication plus invitation capability, wrong/replayed access, membership, stale readiness, mismatch/unknown data, no account/device IDs |
| Save/select/restore/delete a local recovery point | play_backup tests | Exact archived bytes, corrupt archive refusal, traversal, staging cleanup, budget, game version/instance check, working retention, world exclusion |
| Edit local game configs | play_config tests | Game closed, stale edit refusal, external path rejection, invalid JSON, previous file preserved |
| Diagnose and create dependency-closed test halves | play_manifest/play_lab tests | All supported profiles, original untouched, missing dependencies, unknown result preserved, personal-data redaction, dependency closure bounded |
| Preview/share/remove a report or send private help | play endpoint tests, Test-PlayLab.cjs | Unknown fields rejected, raw logs excluded, preview invalidation, private ownership, deduplication, 30-day expiry, anonymous reader payload |
| Publish/promote/read/remove pack releases | play endpoint tests and browser tests | Owner-only edits, experimental default, stable requires exact successful observation, missing/unknown setup blocks promotion |
| Thousands of list entries | lists tests; play 1,001-report fixture | 50-item pages, search beyond legacy caps, compact summaries, on-demand full manifests |
| Local performance tools | play_metrics tests; native checks | Valid retained process handle, current/peak memory consistency, no measurement upload; synthetic checks not FPS |
| Login/devices/invites/password/support | server auth/device/handoff/email/support tests; browser device/webconnection/proof fixtures | Revocation, expiry, rate/guess limits, same-origin cookies, role/ownership boundaries, guest ticket traps/proof |
| Chat, Kash, notifications and submissions | cannabot/lounge/notifications tests; browser scripts | Independent random draws may repeat, atomic payouts, rate limits, privacy, 24-hour chat retention, clear-all, decisions/reasons |
| Review/publish/reject/import/update mods | scans/external/subscriptions/update tests; Test-ReviewWorker.py and moderation scripts | Unsafe archives, packer heuristics vs certainty, manual upload approval, dependency cycles, previous approved revision retained |
| Contextual review (server 0.3.57) | deploy/test-review-context.py, test-review-packing.py, test-review-adversarial.py, test-review-coverage.py; Test-ModReview.cjs --browser | Comments/literals and UI Open false positives, raw-string and shell bypasses, unknown/sensitive paths retained, grouped diagnostic and coverage evidence, changed omissions invalidate decisions, filename disguises, engine errors, caps/truncated output, malware signatures preserved; production-identity handoff verified without mod approval |
| Import Thunderstore ROUNDS shared dependencies | external::tests with a public 33-project metadata fixture | All five reported mods resolve current recursive requirements once; selected root pins stay exact, modpack/Minecraft exact conflicts, missing files, cross-game content and cycles are refused; official-index fallback preserves source identity, exact release history and availability |
| Application update/install/uninstall | updater tests, Test-Maintenance.ps1, Test-Installer.ps1 | Hash checks, previous copy recovery, replacement failure, shortcuts, fixture-only full uninstall, owned external backup removal and unowned-folder preservation |
| Public help vs private community | server page/access tests | Public guide contains no member content; private APIs/scripts denied without sessions |

## Issue-driven fault cases

User reports informed test selection, not verified root-cause claims:
- [Wrong mod/game directory](https://www.reddit.com/r/lethalcompany/comments/189ba7l): validate install roots and surface game/version state.
- [Friends cannot play together with mods](https://www.reddit.com/r/riskofrain/comments/177jdug/friends_suddenly_cant_play_with_me_modded_pc/): compare exact pins/config digests and keep unknown compatibility explicit.
- [Modpack update/world trouble](https://www.reddit.com/r/feedthebeast/comments/1v2lfr7/never_update_your_modpack_mods_without_a_backup/): snapshot exact content, refuse bad hashes, never implicitly downgrade worlds.
- [Launcher closes while loading](https://www.reddit.com/r/Modrinthlauncher/comments/1vojns5/modrinth_launcher_crashingclosing_when_loading/): keep local diagnostics and explain memory/version/lock patterns without claiming a confirmed cause.

## Required live checks and limits

Every supported title still needs a real install/apply/start/stop/vanilla/restore pass,
with observed logs and mod-specific functionality. Add multiplayer host/client trials,
physical controller tests where applicable, and at least one limited-memory PC. Repeat
on Steam public/previous/beta builds when supported. The registry fixture test checks
profiles and shared behavior; it is not an in-game result for each title.

Minecraft Microsoft/API approval and full live launcher authentication remain external
prerequisites. Existing L4D2 Auto-Hop hold-jump activation was user-confirmed previously;
this release does not extend that result to every controller, friend or game build.
Power loss/drive unplug and forced termination during each file-swap phase need dedicated
live fault trials; retained staging/previous folders require inspection, not blind deletion.
The automated corruption/path/activation tests do not simulate every Windows filesystem
or antivirus behavior. No user-count-derived or FPS performance claims are made.

Local backups are configurable and bounded. This owner's drive policy is local-only,
not a public default. Server recovery copies have a 10 GiB limit and preserve 5 GiB free;
new backups stop rather than deleting the existing copy when limits are exceeded.
Play Lab stores bounded metadata only on the server, with hourly room/report expiry.
