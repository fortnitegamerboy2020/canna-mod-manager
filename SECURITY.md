# Authentication and public source

Canna can be open source while its community remains invite-only. The server authenticates every private request and checks the user's permissions. Client source code, the Microsoft application client ID, server URL and protocol are public information; knowledge of them grants no account access. There is no shared desktop API secret.

Each login creates an independent cryptographically random device session. The server stores a hash of the token; Windows desktop storage uses user-bound DPAPI encryption. Browser cookies are Secure, HttpOnly and SameSite=Strict. Do not publish credential files, live databases, backups, private uploads or account sessions.

- Logging out one device revokes only its session. Other devices keep their own tokens and stay signed in.
- Logging out other devices preserves the current session and revokes the others. Revoked devices must sign in again.
- Password reset revokes existing sessions and trusted devices. Sign in with the new password afterwards.
- Sessions do not rotate globally whenever another device logs out. No shared account-wide token is used.

An expired or revoked token cannot silently regain access. Returning a new valid token to a revoked device would defeat the logout. Locally modified clients cannot bypass server authentication, role checks or invite requirements.

Provider, mail, database and upload encryption keys stay on the server. Microsoft login tokens belong to the individual user and are needed locally to launch Minecraft. Windows malware already running as that user is outside DPAPI's protection boundary.

Before making a repository public, check its complete history and all release downloads. Older Canna executables before 0.2.15 contained a shared GitHub read token. Keep those releases private/draft and revoke obsolete shared credentials; publishing new source does not erase older binaries.

Do not post vulnerabilities containing real credentials or account data in public issues. Report them privately to the repository owner.

## License scope

The MIT license covers original Canna code. Dependencies, third-party mods, game artwork and game logos retain their respective licenses and trademarks. This repository does not grant a license to proprietary game assemblies or private mod archives. Do not include those files in source releases.
