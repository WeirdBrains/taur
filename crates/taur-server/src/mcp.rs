//! Model Context Protocol (MCP) server: JSON-RPC 2.0 over the streamable HTTP transport
//! (`POST /mcp`, JSON responses, stateless). Tools are thin wrappers over `taur-core`;
//! management tools use the caller's bearer token.

use serde_json::{Value, json};
use taur_core::{Error, Role, Store, accounts::User};

pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

struct Tool {
    name: &'static str,
    description: &'static str,
    /// (name, JSON type, description, required)
    params: &'static [(&'static str, &'static str, &'static str, bool)],
    auth: bool,
    read_only: bool,
    destructive: bool,
}

const P: (&str, &str, &str, bool) = ("pkgbase", "string", "Package base name", true);

macro_rules! tool {
    ($n:expr, $d:expr, [$($p:expr),*], auth=$a:expr, ro=$r:expr, destructive=$x:expr) => {
        Tool { name: $n, description: $d, params: &[$($p),*], auth: $a, read_only: $r, destructive: $x }
    };
}

const TOOLS: &[Tool] = &[
    tool!(
        "search_packages",
        "Search packages (AUR RPC v5 search). `by`: name, name-desc (default), maintainer, depends, makedepends, optdepends, checkdepends, keywords, groups, provides, conflicts, replaces, submitter, comaintainers.",
        [
            ("query", "string", "Search term", true),
            ("by", "string", "Field to search", false)
        ],
        auth = false,
        ro = true,
        destructive = false
    ),
    tool!(
        "package_info",
        "Full details for packages by name (AUR RPC v5 info).",
        [("names", "array", "Package names", true)],
        auth = false,
        ro = true,
        destructive = false
    ),
    tool!(
        "list_comments",
        "Comments on a package base (pinned first).",
        [P],
        auth = false,
        ro = true,
        destructive = false
    ),
    tool!(
        "whoami",
        "The signed-in account.",
        [],
        auth = true,
        ro = true,
        destructive = false
    ),
    tool!(
        "reject_package",
        "Reject a package base (moderator or site admin). The repository is kept and hidden from search.",
        [P, ("reason", "string", "Why it is rejected", true)],
        auth = true,
        ro = false,
        destructive = true
    ),
    tool!(
        "reject_user",
        "Reject an ordinary account (moderator or site admin). Sessions end; the account row stays.",
        [
            ("name", "string", "User name", true),
            ("reason", "string", "Why the account is rejected", true)
        ],
        auth = true,
        ro = false,
        destructive = true
    ),
    tool!(
        "list_audit",
        "List rejection and reversal events. Site admin only.",
        [],
        auth = true,
        ro = true,
        destructive = false
    ),
    tool!(
        "reverse_rejection",
        "Reverse one open package or user rejection. Site admin only. Does not create an account or package.",
        [
            ("id", "integer", "Audit event id", true),
            ("reason", "string", "Why it is reversed", true)
        ],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "accept_rejection",
        "Accept one rejected upload or registration so that same attempt can be retried. Site admin only. Does not create an account or store a recipe.",
        [
            ("id", "integer", "Audit event id", true),
            ("reason", "string", "Why it is accepted", true)
        ],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "revoke_acceptance",
        "Withdraw an acceptance of a rejected upload or registration. Site admin only. The business rule applies to that attempt again.",
        [
            ("id", "integer", "Acceptance event id", true),
            ("reason", "string", "Why it is withdrawn", true)
        ],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "my_packages",
        "Package bases you maintain, co-maintain or can manage via organizations.",
        [],
        auth = true,
        ro = true,
        destructive = false
    ),
    tool!(
        "vote",
        "Vote (or with on=false, remove your vote) for a package base.",
        [
            P,
            (
                "on",
                "boolean",
                "true to vote (default), false to unvote",
                false
            )
        ],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "notify",
        "Subscribe (or with on=false, unsubscribe) to email notifications for a package base.",
        [P, ("on", "boolean", "true to subscribe (default)", false)],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "flag_out_of_date",
        "Flag a package base out-of-date with a comment.",
        [P, ("comment", "string", "Why it is out of date", true)],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "unflag",
        "Remove the out-of-date flag.",
        [P],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "add_comment",
        "Comment on a package base.",
        [P, ("content", "string", "Comment text", true)],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "pin_comment",
        "Pin (or with pinned=false, unpin) a comment.",
        [
            ("id", "integer", "Comment id", true),
            ("pinned", "boolean", "Default true", false)
        ],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "delete_comment",
        "Delete a comment.",
        [("id", "integer", "Comment id", true)],
        auth = true,
        ro = false,
        destructive = true
    ),
    tool!(
        "set_comaintainers",
        "Replace the co-maintainer list of a package base.",
        [P, ("users", "array", "User names", true)],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "adopt",
        "Adopt an orphaned package base.",
        [P],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "disown",
        "Give up maintainership of a package base (it becomes orphaned).",
        [P],
        auth = true,
        ro = false,
        destructive = true
    ),
    tool!(
        "transfer_package",
        "Transfer a package base to an organization or a user (exactly one).",
        [
            P,
            ("org", "string", "Organization name", false),
            ("user", "string", "User name", false)
        ],
        auth = true,
        ro = false,
        destructive = true
    ),
    tool!(
        "file_request",
        "File an orphan, deletion or merge request.",
        [
            P,
            ("kind", "string", "orphan, deletion or merge", true),
            ("merge_into", "string", "Target for merge", false),
            ("comment", "string", "Reason", true)
        ],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "list_requests",
        "Package requests (pending only unless all=true).",
        [("all", "boolean", "Include closed requests", false)],
        auth = true,
        ro = true,
        destructive = false
    ),
    tool!(
        "close_request",
        "Accept or reject a request (site admins; requesters may reject to withdraw).",
        [
            ("id", "integer", "Request id", true),
            ("accept", "boolean", "true to accept, false to reject", true),
            ("comment", "string", "Closure comment", false)
        ],
        auth = true,
        ro = false,
        destructive = true
    ),
    tool!(
        "list_orgs",
        "Your organizations and roles.",
        [],
        auth = true,
        ro = true,
        destructive = false
    ),
    tool!(
        "show_org",
        "An organization's members and packages.",
        [("org", "string", "Organization", true)],
        auth = true,
        ro = true,
        destructive = false
    ),
    tool!(
        "create_org",
        "Create an organization (you become owner).",
        [("org", "string", "Organization", true)],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "set_member",
        "Add a member or change their role.",
        [
            ("org", "string", "Organization", true),
            ("user", "string", "User", true),
            ("role", "string", "owner, admin or contributor", true)
        ],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "remove_member",
        "Remove a member from an organization.",
        [
            ("org", "string", "Organization", true),
            ("user", "string", "User", true)
        ],
        auth = true,
        ro = false,
        destructive = true
    ),
    tool!(
        "delete_org",
        "Delete an organization (owners).",
        [("org", "string", "Organization", true)],
        auth = true,
        ro = false,
        destructive = true
    ),
    tool!(
        "list_ssh_keys",
        "Your SSH public keys.",
        [],
        auth = true,
        ro = true,
        destructive = false
    ),
    tool!(
        "add_ssh_key",
        "Add an SSH public key (OpenSSH format).",
        [("key", "string", "Public key line", true)],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "remove_ssh_key",
        "Remove one of your SSH keys.",
        [("id", "integer", "Key id", true)],
        auth = true,
        ro = false,
        destructive = true
    ),
];

