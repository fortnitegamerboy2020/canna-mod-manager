# Canna website help maintenance

When changing a user-facing desktop or website feature, update the relevant descriptions, setup steps and FAQs in `server/web/help.html` in the same task. The user explicitly requested that the public guide stays current as features are added. Update its version labels when those versions are published.

Label shipped functionality, previews and future plans accurately. A build, endpoint check or fixture test must not be described as verified full Minecraft login, launch or multiplayer behavior. Review the Minecraft API approval status before claiming account connection works.

Keep Help / FAQ easy to find from both `server/web/login.html` and `server/web/index.html`. `/help` is intentionally public documentation; never expose private posts, profiles, catalog files, credentials or member-only application scripts through it. Preserve authentication checks on community content.
