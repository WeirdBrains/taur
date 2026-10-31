# MCP server

TAUR speaks the [Model Context Protocol](https://modelcontextprotocol.io) so AI
assistants can search packages and, with your token, manage them.

## Streamable HTTP: `POST /mcp`
- JSON-RPC 2.0 messages (single or batch) in the body; replies are `application/json`.
  Notifications get `202 Accepted` with no body. The server is stateless: no
  `Mcp-Session-Id`, no server-initiated stream (`GET`/`DELETE /mcp` return 405).
- Protocol versions: `2025-06-18` (default), `2025-03-26`, `2024-11-05`.
- Auth: `Authorization: Bearer <token>` (a `taurctl login` / `POST /api/v1/login`
  token). Public tools work without it; write tools need a verified account.
  Permissions are exactly those of the JSON API.

Client config example (HTTP):

```json
{ "mcpServers": { "taur": { "url": "https://taur.weirdbrains.com/mcp",
  "headers": { "Authorization": "Bearer YOUR_TOKEN" } } } }
```

## stdio: `taurctl mcp`
Reads JSON-RPC lines on stdin, forwards each to the configured server's `/mcp` with
the stored session token (if logged in), and writes replies to stdout:

```json
{ "mcpServers": { "taur": { "command": "taurctl", "args": ["mcp"] } } }
```

## Tools
Public: `search_packages` (`query`, `by`), `package_info` (`names`), `list_comments`.

Signed in: `whoami`, `my_packages`, `vote` (`on`), `notify` (`on`),
`flag_out_of_date`, `unflag`, `add_comment`, `pin_comment`, `delete_comment`,
`set_comaintainers`, `adopt`, `disown`, `transfer_package` (`org` or `user`),
`file_request`, `list_requests`, `close_request` (`accept`), `list_orgs`, `show_org`,
`create_org`, `set_member`, `remove_member`, `delete_org`, `list_ssh_keys`,
`add_ssh_key`, `remove_ssh_key`.

Each tool carries `readOnlyHint`/`destructiveHint` annotations so clients can ask
before destructive calls. Results are JSON as text plus `structuredContent`; failures
come back as `isError: true` tool results. Account creation, login and deletion are
not exposed.