fn tool_json(t: &Tool) -> Value {
    let mut props = serde_json::Map::new();
    let mut req = vec![];
    for (n, ty, d, r) in t.params {
        let mut p = json!({"type": ty, "description": d});
        if *ty == "array" {
            p["items"] = json!({"type": "string"});
        }
        props.insert(n.to_string(), p);
        if *r {
            req.push(*n);
        }
    }
    json!({
        "name": t.name,
        "description": if t.auth { format!("{} Requires a bearer token.", t.description) } else { t.description.to_string() },
        "inputSchema": {"type": "object", "properties": props, "required": req},
        "annotations": {"readOnlyHint": t.read_only, "destructiveHint": t.destructive, "openWorldHint": false},
    })
}

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_err(id: Value, code: i64, msg: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}})
}

/// Handle one JSON-RPC message (or batch). `None` means no response (notifications).
pub fn handle(store: &Store, token: Option<&str>, msg: Value) -> Option<Value> {
    if let Value::Array(batch) = msg {
        let out: Vec<Value> = batch
            .into_iter()
            .filter_map(|m| handle(store, token, m))
            .collect();
        return (!out.is_empty()).then_some(Value::Array(out));
    }
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str);
    let (Some(id), Some(method)) = (id, method) else {
        // Notifications (no id) and responses get no reply.
        return if msg.get("method").is_some() || msg.get("result").is_some() {
            None
        } else {
            Some(rpc_err(Value::Null, -32600, "invalid request"))
        };
    };
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    Some(match method {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or("");
            let v = if PROTOCOL_VERSIONS.contains(&asked) {
                asked
            } else {
                PROTOCOL_VERSIONS[0]
            };
            rpc_ok(
                id,
                json!({
                    "protocolVersion": v,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "taur", "title": "TAUR", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": "TAUR is an AUR-compatible package server. Search and info are public; management tools need the user's TAUR bearer token.",
                }),
            )
        }
        "ping" => rpc_ok(id, json!({})),
        "tools/list" => rpc_ok(
            id,
            json!({"tools": TOOLS.iter().map(tool_json).collect::<Vec<_>>()}),
        ),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let Some(tool) = TOOLS.iter().find(|t| t.name == name) else {
                return Some(rpc_err(id, -32602, &format!("unknown tool: {name}")));
            };
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let result = (|| -> taur_core::Result<Value> {
                let user = if tool.auth {
                    let u = store.user_for_token(token.ok_or(Error::Unauthorized)?)?;
                    if !tool.read_only {
                        store.require_verified(&u)?;
                    }
                    Some(u)
                } else {
                    None
                };
                call(store, tool.name, user, &args)
            })();
            match result {
                Ok(v) => rpc_ok(
                    id,
                    json!({
                        "content": [{"type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default()}],
                        "structuredContent": if v.is_object() { v.clone() } else { json!({"result": v}) },
                        "isError": false,
                    }),
                ),
                Err(e) => {
                    let text = match &e {
                        Error::Db(_) | Error::Io(_) | Error::Json(_) | Error::Mail(_) => {
                            eprintln!("taur: mcp internal error: {e}");
                            "internal error".to_string()
                        }
                        _ => e.to_string(),
                    };
                    rpc_ok(
                        id,
                        json!({"content": [{"type": "text", "text": text}], "isError": true}),
                    )
                }
            }
        }
        _ => rpc_err(id, -32601, &format!("method not found: {method}")),
    })
}

