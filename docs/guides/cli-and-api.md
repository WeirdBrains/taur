---
title: taurctl, MCP, and the API
summary: The same account from the command line or another program.
audience: user
order: 70
---

## taurctl

```sh
taurctl config set-server https://YOUR-SERVER
taurctl register alice --email alice@example.com
taurctl login alice
taurctl whoami
```

The session token stays in the config directory, mode 0600, and is not printed. `taurctl --json` prints the API response. Commands match the site: search, packages, keys, organizations, comments, requests, and moderation.

`taurctl mcp` speaks MCP on standard input and uses that stored token. It is a local proxy, not a page on the site.

## HTTP API

Send `Authorization: Bearer TOKEN`. A request that changes something also sends `X-CSRF-Token` set to the same token. Register, login, verify, and the password-reset links do not. `taurctl` adds the header for you.

The account, organization, package, request, and moderation paths are under `/api/v1/`. Search uses `/rpc/v5/search/...`, the same RPC yay uses.
