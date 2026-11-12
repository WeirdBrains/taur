//! Model Context Protocol (MCP) server: JSON-RPC 2.0 over the streamable HTTP transport
//! (`POST /mcp`, JSON responses, stateless). Tools are thin wrappers over `taur-core`;
//! management tools use the caller's bearer token. The protocol handling (initialize,
//! ping, tools/list, tools/call, batches) is `weirdnet-mcp`'s.

use serde_json::{Value, json};
use taur_core::{Error, Role, Store, accounts::User};
use weirdnet_mcp::{Param, ParamType, Server, ServerInfo, ToolError, ToolSpec, Tools};

const P: Param = Param::required("pkgbase", ParamType::String, "Package base name");

macro_rules! tool {
    ($n:expr, $d:expr, [$($p:expr),*], auth=$a:expr, ro=$r:expr, destructive=$x:expr) => {
        ToolSpec { name: $n, description: $d, params: &[$($p),*], auth: $a, read_only: $r, destructive: $x, open_world: false }
    };
}

const TOOLS: &[ToolSpec] = &[
    tool!(
        "search_packages",
        "Search packages (AUR RPC v5 search). `by`: name, name-desc (default), maintainer, depends, makedepends, optdepends, checkdepends, keywords, groups, provides, conflicts, replaces, submitter, comaintainers.",
        [
            Param::required("query", ParamType::String, "Search term"),
            Param::optional("by", ParamType::String, "Field to search")
        ],
        auth = false,
        ro = true,
        destructive = false
    ),
    tool!(
        "package_info",
        "Full details for packages by name (AUR RPC v5 info).",
        [Param::required(
            "names",
            ParamType::StringArray,
            "Package names"
        )],
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
        [
            P,
            Param::required("reason", ParamType::String, "Why it is rejected")
        ],
        auth = true,
        ro = false,
        destructive = true
    ),
    tool!(
        "reject_user",
        "Reject an ordinary account (moderator or site admin). Sessions end; the account row stays.",
        [
            Param::required("name", ParamType::String, "User name"),
            Param::required("reason", ParamType::String, "Why the account is rejected")
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
            Param::required("id", ParamType::Integer, "Audit event id"),
            Param::required("reason", ParamType::String, "Why it is reversed")
        ],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "accept_rejection",
        "Accept one rejected upload or registration so that same attempt can be retried. Site admin only. Does not create an account or store a recipe.",
        [
            Param::required("id", ParamType::Integer, "Audit event id"),
            Param::required("reason", ParamType::String, "Why it is accepted")
        ],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "revoke_acceptance",
        "Withdraw an acceptance of a rejected upload or registration. Site admin only. The business rule applies to that attempt again.",
        [
            Param::required("id", ParamType::Integer, "Acceptance event id"),
            Param::required("reason", ParamType::String, "Why it is withdrawn")
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
            Param::optional(
                "on",
                ParamType::Boolean,
                "true to vote (default), false to unvote"
            )
        ],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "notify",
        "Subscribe (or with on=false, unsubscribe) to email notifications for a package base.",
        [
            P,
            Param::optional("on", ParamType::Boolean, "true to subscribe (default)")
        ],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "flag_out_of_date",
        "Flag a package base out-of-date with a comment.",
        [
            P,
            Param::required("comment", ParamType::String, "Why it is out of date")
        ],
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
        [
            P,
            Param::required("content", ParamType::String, "Comment text")
        ],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "pin_comment",
        "Pin (or with pinned=false, unpin) a comment.",
        [
            Param::required("id", ParamType::Integer, "Comment id"),
            Param::optional("pinned", ParamType::Boolean, "Default true")
        ],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "delete_comment",
        "Delete a comment.",
        [Param::required("id", ParamType::Integer, "Comment id")],
        auth = true,
        ro = false,
        destructive = true
    ),
    tool!(
        "set_comaintainers",
        "Replace the co-maintainer list of a package base.",
        [
            P,
            Param::required("users", ParamType::StringArray, "User names")
        ],
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
            Param::optional("org", ParamType::String, "Organization name"),
            Param::optional("user", ParamType::String, "User name")
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
            Param::required("kind", ParamType::String, "orphan, deletion or merge"),
            Param::optional("merge_into", ParamType::String, "Target for merge"),
            Param::required("comment", ParamType::String, "Reason")
        ],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "list_requests",
        "Package requests (pending only unless all=true).",
        [Param::optional(
            "all",
            ParamType::Boolean,
            "Include closed requests"
        )],
        auth = true,
        ro = true,
        destructive = false
    ),
    tool!(
        "close_request",
        "Accept or reject a request (site admins; requesters may reject to withdraw).",
        [
            Param::required("id", ParamType::Integer, "Request id"),
            Param::required(
                "accept",
                ParamType::Boolean,
                "true to accept, false to reject"
            ),
            Param::optional("comment", ParamType::String, "Closure comment")
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
        [Param::required("org", ParamType::String, "Organization")],
        auth = true,
        ro = true,
        destructive = false
    ),
    tool!(
        "create_org",
        "Create an organization (you become owner).",
        [Param::required("org", ParamType::String, "Organization")],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "set_member",
        "Add a member or change their role.",
        [
            Param::required("org", ParamType::String, "Organization"),
            Param::required("user", ParamType::String, "User"),
            Param::required("role", ParamType::String, "owner, admin or contributor")
        ],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "remove_member",
        "Remove a member from an organization.",
        [
            Param::required("org", ParamType::String, "Organization"),
            Param::required("user", ParamType::String, "User")
        ],
        auth = true,
        ro = false,
        destructive = true
    ),
    tool!(
        "delete_org",
        "Delete an organization (owners).",
        [Param::required("org", ParamType::String, "Organization")],
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
        [Param::required("key", ParamType::String, "Public key line")],
        auth = true,
        ro = false,
        destructive = false
    ),
    tool!(
        "remove_ssh_key",
        "Remove one of your SSH keys.",
        [Param::required("id", ParamType::Integer, "Key id")],
        auth = true,
        ro = false,
        destructive = true
    ),
];