fn s<'a>(a: &'a Value, k: &str) -> taur_core::Result<&'a str> {
    a[k].as_str()
        .ok_or_else(|| Error::Invalid(format!("missing string argument \"{k}\"")))
}

fn i(a: &Value, k: &str) -> taur_core::Result<i64> {
    a[k].as_i64()
        .ok_or_else(|| Error::Invalid(format!("missing integer argument \"{k}\"")))
}

fn b(a: &Value, k: &str, default: bool) -> bool {
    a[k].as_bool().unwrap_or(default)
}

fn strings(a: &Value, k: &str) -> taur_core::Result<Vec<String>> {
    a[k].as_array()
        .ok_or_else(|| Error::Invalid(format!("missing array argument \"{k}\"")))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(String::from)
                .ok_or_else(|| Error::Invalid(format!("\"{k}\" must contain strings")))
        })
        .collect()
}

fn pb(store: &Store, name: &str) -> taur_core::Result<i64> {
    store
        .pkgbase_id(name)?
        .ok_or_else(|| Error::NotFound(format!("package base {name}")))
}

fn done() -> Value {
    json!({"ok": true})
}

fn call(store: &Store, name: &str, user: Option<User>, a: &Value) -> taur_core::Result<Value> {
    let uid = user.as_ref().map(|u| u.id).unwrap_or_default();
    Ok(match name {
        "search_packages" => {
            let r = taur_core::rpc::handle(
                store,
                Some("search"),
                &[s(a, "query")?.to_string()],
                a["by"].as_str(),
            );
            if r["type"] == "error" {
                return Err(Error::Invalid(r["error"].as_str().unwrap_or("").into()));
            }
            r
        }
        "package_info" => taur_core::rpc::handle(store, Some("info"), &strings(a, "names")?, None),
        "list_comments" => serde_json::to_value(store.comments(s(a, "pkgbase")?)?)?,
        "whoami" => serde_json::to_value(user)?,
        "reject_package" => {
            let id = store.reject_package(uid, s(a, "pkgbase")?, s(a, "reason")?)?;
            json!({"event": id})
        }
        "reject_user" => {
            let id = store.reject_user(uid, s(a, "name")?, s(a, "reason")?)?;
            json!({"event": id})
        }
        "list_audit" => serde_json::to_value(store.list_audit(uid)?)?,
        "reverse_rejection" => {
            let id = store.reverse_audit(uid, i(a, "id")?, s(a, "reason")?)?;
            json!({"event": id})
        }
        "accept_rejection" => {
            let id = store.accept_rejection(uid, i(a, "id")?, s(a, "reason")?)?;
            json!({"event": id})
        }
        "revoke_acceptance" => {
            let id = store.revoke_acceptance(uid, i(a, "id")?, s(a, "reason")?)?;
            json!({"event": id})
        }
        "my_packages" => serde_json::to_value(store.packages_for(uid)?)?,
        "vote" => {
            if b(a, "on", true) {
                store.vote(uid, s(a, "pkgbase")?)?
            } else {
                store.unvote(uid, s(a, "pkgbase")?)?
            }
            done()
        }
        "notify" => {
            store.subscribe(uid, s(a, "pkgbase")?, b(a, "on", true))?;
            done()
        }
        "flag_out_of_date" => {
            store.flag_out_of_date(uid, s(a, "pkgbase")?, s(a, "comment")?)?;
            done()
        }
        "unflag" => {
            store.unflag(uid, s(a, "pkgbase")?)?;
            done()
        }
        "add_comment" => {
            serde_json::to_value(store.add_comment(uid, s(a, "pkgbase")?, s(a, "content")?)?)?
        }
        "pin_comment" => {
            store.pin_comment(uid, i(a, "id")?, b(a, "pinned", true))?;
            done()
        }
        "delete_comment" => {
            store.delete_comment(uid, i(a, "id")?)?;
            done()
        }
        "set_comaintainers" => {
            store.set_comaintainers(uid, s(a, "pkgbase")?, &strings(a, "users")?)?;
            done()
        }
        "adopt" => {
            store.adopt(uid, pb(store, s(a, "pkgbase")?)?)?;
            done()
        }
        "disown" => {
            store.disown(uid, pb(store, s(a, "pkgbase")?)?)?;
            done()
        }
        "transfer_package" => {
            let p = pb(store, s(a, "pkgbase")?)?;
            match (a["org"].as_str(), a["user"].as_str()) {
                (Some(o), None) => store.set_pkgbase_org(uid, p, Some(store.org_id(o)?))?,
                (None, Some(u)) => store.transfer_to_user(uid, p, store.user_id(u)?)?,
                _ => {
                    return Err(Error::Invalid(
                        "give exactly one of \"org\" or \"user\"".into(),
                    ));
                }
            }
            done()
        }
        "file_request" => serde_json::to_value(store.file_request(
            uid,
            s(a, "pkgbase")?,
            s(a, "kind")?,
            a["merge_into"].as_str(),
            s(a, "comment")?,
        )?)?,
        "list_requests" => serde_json::to_value(store.list_requests(!b(a, "all", false))?)?,
        "close_request" => {
            let id = i(a, "id")?;
            let c = a["comment"].as_str().unwrap_or("");
            if b(a, "accept", false) {
                store.accept_request(uid, id, c)?
            } else {
                store.reject_request(uid, id, c)?
            }
            serde_json::to_value(store.request(id)?)?
        }
        "list_orgs" => serde_json::to_value(store.orgs_for(uid)?)?,
        "show_org" => serde_json::to_value(store.org_detail(uid, s(a, "org")?)?)?,
        "create_org" => {
            store.create_org(uid, s(a, "org")?)?;
            serde_json::to_value(store.org_detail(uid, s(a, "org")?)?)?
        }
        "set_member" => {
            let role = Role::parse(s(a, "role")?)
                .ok_or_else(|| Error::Invalid("role must be owner, admin or contributor".into()))?;
            store.set_member(
                uid,
                store.org_id(s(a, "org")?)?,
                store.user_id(s(a, "user")?)?,
                role,
            )?;
            done()
        }
        "remove_member" => {
            store.remove_member(
                uid,
                store.org_id(s(a, "org")?)?,
                store.user_id(s(a, "user")?)?,
            )?;
            done()
        }
        "delete_org" => {
            store.delete_org(uid, store.org_id(s(a, "org")?)?)?;
            done()
        }
        "list_ssh_keys" => serde_json::to_value(store.list_ssh_keys(uid)?)?,
        "add_ssh_key" => serde_json::to_value(store.add_ssh_key(uid, s(a, "key")?)?)?,
        "remove_ssh_key" => {
            store.remove_ssh_key(uid, i(a, "id")?)?;
            done()
        }
        _ => return Err(Error::Invalid(format!("unknown tool: {name}"))),
    })
}
