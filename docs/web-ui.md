# Web UI

`taur serve` serves a static web interface (plain HTML/CSS/JavaScript in
`crates/taur-server/web/`, embedded in the binary; no Node toolchain). It is a thin
client of the JSON API, like `taurctl`. The footer shows the running server's version
(`CARGO_PKG_VERSION` of the `taur` binary). The same TAUR mark used on the project site is served as `/favicon.ico` (embedded `web/favicon.ico`, `image/x-icon`) and linked from the page.

## Pages
| Path | What |
|---|---|
| `/docs`, `/docs/<slug>` | Task guides compiled from `docs/guides/`. The server hides a guide the role cannot read (404). |
| `/upload` | Verified users upload a `.tar.gz`, `.tar`, or `.zip` containing `PKGBUILD` and `.SRCINFO`. The owner list defaults to their account and includes organizations where they can manage packages (Owner, Admin, or Contributor). Open the list and type in the filter at the top to narrow it. |
| `/packages/<name>` | Also: maintainers archive or unarchive, and mark a package out of date or clear that flag. Anyone signed in can flag it. |
| `/`, `/packages?q=&by=` | Search packages (all RPC `by` fields), sorted by popularity |
| `/packages/<name>` | Package details (deps, maintainers, org, votes, popularity, flag, clone URL, snapshot), comments; when signed in: vote, flag/unflag, adopt/disown, comment/edit/delete, file requests; maintainers also edit co-maintainers and transfer to an org or user. Same URL yay shows as "AUR URL". Maintainer and co-maintainer names link to `/users/<name>`. |
| `/users/<name>` | Packages that account maintains and co-maintains. |
| `/register`, `/login` | Account creation and sign-in |
| `/verify?token=` | Target of the verification email |
| `/reset`, `/reset?token=` | Request a reset link; target of the reset email |
| `/account` | Verification status/resend, your packages, SSH keys, sessions, delete account |
| `/orgs`, `/orgs/<name>` | Your organizations; create; members, role changes, add/remove, delete (per role) |
| `/requests` | Pending (or all) requests; accept/reject for site admins, withdraw your own |
| `/admin` | User search. Moderators and site admins. Email addresses only for site admins. |
| `/users/<name>` | Plus, for a moderator or site admin: reject. For a site admin: verify, resend, password reset, role grants, revoke sessions. |
| `/audit` | Rejection history. Site admin: accept, reverse, revoke. |
| `/registries`, `/registries/<name>` | Site owner: enforcing registries, operator keys, identity bindings, expected recipes. |
| `/mirrors`, `/mirrors/<name>` | Site admin: add a recipe source, select, sync, unselect, drop, remove. |

Buttons are shown when they're likely to be allowed; the server enforces every rule.
Admin links are shown only for a moderator or site admin, and registry, mirror, and
audit links only for a site admin.

## Auth and security
- Bearer token from `POST /api/v1/login`, kept in `localStorage` (`taur.token`) and
  sent as `Authorization: Bearer`. No cookies. Logging out calls `/api/v1/logout` and
  forgets the token.
- Every state-changing `/api/v1` request except register, login, verify, and the
  password-reset links also sends `X-CSRF-Token` set to the same bearer token.
  A cross-site form cannot set that header. A missing or different token is `403`.
- All data is inserted as text nodes (never parsed as HTML). Pages are sent with a strict
  Content-Security-Policy (`script-src 'self'`, no inline script), `nosniff` and
  `no-referrer`.
- Confirmations: delete account, delete organization, disown, reject, revoke a session or a role, drop or remove a mirror, revoke an acceptance (matching `taurctl`).

## Tests
HTTP tests check every page route and asset is served with the right headers and that
the API calls the UI makes work. A headless Chromium check renders package, search,
verify and reset pages: `cargo test -p taur-server -- --ignored` (needs local `chromium`).