struct TaurTools<'a>(&'a Store);

impl Tools for TaurTools<'_> {
    fn specs(&self) -> &[ToolSpec] {
        TOOLS
    }

    fn call(&self, tool: &ToolSpec, args: &Value, token: Option<&str>) -> Result<Value, ToolError> {
        let store = self.0;
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
            call(store, tool.name, user, args)
        })();
        result.map_err(|e| match &e {
            Error::Db(_) | Error::Io(_) | Error::Json(_) | Error::Mail(_) => {
                eprintln!("taur: mcp internal error: {e}");
                ToolError("internal error".into())
            }
            _ => ToolError(e.to_string()),
        })
    }
}

/// Handle one JSON-RPC message (or batch). `None` means no response (notifications).
pub fn handle(store: &Store, token: Option<&str>, msg: Value) -> Option<Value> {
    let info = ServerInfo {
        name: "taur".into(),
        title: "TAUR".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        instructions: "TAUR is an AUR-compatible package server. Search and info are public; management tools need the user's TAUR bearer token.".into(),
    };
    Server::new(info, TaurTools(store)).handle(token, msg)
}

fn s<'a>(a: &'a Value, k: &str) -> taur_core::Result<&'a str> {
    weirdnet_mcp::args::string(a, k).map_err(Error::Invalid)
}

fn i(a: &Value, k: &str) -> taur_core::Result<i64> {
    weirdnet_mcp::args::integer(a, k).map_err(Error::Invalid)
}

fn b(a: &Value, k: &str, default: bool) -> bool {
    weirdnet_mcp::args::boolean(a, k, default)
}

fn strings(a: &Value, k: &str) -> taur_core::Result<Vec<String>> {
    weirdnet_mcp::args::strings(a, k).map_err(Error::Invalid)
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
