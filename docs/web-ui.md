# Web UI

`taur serve` serves a static web interface (plain HTML/CSS/JavaScript in
`crates/taur-server/web/`, embedded in the binary; no Node toolchain). It is a thin
client of the JSON API, like `taurctl`.

## Pages
| Path | What |
|---|---|
| `/`, `/packages?q=&by=` | Search packages (all RPC `by` fields), sorted by popularity |
| `/packages/<name>` | Package details (deps, maintainers, org, votes, popularity, flag, clone URL, snapshot), comments; when signed in: vote, flag/unflag, adopt/disown, comment/edit/delete, file requests; maintainers also edit co-maintainers and transfer to an org or user. Same URL yay shows as "AUR URL". |
| `/register`, `/login` | Account creation and sign-in |
| `/verify?token=` | Target of the verification email |
| `/reset`, `/reset?token=` | Request a reset link; target of the reset email |
| `/account` | Verification status/resend, your packages, SSH keys, delete account |
| `/orgs`, `/orgs/<name>` | Your organizations; create; members, role changes, add/remove, delete (per role) |
| `/requests` | Pending (or all) requests; accept/reject for site admins, withdraw your own |

Buttons are shown when they're likely to be allowed; the server enforces every rule.

## Auth and security
- Bearer token from `POST /api/v1/login`, kept in `localStorage` (`taur.token`) and
  sent as `Authorization: Bearer`. No cookies, so no CSRF surface. Logging out calls
  `/api/v1/logout` and forgets the token.
- All data is inserted as text nodes (never parsed as HTML). Pages are sent with a strict
  Content-Security-Policy (`script-src 'self'`, no inline script), `nosniff` and
  `no-referrer`.
- Confirmations: delete account, delete organization, disown (matching `taurctl`).

## Tests
HTTP tests check every page route and asset is served with the right headers and that
the API calls the UI makes work. A headless Chromium check renders package, search,
verify and reset pages: `cargo test -p taur-server -- --ignored` (needs local `chromium`).
