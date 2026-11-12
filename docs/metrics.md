# Usage metrics

TAUR counts web UI page views and package downloads, and shows them on package pages,
on the public `/stats` page and through `GET /api/v1/metrics`. It does not count
installs. Only daily aggregates are kept; no client address is stored.

## What is counted

| Metric | Event | Not counted |
|---|---|---|
| Page views | The web UI reports each page it renders (`POST /api/v1/views` with the path only, no query string or session token). A view counts at most once per visitor and page every 30 minutes. | Pages a browser loads without running the UI (crawlers without JavaScript), requests with `DNT: 1` or `Sec-GPC: 1`, unknown routes, package pages for packages that do not exist, a visitor's pages beyond 100 per day. |
| Visitors | Distinct visitors per UTC day. A visitor is the first 16 bytes of SHA-256 over a random salt for that day, the client address and the User-Agent. | Cross-day identity: the salt and every visitor hash are deleted when the day ends, so the same person on two days counts twice (the API calls the sum `visitor_days`). People sharing one address and browser count once. |
| Downloads | A successful `GET` of `/cgit/aur.git/snapshot/<pkgbase>.tar.gz` (current or `?id=` revision), the same event as `DownloadCount`. Kept as a per-package-base daily total as well as the all-time counter. | `HEAD`, failures, plain-file views, RPC, metadata dumps and git clones (paru clones over git, so paru installs are not seen). |
| Installs | Not counted. pacman, yay and paru do not report installs, and TAUR does not add a client that phones home. A download is not an install; `installs` in the API is always `null`. | — |

Page keys are normalized so the set stays small and holds no secrets: `/packages/<name>`
for an existing package, `/packages/<name>/revisions/*`, and templates such as
`/users/*`, `/orgs/*` and `/docs/*` for other named pages. Query strings (which can
carry verification or reset tokens) are dropped.

The client address is resolved the same way as for rate limits: `X-Forwarded-For` is
used only from `TAUR_TRUSTED_PROXIES`. Behind a proxy that is not trusted, every
visitor shares the proxy address and visitors are distinguished only by User-Agent.

## Storage and retention

Tables in the SQLite database: `metric_salts` and `page_view_seen` (today only),
`page_view_days`, `site_visitor_days` and `download_days` (one row per day and page or
package base, deleted after 400 days). They are created on start; existing databases
start at zero, and downloads before this version appear only in the all-time
`DownloadCount`.

## Turning it off

`TAUR_PAGE_VIEWS=false` (or `taur serve --page-views false`) stops counting page views.
The endpoint still answers `204`, and `/api/v1/metrics` reports
`"page_views_enabled": false`. Downloads are always counted.

## API

`POST /api/v1/views` with `{"path": "/packages/foo"}` answers `204` whether or not the
view was counted. It is rate limited with the rest of `/api/v1` and needs no account.

`GET /api/v1/metrics?days=30` (1–365, default 30, today included, UTC) is public:

```json
{
  "days": 30, "views": 120, "visitor_days": 45, "downloads": 18,
  "downloads_all_time": 230, "installs": null, "page_views_enabled": true,
  "top_pages": [{"name": "/packages/taur", "count": 40}],
  "top_downloads": [{"name": "taur", "count": 12}],
  "daily": [{"date": "2026-11-12", "views": 7, "visitors": 3, "downloads": 1}]
}
```

`GET /api/v1/packages/by-name/<name>` adds `PageViews30d` and `Downloads30d`. The AUR
RPC is unchanged apart from the existing `DownloadCount`.
