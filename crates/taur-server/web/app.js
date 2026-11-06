// TAUR web UI: a static single-page app over the JSON API.
// Auth uses a bearer token kept in localStorage (no cookies). Never logged.
"use strict";

const TOKEN_KEY = "taur.token";
const token = () => localStorage.getItem(TOKEN_KEY);
let me = null; // cached account

// ---- DOM helpers (text nodes only; data is never parsed as HTML) ----
function h(tag, attrs, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === null || v === undefined || v === false) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "class") el.className = v;
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid === null || kid === undefined || kid === false) continue;
    el.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
  return el;
}
const link = (href, text) => h("a", { href, "data-link": true }, text);
const app = () => document.getElementById("app");
function render(...nodes) { app().replaceChildren(...nodes); }
function msg(text, kind = "error") { return h("div", { class: `msg ${kind}`, role: "status" }, text); }
function flash(text, kind = "ok") { app().prepend(msg(text, kind)); window.scrollTo(0, 0); }
const fmtTime = (t) => (t ? new Date(t * 1000).toLocaleString() : "—");
const val = (form, name) => form.elements[name].value;

// ---- API ----
async function api(method, path, body, auth = true) {
  const headers = {};
  if (body !== undefined) headers["Content-Type"] = "application/json";
  if (auth && token()) headers["Authorization"] = `Bearer ${token()}`;
  const res = await fetch(path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  const text = await res.text();
  let data = null;
  try { data = text ? JSON.parse(text) : null; } catch { data = text; }
  if (!res.ok) {
    if (res.status === 401 && auth && token()) { localStorage.removeItem(TOKEN_KEY); me = null; }
    throw new Error((data && data.error) || `${res.status} ${res.statusText}`);
  }
  if (data && data.type === "error") throw new Error(data.error);
  return data;
}
const enc = encodeURIComponent;

async function loadMe() {
  if (!token()) { me = null; return null; }
  try { me = await api("GET", "/api/v1/account"); } catch { me = null; }
  return me;
}

function nav() {
  const n = document.getElementById("nav");
  const items = [link("/", "Packages"), link("/requests", "Requests")];
  if (me) {
    items.push(link("/orgs", "Organizations"), link("/account", me.name));
    items.push(h("a", { href: "#", class: "right", onclick: async (e) => { e.preventDefault(); await logout(); } }, "Log out"));
  } else {
    items.push(h("span", { class: "right" }), link("/login", "Log in"), link("/register", "Register"));
  }
  n.replaceChildren(...items);
}

async function logout() {
  try { await api("POST", "/api/v1/logout"); } catch { /* already gone */ }
  localStorage.removeItem(TOKEN_KEY);
  me = null;
  go("/");
}

// guard wraps an action: shows errors, reruns the route on success
function guard(fn, okText) {
  return async (e) => {
    if (e) e.preventDefault();
    try { await fn(e); await route(); if (okText) flash(okText); }
    catch (err) { flash(err.message, "error"); }
  };
}

function needLogin() {
  render(h("div", { class: "card" }, h("p", {}, "Please ", link("/login", "log in"), " first.")));
}

// ---- pages ----
async function pageSearch(params) {
  const q = params.get("q") || "";
  const by = params.get("by") || "name-desc";
  const fields = ["name-desc", "name", "maintainer", "submitter", "depends", "makedepends", "optdepends", "checkdepends", "provides", "conflicts", "replaces", "groups", "keywords", "comaintainers"];
  const form = h("form", { class: "inline", onsubmit: (e) => { e.preventDefault(); go(`/?q=${enc(val(form, "q"))}&by=${enc(val(form, "by"))}`); } },
    h("input", { name: "q", value: q, placeholder: "Search packages", "aria-label": "Search" }),
    h("select", { name: "by", "aria-label": "Search by" }, fields.map((f) => h("option", { value: f, selected: f === by }, f))),
    h("button", {}, "Search"));
  const out = [h("h1", {}, "Packages"), h("div", { class: "card" }, form)];
  if (q.length >= 2 || (by === "maintainer" && q)) {
    try {
      const r = await api("GET", `/rpc/v5/search/${enc(q)}?by=${enc(by)}`, undefined, false);
      const rows = r.results.sort((a, b) => b.Popularity - a.Popularity || a.Name.localeCompare(b.Name));
      out.push(h("div", { class: "card" }, h("p", { class: "muted" }, `${r.resultcount} packages found`),
        h("table", {}, h("thead", {}, h("tr", {}, ["Name", "Version", "Votes", "Popularity", "Description", "Maintainer"].map((t) => h("th", {}, t)))),
          h("tbody", {}, rows.map((p) => h("tr", {},
            h("td", {}, link(`/packages/${enc(p.Name)}`, p.Name)),
            h("td", { class: p.OutOfDate ? "flagged" : "" }, p.Version),
            h("td", {}, p.NumVotes), h("td", {}, p.Popularity.toFixed(2)),
            h("td", {}, p.Description || ""), h("td", {}, maintainerLink(p.Maintainer))))))));
    } catch (e) { out.push(msg(e.message)); }
  } else if (q) {
    out.push(msg("Enter at least two characters."));
  }
  render(...out);
}

async function pagePackage(name) {
  let info;
  try { info = (await api("GET", `/rpc/v5/info?arg[]=${enc(name)}`, undefined, false)).results[0]; }
  catch (e) { return render(msg(e.message)); }
  if (!info) return render(msg(`No package named ${name}.`));
  const base = info.PackageBase;
  const [comments, flag] = await Promise.all([
    api("GET", `/api/v1/packages/${enc(base)}/comments`, undefined, false).catch(() => []),
    api("GET", `/api/v1/packages/${enc(base)}/flag`, undefined, false).catch(() => ({})),
  ]);
  const server = await api("GET", "/api/v1/server", undefined, false).catch(() => ({}));
  const subscribed = me ? (await api("GET", `/api/v1/packages/${enc(base)}/notify`).catch(() => ({ subscribed: false }))).subscribed : false;
  const voted = me ? (await api("GET", `/api/v1/packages/${enc(base)}/vote`).catch(() => ({ voted: false }))).voted : false;
  const list = (k) => (info[k] && info[k].length ? info[k].join(", ") : "—");
  const origin = location.origin;
  const rows = [
    ["Package base", base], ["Description", info.Description || "—"],
    ["Upstream URL", info.URL ? h("a", { href: info.URL, rel: "nofollow noopener" }, info.URL) : "—"],
    ["Git clone", h("code", {}, `${origin}/${base}.git`)],
    ...(server.ssh_clone_base ? [["SSH clone", h("code", {}, `${server.ssh_clone_base}/${base}.git`)]] : []), ["Snapshot", h("a", { href: info.URLPath }, "tarball")],
    ["Licenses", list("License")], ["Keywords", list("Keywords")], ["Depends", list("Depends")],
    ["Make depends", list("MakeDepends")], ["Check depends", list("CheckDepends")], ["Optional depends", list("OptDepends")],
    ["Provides", list("Provides")], ["Conflicts", list("Conflicts")], ["Replaces", list("Replaces")], ["Groups", list("Groups")],
    ["Maintainer", maintainerLink(info.Maintainer)], ["Co-maintainers", peopleLinks(info.CoMaintainers)],
    ["Organization", info.Organization || "—"], ["Submitter", info.Submitter || "—"],
    ["Votes", info.NumVotes], ["Popularity", info.Popularity.toFixed(2)],
    ["First submitted", fmtTime(info.FirstSubmitted)], ["Last updated", fmtTime(info.LastModified)],
    ["Out of date", info.OutOfDate ? h("span", { class: "flagged" }, `since ${fmtTime(info.OutOfDate)} by ${flag.flagger || "?"}: ${flag.comment || ""}`) : "no"],
  ];
  const dl = h("dl", {}, rows.map(([k, v]) => [h("dt", {}, k), h("dd", {}, v)]));
  const out = [h("h1", {}, `${info.Name} ${info.Version}`), h("div", { class: "card" }, dl)];

  if (me) {
    const P = `/api/v1/packages/${enc(base)}`;
    const actions = h("div", { class: "actions" },
      voted ? h("button", { class: "secondary", onclick: guard(() => api("DELETE", `${P}/vote`), "Vote removed.") }, "Remove vote")
            : h("button", { onclick: guard(() => api("POST", `${P}/vote`), "Thanks for voting.") }, "Vote"),
      subscribed ? h("button", { class: "secondary", onclick: guard(() => api("DELETE", `${P}/notify`), "Notifications off.") }, "Disable notifications")
                 : h("button", { class: "secondary", onclick: guard(() => api("POST", `${P}/notify`), "You will be emailed about updates and comments.") }, "Enable notifications"),
      info.OutOfDate ? h("button", { class: "secondary", onclick: guard(() => api("DELETE", `${P}/flag`), "Flag removed.") }, "Unflag")
        : h("button", { class: "secondary", onclick: guard(async () => {
            const c = prompt("Why is it out of date? (e.g. new upstream version)");
            if (!c) throw new Error("A comment is required to flag a package.");
            await api("POST", `${P}/flag`, { comment: c });
          }, "Flagged out-of-date.") }, "Flag out-of-date"),
      !info.Maintainer ? h("button", { class: "secondary", onclick: guard(() => api("POST", `${P}/adopt`), "Adopted.") }, "Adopt") : null,
      info.Maintainer === me.name ? h("button", { class: "danger", onclick: guard(async () => {
          if (!confirm(`Disown ${base}?`)) throw new Error("Cancelled.");
          await api("POST", `${P}/disown`);
        }, "Disowned.") }, "Disown") : null);
    out.push(h("div", { class: "card" }, h("h2", {}, "Actions"), actions));

    if (info.Maintainer === me.name) {
      const cm = h("form", { class: "inline", onsubmit: guard(() => api("PUT", `${P}/comaintainers`, { users: val(cm, "users").split(/[\s,]+/).filter(Boolean) }), "Co-maintainers saved.") },
        h("input", { name: "users", value: (info.CoMaintainers || []).join(", "), placeholder: "user1, user2", "aria-label": "Co-maintainers" }),
        h("button", {}, "Save co-maintainers"));
      const tr = h("form", { class: "inline", onsubmit: guard(() => api("POST", `${P}/owner`, val(tr, "kind") === "org" ? { org: val(tr, "target") } : { user: val(tr, "target") }), "Transferred.") },
        h("select", { name: "kind", "aria-label": "Transfer to" }, h("option", { value: "org" }, "organization"), h("option", { value: "user" }, "user")),
        h("input", { name: "target", placeholder: "name", required: true, "aria-label": "Target" }), h("button", {}, "Transfer"));
      out.push(h("div", { class: "card" }, h("h2", {}, "Maintenance"), cm, h("p", {}), tr));
    }

    const rq = h("form", { class: "stack", onsubmit: guard(() => api("POST", `${P}/requests`, { kind: val(rq, "kind"), merge_into: val(rq, "into") || null, comment: val(rq, "comment") }), "Request filed.") },
      h("select", { name: "kind", "aria-label": "Request type" }, ["orphan", "deletion", "merge"].map((k) => h("option", { value: k }, k))),
      h("input", { name: "into", placeholder: "merge into (merge requests only)", "aria-label": "Merge into" }),
      h("textarea", { name: "comment", placeholder: "Reason", required: true, "aria-label": "Reason" }), h("button", {}, "File request"));
    out.push(h("div", { class: "card" }, h("h2", {}, "File a request"), rq));
  }

  const cbox = h("div", { class: "card" }, h("h2", {}, `Comments (${comments.length})`));
  if (me) {
    const cf = h("form", { class: "stack", onsubmit: guard(() => api("POST", `/api/v1/packages/${enc(base)}/comments`, { content: val(cf, "content") }), "Comment added.") },
      h("textarea", { name: "content", required: true, placeholder: "Add a comment", "aria-label": "Comment" }), h("button", {}, "Comment"));
    cbox.append(cf);
  } else {
    cbox.append(h("p", { class: "muted" }, link("/login", "Log in"), " to vote, comment, flag or file requests."));
  }
  for (const c of comments) {
    const mine = me && c.author === me.name;
    const manager = me && (info.Maintainer === me.name || (info.CoMaintainers || []).includes(me.name) || me.is_admin);
    const canDelete = me && (mine || manager);
    const PIN = `/api/v1/comments/${c.id}/pin`;
    cbox.append(h("div", { class: "comment" },
      h("div", { class: "muted" }, `${c.pinned ? "📌 pinned · " : ""}${c.author || "deleted user"} · ${fmtTime(c.created_at)}${c.edited_at ? " (edited)" : ""}`),
      h("pre", {}, c.content),
      h("div", { class: "actions" },
        mine ? h("button", { class: "secondary", onclick: guard(async () => {
          const t = prompt("Edit comment", c.content);
          if (t === null) throw new Error("Cancelled.");
          await api("PATCH", `/api/v1/comments/${c.id}`, { content: t });
        }, "Comment updated.") }, "Edit") : null,
        manager ? h("button", { class: "secondary", onclick: guard(() => api(c.pinned ? "DELETE" : "POST", PIN), c.pinned ? "Comment unpinned." : "Comment pinned.") }, c.pinned ? "Unpin" : "Pin") : null,
        canDelete ? h("button", { class: "secondary", onclick: guard(() => api("DELETE", `/api/v1/comments/${c.id}`), "Comment deleted.") }, "Delete") : null)));
  }
  out.push(cbox);
  render(...out);
}

function maintainerLink(name) {
  return name ? link(`/users/${enc(name)}`, name) : "orphan";
}
function peopleLinks(names) {
  if (!names || !names.length) return "—";
  const out = [];
  names.forEach((n, i) => { if (i) out.push(", "); out.push(link(`/users/${enc(n)}`, n)); });
  return out;
}
function pkgTable(rows) {
  if (!rows.length) return h("p", { class: "muted" }, "None.");
  return h("table", {}, h("thead", {}, h("tr", {}, ["Name", "Version", "Votes", "Description"].map((t) => h("th", {}, t)))),
    h("tbody", {}, rows.map((p) => h("tr", {},
      h("td", {}, link(`/packages/${enc(p.Name)}`, p.Name)),
      h("td", { class: p.OutOfDate ? "flagged" : "" }, p.Version),
      h("td", {}, p.NumVotes), h("td", {}, p.Description || "")))));
}
async function pageUser(name) {
  const load = async (by) => {
    try {
      const r = await api("GET", `/rpc/v5/search/${enc(name)}?by=${enc(by)}`, undefined, false);
      return r.type === "error" ? [] : (r.results || []);
    } catch (e) { return []; }
  };
  const [maintained, co] = await Promise.all([load("maintainer"), load("comaintainers")]);
  render(h("h1", {}, name),
    h("h2", {}, "Maintains"), h("div", { class: "card" }, pkgTable(maintained)),
    h("h2", {}, "Co-maintains"), h("div", { class: "card" }, pkgTable(co)));
}
function pageLogin() {
  const f = h("form", { class: "stack", onsubmit: async (e) => {
    e.preventDefault();
    try {
      const s = await api("POST", "/api/v1/login", { name: val(f, "name"), password: val(f, "password") }, false);
      localStorage.setItem(TOKEN_KEY, s.token);
      await loadMe();
      go("/account");
    } catch (err) { flash(err.message, "error"); }
  } },
    h("input", { name: "name", placeholder: "User name", autocomplete: "username", required: true, "aria-label": "User name" }),
    h("input", { name: "password", type: "password", placeholder: "Password", autocomplete: "current-password", required: true, "aria-label": "Password" }),
    h("button", {}, "Log in"));
  render(h("h1", {}, "Log in"), h("div", { class: "card" }, f, h("p", {}, link("/reset", "Forgot your password?"), " · ", link("/register", "Create an account"))));
}

function pageRegister() {
  const f = h("form", { class: "stack", onsubmit: async (e) => {
    e.preventDefault();
    if (val(f, "password") !== val(f, "password2")) return flash("Passwords do not match.", "error");
    try {
      await api("POST", "/api/v1/register", { name: val(f, "name"), email: val(f, "email"), password: val(f, "password") }, false);
      render(h("h1", {}, "Check your email"), h("div", { class: "card" }, h("p", {}, "We sent a verification link to your address. Open it to activate your account, then ", link("/login", "log in"), ".")));
    } catch (err) { flash(err.message, "error"); }
  } },
    h("input", { name: "name", placeholder: "User name (3-16 chars)", autocomplete: "username", required: true, "aria-label": "User name" }),
    h("input", { name: "email", type: "email", placeholder: "Email", autocomplete: "email", required: true, "aria-label": "Email" }),
    h("input", { name: "password", type: "password", placeholder: "Password (8+ chars)", autocomplete: "new-password", required: true, "aria-label": "Password" }),
    h("input", { name: "password2", type: "password", placeholder: "Repeat password", autocomplete: "new-password", required: true, "aria-label": "Repeat password" }),
    h("button", {}, "Register"));
  render(h("h1", {}, "Register"), h("div", { class: "card" }, f));
}

async function pageVerify(params) {
  const t = params.get("token");
  if (!t) return render(msg("This verification link is missing its token."));
  render(h("p", { class: "muted" }, "Verifying…"));
  try {
    const u = await api("POST", "/api/v1/verify", { token: t }, false);
    history.replaceState(null, "", "/verify");
    await loadMe(); nav();
    render(h("h1", {}, "Email verified"), h("div", { class: "card" }, h("p", {}, `Thanks, ${u.name}. Your account is active.`), me ? link("/account", "Go to your account") : link("/login", "Log in")));
  } catch (e) { render(h("h1", {}, "Verification failed"), msg(e.message), h("p", {}, "Log in and request a new link from your account page.")); }
}

function pageReset(params) {
  const t = params.get("token");
  if (t) {
    const f = h("form", { class: "stack", onsubmit: async (e) => {
      e.preventDefault();
      if (val(f, "password") !== val(f, "password2")) return flash("Passwords do not match.", "error");
      try {
        await api("POST", "/api/v1/password-reset/confirm", { token: t, password: val(f, "password") }, false);
        localStorage.removeItem(TOKEN_KEY); me = null; nav();
        history.replaceState(null, "", "/reset");
        render(h("h1", {}, "Password changed"), h("div", { class: "card" }, h("p", {}, "All sessions were signed out. ", link("/login", "Log in"), " with your new password.")));
      } catch (err) { flash(err.message, "error"); }
    } },
      h("input", { name: "password", type: "password", placeholder: "New password (8+ chars)", autocomplete: "new-password", required: true, "aria-label": "New password" }),
      h("input", { name: "password2", type: "password", placeholder: "Repeat new password", autocomplete: "new-password", required: true, "aria-label": "Repeat new password" }),
      h("button", {}, "Set new password"));
    return render(h("h1", {}, "Choose a new password"), h("div", { class: "card" }, f));
  }
  const f = h("form", { class: "stack", onsubmit: async (e) => {
    e.preventDefault();
    try {
      await api("POST", "/api/v1/password-reset", { email: val(f, "email") }, false);
      render(h("h1", {}, "Check your email"), h("div", { class: "card" }, h("p", {}, "If that address has an account, a reset link is on its way. It expires in one hour.")));
    } catch (err) { flash(err.message, "error"); }
  } },
    h("input", { name: "email", type: "email", placeholder: "Email", required: true, "aria-label": "Email" }), h("button", {}, "Send reset link"));
  render(h("h1", {}, "Reset password"), h("div", { class: "card" }, f));
}

async function pageAccount() {
  if (!me) return needLogin();
  const [keys, pkgs] = await Promise.all([api("GET", "/api/v1/ssh-keys").catch(() => []), api("GET", "/api/v1/packages").catch(() => [])]);
  const out = [h("h1", {}, me.name)];
  out.push(h("div", { class: "card" }, h("dl", {}, h("dt", {}, "Email"), h("dd", {}, me.email), h("dt", {}, "Status"),
    h("dd", {}, me.verified ? "verified" : h("span", { class: "flagged" }, "unverified — you can't push or manage packages yet "))),
    me.verified ? null : h("button", { onclick: guard(() => api("POST", "/api/v1/verify/resend"), "Verification email sent.") }, "Resend verification email")));
  out.push(h("div", { class: "card" }, h("h2", {}, "Your packages"),
    pkgs.length ? h("table", {}, h("tr", {}, h("th", {}, "Package base"), h("th", {}, "Maintainer"), h("th", {}, "Organization")),
      pkgs.map((p) => h("tr", {}, h("td", {}, link(`/packages/${enc(p.pkgbase)}`, p.pkgbase)), h("td", {}, p.maintainer || "orphan"), h("td", {}, p.org || "—"))))
      : h("p", { class: "muted" }, "None yet. Push one with git (see the docs)."),
  ));
  const kf = h("form", { class: "stack", onsubmit: guard(() => api("POST", "/api/v1/ssh-keys", { public_key: val(kf, "key") }), "Key added.") },
    h("textarea", { name: "key", placeholder: "ssh-ed25519 AAAA… you@host", required: true, "aria-label": "Public key" }), h("button", {}, "Add SSH key"));
  out.push(h("div", { class: "card" }, h("h2", {}, "SSH keys"),
    keys.length ? h("table", {}, keys.map((k) => h("tr", {}, h("td", {}, h("code", {}, k.fingerprint)), h("td", {}, k.algorithm), h("td", {}, k.comment),
      h("td", {}, h("button", { class: "secondary", onclick: guard(() => api("DELETE", `/api/v1/ssh-keys/${k.id}`), "Key removed.") }, "Remove"))))) : h("p", { class: "muted" }, "No keys."),
    kf));
  const df = h("form", { class: "stack", onsubmit: async (e) => {
    e.preventDefault();
    if (!confirm("Delete your account permanently? Packages you maintain become orphans.")) return;
    try {
      await api("DELETE", "/api/v1/account", { password: val(df, "password") });
      localStorage.removeItem(TOKEN_KEY); me = null; go("/");
    } catch (err) { flash(err.message, "error"); }
  } },
    h("input", { name: "password", type: "password", placeholder: "Current password", autocomplete: "current-password", required: true, "aria-label": "Current password" }),
    h("button", { class: "danger" }, "Delete account"));
  out.push(h("div", { class: "card" }, h("h2", {}, "Delete account"), df));
  render(...out);
}

async function pageOrgs() {
  if (!me) return needLogin();
  const orgs = await api("GET", "/api/v1/orgs").catch(() => []);
  const f = h("form", { class: "inline", onsubmit: guard(async () => { const n = val(f, "name"); await api("POST", "/api/v1/orgs", { name: n }); }, "Organization created.") },
    h("input", { name: "name", placeholder: "new organization", required: true, "aria-label": "Organization name" }), h("button", {}, "Create"));
  render(h("h1", {}, "Organizations"), h("div", { class: "card" },
    orgs.length ? h("table", {}, orgs.map((o) => h("tr", {}, h("td", {}, link(`/orgs/${enc(o.name)}`, o.name)), h("td", {}, o.role)))) : h("p", { class: "muted" }, "You aren't in any organization."),
    h("p", {}), f));
}

async function pageOrg(name) {
  if (!me) return needLogin();
  let org;
  try { org = await api("GET", `/api/v1/orgs/${enc(name)}`); } catch (e) { return render(msg(e.message)); }
  const M = (u) => `/api/v1/orgs/${enc(name)}/members/${enc(u)}`;
  const roles = ["owner", "admin", "contributor"];
  const myRole = (org.members.find((m) => m.name === me.name) || {}).role;
  const canManage = myRole === "owner" || myRole === "admin";
  const rows = org.members.map((m) => h("tr", {}, h("td", {}, m.name),
    h("td", {}, canManage ? h("select", { "aria-label": `Role of ${m.name}`, onchange: guard((e) => api("PUT", M(m.name), { role: e.target.value }), "Role updated.") },
      roles.map((r) => h("option", { value: r, selected: r === m.role }, r))) : m.role),
    h("td", {}, canManage ? h("button", { class: "secondary", onclick: guard(() => api("DELETE", M(m.name)), "Member removed.") }, "Remove") : null)));
  const add = h("form", { class: "inline", onsubmit: guard(() => api("PUT", M(val(add, "user")), { role: val(add, "role") }), "Member added.") },
    h("input", { name: "user", placeholder: "user name", required: true, "aria-label": "User name" }),
    h("select", { name: "role", "aria-label": "Role" }, ["contributor", "admin", "owner"].map((r) => h("option", { value: r }, r))), h("button", {}, "Add member"));
  const out = [h("h1", {}, org.name), h("div", { class: "card" }, h("h2", {}, "Members"), h("table", {}, rows), canManage ? [h("p", {}), add] : null)];
  if (myRole === "owner") {
    out.push(h("div", { class: "card" }, h("h2", {}, "Danger zone"), h("button", { class: "danger", onclick: async () => {
      if (!confirm(`Delete organization ${name}?`)) return;
      try { await api("DELETE", `/api/v1/orgs/${enc(name)}`); go("/orgs"); } catch (e) { flash(e.message, "error"); }
    } }, "Delete organization")));
  }
  render(...out);
}

async function pageRequests(params) {
  if (!me) return needLogin();
  const all = params.get("all") === "1";
  let reqs;
  try { reqs = await api("GET", `/api/v1/requests${all ? "?all=true" : ""}`); } catch (e) { return render(msg(e.message)); }
  const act = (id, what) => guard(async () => { const c = prompt(`Comment for ${what}`, "") ?? ""; await api("POST", `/api/v1/requests/${id}/${what}`, { comment: c }); }, `Request ${what}ed.`);
  render(h("h1", {}, "Requests"), h("p", {}, all ? link("/requests", "Show pending only") : link("/requests?all=1", "Show all")),
    h("div", { class: "card" }, reqs.length ? h("table", {},
      h("tr", {}, ["#", "Type", "Package", "By", "Status", "Comment", ""].map((t) => h("th", {}, t))),
      reqs.map((r) => h("tr", {}, h("td", {}, r.id), h("td", {}, r.kind + (r.merge_into ? ` → ${r.merge_into}` : "")),
        h("td", {}, link(`/packages/${enc(r.pkgbase)}`, r.pkgbase)), h("td", {}, r.requester || "—"), h("td", {}, r.status), h("td", {}, r.comments),
        h("td", {}, r.status === "pending" ? h("div", { class: "actions" },
          h("button", { onclick: act(r.id, "accept") }, "Accept"),
          h("button", { class: "secondary", onclick: act(r.id, "reject") }, r.requester === me.name ? "Withdraw" : "Reject")) : null))))
      : h("p", { class: "muted" }, "No requests.")));
}

// ---- router ----
async function route() {
  const path = decodeURIComponent(location.pathname);
  const params = new URLSearchParams(location.search);
  nav();
  let m;
  if (path === "/" || path === "/packages" || path === "/packages/") return pageSearch(params);
  if ((m = path.match(/^\/packages\/([^/]+)\/?$/))) return pagePackage(m[1]);
  if ((m = path.match(/^\/users\/([^/]+)\/?$/))) return pageUser(m[1]);
  if (path === "/login") return pageLogin();
  if (path === "/register") return pageRegister();
  if (path === "/verify") return pageVerify(params);
  if (path === "/reset") return pageReset(params);
  if (path === "/account") return pageAccount();
  if (path === "/orgs") return pageOrgs();
  if ((m = path.match(/^\/orgs\/([^/]+)\/?$/))) return pageOrg(m[1]);
  if (path === "/requests") return pageRequests(params);
  render(msg("Page not found."));
}

function go(url) { history.pushState(null, "", url); route(); }

document.addEventListener("click", (e) => {
  const a = e.target.closest("a[data-link]");
  if (a && !e.metaKey && !e.ctrlKey && a.origin === location.origin) { e.preventDefault(); go(a.getAttribute("href")); }
});
window.addEventListener("popstate", route);
loadMe().then(route);
