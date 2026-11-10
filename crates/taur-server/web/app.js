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
function render(...nodes) {
  // replaceChildren stringifies null/undefined/false to "null"/"undefined"/"false".
  // Skip those placeholders (same rule as h()); keep 0 and "".
  app().replaceChildren(...nodes.flat(Infinity).filter((n) => n !== null && n !== undefined && n !== false));
}
function msg(text, kind = "error") { return h("div", { class: `msg ${kind}`, role: "status" }, text); }
function flash(text, kind = "ok") { app().prepend(msg(text, kind)); window.scrollTo(0, 0); }
const fmtTime = (t) => (t ? new Date(t * 1000).toLocaleString() : "—");
const val = (form, name) => form.elements[name].value;

// ---- API ----
async function api(method, path, body, auth = true) {
  const headers = {};
  if (body !== undefined) headers["Content-Type"] = "application/json";
  if (auth && token()) {
    headers["Authorization"] = `Bearer ${token()}`;
    if (method !== "GET" && method !== "HEAD") headers["X-CSRF-Token"] = token();
  }
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
  const items = [link("/", "Packages"), link("/orgs", "Organizations"), link("/docs", "Docs"), link("/requests", "Requests")];
  if (me) {
    items.push(link("/upload", "Upload"), link("/account", me.name));
    if (me.is_admin || me.is_moderator) items.push(link("/admin", "Admin"));
    if (me.is_admin) items.push(link("/audit", "Audit"), link("/mirrors", "Mirrors"));
    if (me.is_owner) items.push(link("/registries", "Registries"));
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

function indexHref(sort, page) {
  const p = new URLSearchParams();
  if (sort) p.set("sort", sort);
  if (page && Number(page) > 1) p.set("page", String(page));
  const s = p.toString();
  return s ? `/?${s}` : "/";
}

function pager(page, total, per, hrefFor) {
  const pages = Math.max(1, Math.ceil((total || 0) / (per || 20)));
  const prev = page > 1 ? link(hrefFor(page - 1), "Previous") : h("span", { class: "muted" }, "Previous");
  const next = page < pages ? link(hrefFor(page + 1), "Next") : h("span", { class: "muted" }, "Next");
  return h("nav", { class: "pager", "aria-label": "Pagination" }, prev, h("span", {}, `Page ${page} of ${pages}`), next);
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
  } else {
    const requested = params.get("sort") || "updated";
    const page = Number(params.get("page") || "1");
    try {
      const r = await api("GET", `/api/v1/packages/index?sort=${enc(requested)}&page=${enc(String(page))}`, undefined, false);
      const sortForm = h("form", { class: "inline", onsubmit: (e) => { e.preventDefault(); go(indexHref(val(sortForm, "sort"), 1)); } },
        h("label", {}, "Sort by ",
          h("select", { name: "sort", "aria-label": "Sort packages", onchange: () => sortForm.requestSubmit() },
            [["updated", "Updated at"], ["downloaded", "Downloaded"], ["popularity", "Popularity"]].map(([v, lab]) =>
              h("option", { value: v, selected: v === requested }, lab)))));
      const note = h("p", { class: "muted" }, "Downloads count successful snapshot and recipe-archive responses (current and historic). They are not installs or unique users.");
      const rows = r.packages || [];
      const table = rows.length
        ? h("table", {}, h("thead", {}, h("tr", {}, ["Name", "Version", "Votes", "Downloads", "Popularity", "Description", "Updated"].map((x) => h("th", {}, x)))),
            h("tbody", {}, rows.map((pkg) => h("tr", {},
              h("td", {}, link(`/packages/${enc(pkg.Name)}`, pkg.Name)),
              h("td", { class: pkg.OutOfDate ? "flagged" : "" }, pkg.Version),
              h("td", {}, pkg.NumVotes),
              h("td", {}, pkg.DownloadCount ?? 0),
              h("td", {}, Number(pkg.Popularity || 0).toFixed(2)),
              h("td", {}, pkg.Description || ""),
              h("td", {}, fmtTime(pkg.LastModified))))))
        : h("p", { class: "muted" }, "No packages yet.");
      out.push(h("div", { class: "card" }, sortForm, note, table,
        rows.length ? pager(r.page, r.total, r.per_page, (n) => indexHref(requested, n)) : null));
    } catch (e) { out.push(msg(e.message)); }
  }
  render(...out);
}

async function pagePackage(name) {
  let info;
  try { info = await api("GET", `/api/v1/packages/by-name/${enc(name)}`, undefined, !!token()); }
  catch (e) { return render(msg(e.message)); }
  if (!info || !info.Name) return render(msg(`No package named ${name}.`));
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
    ["Organization", info.Organization ? link(`/orgs/${enc(info.Organization)}`, info.Organization) : "—"], ["Submitter", info.Submitter || "—"],
    ["Votes", info.NumVotes],
    ["Downloads", info.DownloadCount ?? 0],
    ["Popularity", info.Popularity.toFixed(2)],
    ["First submitted", fmtTime(info.FirstSubmitted)], ["Last updated", fmtTime(info.LastModified)],
    ["Out of date", info.OutOfDate ? h("span", { class: "flagged" }, `since ${fmtTime(info.OutOfDate)} by ${flag.flagger || "?"}: ${flag.comment || ""}`) : "no"],
  ];
  const dl = h("dl", {}, rows.map(([k, v]) => [h("dt", {}, k), h("dd", {}, v)]));
  const out = [
    info.Archived ? h("p", { class: "flagged" }, `Archived since ${fmtTime(info.Archived)}. It is hidden from search and install. The git history is kept.`) : null,
    h("h1", {}, `${info.Name} ${info.Version}`),
    h("div", { class: "card" }, dl, h("p", { class: "muted" }, "Downloads count successful snapshot and recipe-archive responses for this package base, including historic revisions. They are not a count of installed binaries or unique users.")),
  ];

  if (me) {
    const P = `/api/v1/packages/${enc(base)}`;
    const actions = h("div", { class: "actions" },
      voted ? h("button", { class: "secondary", onclick: guard(() => api("DELETE", `${P}/vote`), "Vote removed.") }, "Remove vote")
            : h("button", { onclick: guard(() => api("POST", `${P}/vote`), "Thanks for voting.") }, "Vote"),
      subscribed ? h("button", { class: "secondary", onclick: guard(() => api("DELETE", `${P}/notify`), "Notifications off.") }, "Disable notifications")
                 : h("button", { class: "secondary", onclick: guard(() => api("POST", `${P}/notify`), "You will be emailed about updates and comments.") }, "Enable notifications"),
      info.OutOfDate && (info.CanManage || (flag.flagger === me.name))
        ? h("button", { class: "secondary", onclick: guard(() => api("DELETE", `${P}/flag`), info.CanManage ? "Out-of-date flag cleared." : "Flag removed.") }, info.CanManage ? "Clear out-of-date flag" : "Remove your flag")
        : !info.OutOfDate ? h("button", { class: "secondary", onclick: guard(async () => {
            const c = prompt(info.CanManage ? "Mark this package out of date. What changed upstream?" : "Why is it out of date? (e.g. new upstream version)");
            if (!c) throw new Error("A comment is required to flag a package.");
            await api("POST", `${P}/flag`, { comment: c });
          }, info.CanManage ? "Marked out of date." : "Flagged out-of-date.") }, info.CanManage ? "Mark out of date" : "Flag out-of-date") : null,
      info.CanManage ? (info.Archived
        ? h("button", { onclick: guard(() => api("DELETE", `${P}/archive`), "Package restored.") }, "Unarchive")
        : h("button", { class: "secondary", onclick: confirmAct(`Archive ${base}? It leaves search and install. The git history stays.`, () => api("POST", `${P}/archive`), "Package archived.") }, "Archive")) : null,
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
      h("select", { name: "kind", "aria-label": "Request type" }, ["orphan", "deletion", "merge", "adoption"].map((k) => h("option", { value: k }, k))),
      h("input", { name: "into", placeholder: "merge into (merge requests only)", "aria-label": "Merge into" }),
      h("textarea", { name: "comment", placeholder: "Reason", required: true, "aria-label": "Reason" }), h("button", {}, "File request"));
    out.push(h("div", { class: "card" }, h("h2", {}, "File a request"), rq));
    if (me.is_admin || me.is_moderator) out.push(rejectCard("package", base));
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
  let revisions = [];
  try { revisions = await api("GET", `/api/v1/packages/${enc(base)}/revisions`, undefined, false); } catch { revisions = []; }
  const previous = Array.isArray(revisions) ? revisions.filter((rev) => !rev.current) : [];
  const hist = previous.length
    ? h("ul", { class: "versions" }, previous.map((rev) => h("li", {},
        link(`/packages/${enc(info.Name)}/revisions/${enc(rev.id)}`, `${rev.version || "recipe"} (${rev.id.slice(0, 7)})`),
        ` · ${fmtTime(rev.committed_at)}`)))
    : h("p", { class: "muted" }, "No previous versions.");
  out.push(h("div", { class: "card" }, h("h2", {}, "Previous versions"), hist));
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
  if (!rows.length) return h("p", { class: "muted" }, "None. ", link("/upload", "Upload a package"), " or read ", link("/docs/publish", "how to publish"), ".");
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
  const blocks = [h("h1", {}, name),
    h("h2", {}, "Maintains"), h("div", { class: "card" }, pkgTable(maintained)),
    h("h2", {}, "Co-maintains"), h("div", { class: "card" }, pkgTable(co))];
  if (me && (me.is_admin || me.is_moderator)) blocks.push(rejectCard("user", name));
  if (me && me.is_admin && me.name !== name) blocks.push(adminUserCard(name));
  render(...blocks);
}
function confirmAct(question, fn, okText) {
  return guard(async () => {
    if (!confirm(question)) throw new Error("Cancelled.");
    await fn();
  }, okText);
}
function rejectCard(kind, name) {
  const path = kind === "user"
    ? `/api/v1/moderation/users/${enc(name)}/reject`
    : `/api/v1/moderation/packages/${enc(name)}/reject`;
  const label = kind === "user" ? "Reject user" : "Reject package";
  const f = h("form", { class: "stack", onsubmit: confirmAct(`Reject ${kind} ${name}? It disappears from search and, for an account, existing sessions end.`, () => api("POST", path, { reason: val(f, "reason") }), "Rejected.") },
    h("textarea", { name: "reason", required: true, placeholder: "Reason", "aria-label": "Reason" }),
    h("button", { class: "danger" }, label));
  return h("div", { class: "card" }, h("h2", {}, "Moderation"), f);
}
function adminUserCard(name) {
  const base = `/api/v1/admin/users/${enc(name)}`;
  return h("div", { class: "card" }, h("h2", {}, "Site admin"),
    h("div", { class: "actions" },
      h("button", { class: "secondary", onclick: guard(() => api("POST", `${base}/verify`), "Marked verified.") }, "Verify email"),
      h("button", { class: "secondary", onclick: guard(() => api("POST", `${base}/resend`), "Verification email sent.") }, "Resend verification"),
      h("button", { class: "secondary", onclick: confirmAct(`Email ${name} a password reset link?`, () => api("POST", `${base}/password-reset`), "Reset link sent.") }, "Send password reset"),
      h("button", { onclick: guard(() => api("PUT", `/api/v1/admin/moderators/${enc(name)}`, { on: true }), "Moderator granted.") }, "Grant moderator"),
      h("button", { class: "secondary", onclick: confirmAct(`Remove moderator from ${name}?`, () => api("PUT", `/api/v1/admin/moderators/${enc(name)}`, { on: false }), "Moderator removed.") }, "Remove moderator"),
      me.is_owner ? h("button", { onclick: guard(() => api("PUT", `${base}/admin`, { on: true }), "Site admin granted.") }, "Grant site admin") : null,
      me.is_owner ? h("button", { class: "danger", onclick: confirmAct(`Remove site admin from ${name}?`, () => api("PUT", `${base}/admin`, { on: false }), "Site admin removed.") }, "Remove site admin") : null,
      me.is_owner ? h("button", { onclick: guard(() => api("PUT", `${base}/owner`, { on: true }), "Site owner granted.") }, "Grant site owner") : null,
      me.is_owner ? h("button", { class: "danger", onclick: confirmAct(`Remove site owner from ${name}? They stay a site admin.`, () => api("PUT", `${base}/owner`, { on: false }), "Site owner removed.") }, "Remove site owner") : null,
      h("button", { class: "danger", onclick: confirmAct(`Sign ${name} out of every session?`, () => api("DELETE", `${base}/sessions`), "Sessions revoked.") }, "Revoke sessions")));
}
async function pageAudit() {
  if (!me || !me.is_admin) { render(msg("Site admins audit rejections.")); return; }
  const rows = await api("GET", "/api/v1/audit");
  const revoked = new Set((rows || []).filter((e) => e.action === "revoke").map((e) => e.reverses));
  render(h("h1", {}, "Audit"),
    h("p", { class: "muted" }, "Rejections, acceptances, and reversals. Accept lets that same rejected upload or registration be submitted again. It does not create an account or store a recipe. Reverse clears a package or user suspension and does not permit a rejected upload or registration. A rejected request stays closed."),
    h("table", {}, h("thead", {}, h("tr", {}, ["When", "Action", "Target", "Reason", "Actor", "Source", ""].map((t) => h("th", {}, t)))),
      h("tbody", {}, (rows || []).map((e) => h("tr", {},
        h("td", {}, fmtTime(e.at)),
        h("td", {}, e.action),
        h("td", {}, `${e.target_kind} ${e.target}`),
        h("td", {}, e.reason),
        h("td", {}, e.actor || "—"),
        h("td", {}, e.source),
        h("td", {}, e.open && (e.target_kind === "registry_upload" || e.target_kind === "registration") ? h("button", { onclick: guard(async () => {
          const reason = prompt("Why accept this rejected attempt? The same upload or registration can then be retried.");
          if (!reason) throw new Error("A reason is required.");
          await api("POST", `/api/v1/audit/${e.id}/accept`, { reason });
        }, "Accepted.") }, "Accept") : e.open && e.action === "reject" && e.target_kind !== "request" ? h("button", { onclick: guard(async () => {
          const reason = prompt("Why reverse this rejection?");
          if (!reason) throw new Error("A reason is required.");
          await api("POST", `/api/v1/audit/${e.id}/reverse`, { reason });
        }, "Reversed.") }, "Reverse") : e.action === "accept" && !revoked.has(e.id) ? h("button", { class: "danger", onclick: guard(async () => {
          if (!confirm("Revoke this acceptance? The original rule applies again.")) throw new Error("Cancelled.");
          const reason = prompt("Why revoke this acceptance?");
          if (!reason) throw new Error("A reason is required.");
          await api("POST", `/api/v1/audit/${e.id}/revoke`, { reason });
        }, "Revoked.") }, "Revoke") : null))))));
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
      : h("p", { class: "muted" }, "None yet. ", link("/upload", "Upload a package"), " or ", link("/docs/publish", "publish with git"), "."),
  ));
  const kf = h("form", { class: "stack", onsubmit: guard(() => api("POST", "/api/v1/ssh-keys", { public_key: val(kf, "key") }), "Key added.") },
    h("textarea", { name: "key", placeholder: "ssh-ed25519 AAAA… you@host", required: true, "aria-label": "Public key" }), h("button", {}, "Add SSH key"));
  out.push(h("div", { class: "card" }, h("h2", {}, "SSH keys"),
    keys.length ? h("table", {}, keys.map((k) => h("tr", {}, h("td", {}, h("code", {}, k.fingerprint)), h("td", {}, k.algorithm), h("td", {}, k.comment),
      h("td", {}, h("button", { class: "secondary", onclick: confirmAct("Remove this SSH key?", () => api("DELETE", `/api/v1/ssh-keys/${k.id}`), "Key removed.") }, "Remove"))))) : h("p", { class: "muted" }, "No keys."),
    kf));
  const sessions = await api("GET", "/api/v1/sessions").catch(() => []);
  out.push(h("div", { class: "card" }, h("h2", {}, "Sessions"),
    sessions.length ? h("table", {}, sessions.map((s) => h("tr", {},
      h("td", {}, fmtTime(s.created_at)),
      h("td", {}, s.current ? "this session" : "other"),
      h("td", {}, h("button", { class: "secondary", onclick: confirmAct(s.current ? "Sign out this session?" : "Revoke this session?", () => api("DELETE", `/api/v1/sessions/${s.id}`), "Session revoked.") }, "Revoke")))))
      : h("p", { class: "muted" }, "No sessions.")));
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

async function pageOrgs(params) {
  const q = (params && params.get("q")) || "";
  let dir = { organizations: [] };
  try { dir = await api("GET", `/api/v1/organizations?q=${enc(q)}`, undefined, false); } catch (e) { return render(msg(e.message)); }
  const orgs = dir.organizations || [];
  const filter = h("form", { class: "inline", onsubmit: (e) => { e.preventDefault(); go(`/orgs?q=${enc(val(filter, "q"))}`); } },
    h("input", { name: "q", value: q, placeholder: "Search organizations", "aria-label": "Search organizations" }),
    h("button", {}, "Search"));
  const list = orgs.length
    ? h("table", {}, h("thead", {}, h("tr", {}, h("th", {}, "Organization"), h("th", {}, "Packages"))),
        h("tbody", {}, orgs.map((o) => h("tr", {},
          h("td", {}, link(`/orgs/${enc(o.name)}`, o.name)),
          h("td", {}, String(o.package_count))))))
    : h("p", { class: "muted" }, q ? "No organizations match that name." : "No organizations yet.");
  const out = [h("h1", {}, "Organizations"), h("div", { class: "card" }, filter, list)];
  if (me) {
    const mine = await api("GET", "/api/v1/orgs").catch(() => []);
    const f = h("form", { class: "inline", onsubmit: guard(async () => { const n = val(f, "name"); await api("POST", "/api/v1/orgs", { name: n }); }, "Organization created.") },
      h("input", { name: "name", placeholder: "new organization", required: true, "aria-label": "Organization name" }), h("button", {}, "Create"));
    out.push(h("div", { class: "card" }, h("h2", {}, "Your organizations"),
      mine.length ? h("table", {}, mine.map((o) => h("tr", {}, h("td", {}, link(`/orgs/${enc(o.name)}`, o.name)), h("td", {}, o.role)))) : h("p", { class: "muted" }, "You aren't in any organization."),
      h("p", {}), f));
  }
  render(...out);
}

async function pageOrg(name) {
  let profile;
  try { profile = await api("GET", `/api/v1/organizations/${enc(name)}`, undefined, false); }
  catch (e) { return render(h("h1", {}, "Organization not found"), msg(e.message)); }
  const pkgs = profile.packages || [];
  const pkgList = pkgs.length
    ? h("table", {}, h("thead", {}, h("tr", {}, ["Name", "Version", "Description"].map((x) => h("th", {}, x)))),
        h("tbody", {}, pkgs.map((pkg) => h("tr", {},
          h("td", {}, link(`/packages/${enc(pkg.Name)}`, pkg.Name)),
          h("td", {}, pkg.Version),
          h("td", {}, pkg.Description || "")))))
    : h("p", { class: "muted" }, "This organization has no public packages.");
  const out = [
    h("p", {}, link("/orgs", "Organizations")),
    h("h1", {}, profile.name),
    h("p", { class: "muted" }, `Created ${fmtTime(profile.created_at)}`),
    h("div", { class: "card" }, h("h2", {}, "Packages"), pkgList),
  ];
  if (!me) return render(...out);
  let org;
  try { org = await api("GET", `/api/v1/orgs/${enc(name)}`); } catch (e) { return render(...out); }
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
  out.push(h("div", { class: "card" }, h("h2", {}, "Members"), h("table", {}, rows), canManage ? [h("p", {}), add] : null));
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

async function pageAdmin(params) {
  if (!me || (!me.is_admin && !me.is_moderator)) return render(msg("Moderators and site admins can search accounts."));
  const q = params.get("q") || "";
  const f = h("form", { class: "inline", onsubmit: (e) => { e.preventDefault(); go(`/admin?q=${enc(val(f, "q"))}`); } },
    h("input", { name: "q", value: q, placeholder: "User name", "aria-label": "User name" }),
    h("button", {}, "Search"));
  let rows = [];
  try { rows = await api("GET", `/api/v1/admin/users?q=${enc(q)}`); }
  catch (e) { return render(msg(e.message)); }
  render(h("h1", {}, "Users"),
    h("p", { class: "muted" }, me.is_admin ? "Site admin. Email addresses are visible to you." : "Moderator. You can reject an ordinary account. You cannot grant roles, accept requests, or see email addresses."),
    h("div", { class: "card" }, f, h("table", {},
      h("tr", {}, ["Name", "Email", "Status", "Roles"].map((t) => h("th", {}, t))),
      rows.map((u) => h("tr", {},
        h("td", {}, link(`/users/${enc(u.name)}`, u.name)),
        h("td", {}, u.email || "—"),
        h("td", {}, `${u.verified ? "verified" : "unverified"}${u.suspended ? ", suspended" : ""}`),
        h("td", {}, [u.is_admin ? "admin" : null, u.is_moderator ? "moderator" : null].filter(Boolean).join(", ") || "—"))))));
}

async function pageRegistries() {
  if (!me || !me.is_owner) return render(msg("Site owners manage package registries."));
  let rows = [];
  try { rows = await api("GET", "/api/v1/registries"); }
  catch (e) { return render(msg(e.message)); }
  const f = h("form", { class: "inline", onsubmit: guard(() => api("POST", "/api/v1/registries", { name: val(f, "name") }), "Registry created.") },
    h("input", { name: "name", placeholder: "registry name", required: true, "aria-label": "Registry name" }),
    h("button", {}, "Create enforcing registry"));
  render(h("h1", {}, "Registries"),
    h("p", { class: "muted" }, "Operator trust records. Signing a recipe and generating a key stay on the machine that holds the secret seed (`taur registry keygen` and `taur registry sign`)."),
    h("div", { class: "card" },
      rows.length ? h("table", {}, rows.map((r) => h("tr", {},
        h("td", {}, link(`/registries/${enc(r.name)}`, r.name)),
        h("td", {}, r.enforcement),
        h("td", {}, r.provenance)))) : h("p", { class: "muted" }, "Only the built-in local registry."),
      h("p", {}), f));
}

async function pageRegistry(name) {
  if (!me || !me.is_owner) return render(msg("Site owners manage package registries."));
  let trust;
  try { trust = await api("GET", `/api/v1/registries/${enc(name)}`); }
  catch (e) { return render(msg(e.message)); }
  const key = h("form", { class: "stack", onsubmit: guard(() => api("POST", `/api/v1/registries/${enc(name)}/keys`, { label: val(key, "label"), public_key_hex: val(key, "hex").trim() }), "Key added.") },
    h("input", { name: "label", placeholder: "label", required: true, "aria-label": "Key label" }),
    h("textarea", { name: "hex", placeholder: "32-byte Ed25519 public key, hex", required: true, "aria-label": "Public key hex" }),
    h("button", {}, "Add operator key"));
  const idn = h("form", { class: "inline", onsubmit: guard(() => api("POST", `/api/v1/registries/${enc(name)}/identities`, { nick: val(idn, "nick"), email: val(idn, "email") }), "Identity recorded.") },
    h("input", { name: "nick", placeholder: "nick", required: true, "aria-label": "Nick" }),
    h("input", { name: "email", type: "email", placeholder: "email", required: true, "aria-label": "Email" }),
    h("button", {}, "Bind identity"));
  const exp = h("form", { class: "stack", onsubmit: guard(() => api("POST", `/api/v1/registries/${enc(name)}/expectations`, {
    pkgbase: val(exp, "pkgbase"), repository: val(exp, "repository"), version: val(exp, "version"),
    signature_hex: val(exp, "signature").trim(), key_fingerprint: val(exp, "fingerprint").trim(), nick: val(exp, "nick"),
  }), "Expectation recorded.") },
    h("input", { name: "pkgbase", placeholder: "package base", required: true, "aria-label": "Package base" }),
    h("input", { name: "repository", placeholder: "repository", required: true, "aria-label": "Repository" }),
    h("input", { name: "version", placeholder: "version", required: true, "aria-label": "Version" }),
    h("input", { name: "fingerprint", placeholder: "key fingerprint", required: true, "aria-label": "Key fingerprint" }),
    h("input", { name: "nick", placeholder: "nick", required: true, "aria-label": "Nick" }),
    h("textarea", { name: "signature", placeholder: "64-byte signature, hex", required: true, "aria-label": "Signature hex" }),
    h("button", {}, "Record expected recipe"));
  const list = (title, rows, cells) => h("div", { class: "card" }, h("h2", {}, title),
    rows.length ? h("table", {}, rows.map((row) => h("tr", {}, cells(row).map((c) => h("td", {}, c))))) : h("p", { class: "muted" }, "None."));
  render(h("h1", {}, name),
    list("Keys", trust.keys, (k) => [k.label, k.fingerprint]),
    h("div", { class: "card" }, h("h2", {}, "Add key"), key),
    list("Identities", trust.identities, (i) => [i.nick, i.email, i.provenance]),
    h("div", { class: "card" }, h("h2", {}, "Bind identity"), idn),
    list("Expected recipes", trust.expectations, (e) => [e.pkgbase, e.repository, e.version, e.provenance]),
    h("div", { class: "card" }, h("h2", {}, "Expect a recipe"), exp));
}

async function pageMirrors() {
  if (!me || !me.is_admin) return render(msg("Site admins manage recipe mirrors."));
  let rows = [];
  try { rows = await api("GET", "/api/v1/mirrors"); }
  catch (e) { return render(msg(e.message)); }
  const f = h("form", { class: "inline", onsubmit: guard(() => api("POST", "/api/v1/mirrors", { name: val(f, "name"), url: val(f, "url") }), "Mirror added. Nothing has been fetched.") },
    h("input", { name: "name", placeholder: "name", required: true, "aria-label": "Mirror name" }),
    h("input", { name: "url", placeholder: "https://example.com", required: true, "aria-label": "Base URL" }),
    h("button", {}, "Add mirror"));
  render(h("h1", {}, "Mirrors"),
    h("p", { class: "muted" }, "Copies recipes from an AUR-style Git source. Not a pacman repository and not an operator signature. Nothing is fetched until you sync."),
    h("div", { class: "card" },
      rows.length ? h("table", {}, rows.map((m) => h("tr", {},
        h("td", {}, link(`/mirrors/${enc(m.name)}`, m.name)),
        h("td", {}, m.base_url),
        h("td", {}, m.provenance)))) : h("p", { class: "muted" }, "No mirrors."),
      h("p", {}), f));
}

async function pageMirror(name) {
  if (!me || !me.is_admin) return render(msg("Site admins manage recipe mirrors."));
  let rows;
  try { rows = await api("GET", `/api/v1/mirrors/${enc(name)}`); }
  catch (e) { return render(msg(e.message)); }
  const sel = h("form", { class: "inline", onsubmit: guard(() => api("POST", `/api/v1/mirrors/${enc(name)}/select`, { pkgbases: val(sel, "pkgs").split(/[\s,]+/).filter(Boolean) }), "Selected. Not fetched yet.") },
    h("input", { name: "pkgs", placeholder: "pkgbase pkgbase", required: true, "aria-label": "Package bases" }),
    h("button", {}, "Select"));
  render(h("h1", {}, name),
    h("div", { class: "card" }, h("h2", {}, "Selection"),
      rows.length ? h("table", {},
        h("tr", {}, ["Package", "Status", "Revision", "Stale", ""].map((t) => h("th", {}, t))),
        rows.map((p) => h("tr", {},
          h("td", {}, p.pkgbase),
          h("td", {}, p.status),
          h("td", {}, p.revision || "—"),
          h("td", {}, p.stale ? "yes" : "no"),
          h("td", {}, h("div", { class: "actions" },
            h("button", { class: "secondary", onclick: guard(() => api("POST", `/api/v1/mirrors/${enc(name)}/unselect`, { pkgbase: p.pkgbase }), "Unselected.") }, "Unselect"),
            h("button", { class: "danger", onclick: confirmAct(`Drop ${p.pkgbase} from this mirror?`, () => api("POST", `/api/v1/mirrors/${enc(name)}/drop`, { pkgbase: p.pkgbase }), "Dropped.") }, "Drop"))))))
        : h("p", { class: "muted" }, "Nothing selected."),
      h("p", {}), sel,
      h("div", { class: "actions" },
        h("button", { onclick: guard(() => api("POST", `/api/v1/mirrors/${enc(name)}/sync`), "Sync finished.") }, "Sync"),
        h("button", { class: "danger", onclick: confirmAct(`Remove mirror ${name}? Drop its packages first.`, () => api("DELETE", `/api/v1/mirrors/${enc(name)}`).then(() => go("/mirrors")), "Mirror removed.") }, "Remove mirror"))));
}

function inline(text) {
  const out = [];
  const re = /(`[^`]+`)|(\*\*[^*]+\*\*)|(\[[^\]]+\]\([^)\s]+\))/g;
  let last = 0;
  for (const m of text.matchAll(re)) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const tok = m[0];
    if (tok.startsWith("`")) out.push(h("code", {}, tok.slice(1, -1)));
    else if (tok.startsWith("**")) out.push(h("strong", {}, tok.slice(2, -2)));
    else {
      const label = tok.slice(1, tok.indexOf("]"));
      const href = tok.slice(tok.indexOf("(") + 1, -1);
      if (href.startsWith("/") || href.startsWith("https://") || href.startsWith("http://")) {
        out.push(h("a", { href, "data-link": href.startsWith("/"), rel: href.startsWith("/") ? null : "nofollow noopener" }, label));
      } else out.push(label);
    }
    last = m.index + tok.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}
function renderMarkdown(src) {
  const root = h("div", { class: "doc" });
  const lines = src.replace(/\r\n/g, "\n").split("\n");
  let i = 0;
  const blocks = (line) => line.startsWith("#") || line.startsWith("- ") || line.startsWith("```") || /^\d+\. /.test(line);
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim() === "") { i++; continue; }
    if (line.startsWith("```")) {
      const buf = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) { buf.push(lines[i]); i++; }
      if (i < lines.length) i++;
      root.append(h("pre", {}, h("code", {}, buf.join("\n"))));
      continue;
    }
    const heading = line.match(/^(#{1,3}) (.+)$/);
    if (heading) {
      root.append(h(`h${heading[1].length}`, {}, inline(heading[2])));
      i++;
      continue;
    }
    if (line.startsWith("- ")) {
      const ul = h("ul");
      while (i < lines.length && lines[i].startsWith("- ")) { ul.append(h("li", {}, inline(lines[i].slice(2)))); i++; }
      root.append(ul);
      continue;
    }
    if (/^\d+\. /.test(line)) {
      const ol = h("ol");
      while (i < lines.length && /^\d+\. /.test(lines[i])) { ol.append(h("li", {}, inline(lines[i].replace(/^\d+\. /, "")))); i++; }
      root.append(ol);
      continue;
    }
    const buf = [line];
    i++;
    while (i < lines.length && lines[i].trim() !== "" && !blocks(lines[i])) { buf.push(lines[i]); i++; }
    root.append(h("p", {}, inline(buf.join(" "))));
  }
  return root;
}
async function pageDocs() {
  let rows;
  try { rows = await api("GET", "/api/v1/docs", undefined, !!token()); }
  catch (e) { return render(msg(e.message)); }
  const lead = rows[0];
  const rest = rows.slice(1);
  render(h("h1", {}, "Docs"),
    lead ? h("div", { class: "card" }, h("h2", {}, link(`/docs/${enc(lead.slug)}`, lead.title)), h("p", {}, lead.summary)) : null,
    h("div", { class: "card" }, rest.length ? h("ul", {}, rest.map((g) => h("li", {}, link(`/docs/${enc(g.slug)}`, g.title), " — ", g.summary))) : h("p", { class: "muted" }, "No further guides for this account.")));
}
async function pageDoc(slug) {
  let doc;
  try { doc = await api("GET", `/api/v1/docs/${enc(slug)}`, undefined, !!token()); }
  catch (e) { return render(msg(e.message || "That guide is not available.")); }
  render(h("p", {}, link("/docs", "Docs")), h("h1", {}, doc.title), h("div", { class: "card" }, renderMarkdown(doc.body)));
}


function ownerPicker(choices, selected) {
  let value = selected;
  let open = false;
  let active = 0;
  const hidden = h("input", { type: "hidden", name: "owner", value, "aria-label": "Owner" });
  const button = h("button", {
    type: "button",
    id: "upload-owner-toggle",
    class: "combo-toggle",
    "aria-haspopup": "listbox",
    "aria-expanded": "false",
    "aria-controls": "upload-owner-list",
    "aria-label": "Owner",
  }, labelOf(value));
  const filter = h("input", {
    type: "search",
    class: "combo-filter",
    placeholder: "Filter owners",
    "aria-label": "Filter owners",
    autocomplete: "off",
    role: "combobox",
    "aria-autocomplete": "list",
    "aria-controls": "upload-owner-list",
    "aria-expanded": "false",
  });
  const list = h("div", { class: "combo-options", role: "listbox", id: "upload-owner-list", "aria-label": "Owner" });
  const menu = h("div", { class: "combo-menu", hidden: true }, filter, list);
  const wrap = h("div", { class: "combo" }, hidden, button, menu);
  function labelOf(v) {
    const found = choices.find((c) => c.value === v);
    return found ? found.label : (choices[0] ? choices[0].label : "");
  }
  function matches() {
    const q = filter.value.trim().toLowerCase();
    return choices.filter((c) => !q || c.label.toLowerCase().includes(q) || String(c.value).toLowerCase().includes(q));
  }
  function setOpen(next) {
    open = next;
    menu.hidden = !open;
    button.setAttribute("aria-expanded", open ? "true" : "false");
    filter.setAttribute("aria-expanded", open ? "true" : "false");
    if (open) {
      const rows = matches();
      const idx = rows.findIndex((c) => c.value === value);
      active = idx >= 0 ? idx : 0;
      draw();
      filter.focus();
      filter.select();
    } else {
      filter.value = "";
    }
  }
  function select(choice) {
    value = choice.value;
    hidden.value = value;
    button.replaceChildren(document.createTextNode(labelOf(value)));
    setOpen(false);
    button.focus();
  }
  function draw() {
    const rows = matches();
    if (!rows.length) {
      active = 0;
      filter.removeAttribute("aria-activedescendant");
      list.replaceChildren(h("div", { class: "combo-empty muted", role: "status" }, "No matches"));
      return;
    }
    if (active >= rows.length) active = rows.length - 1;
    if (active < 0) active = 0;
    list.replaceChildren(...rows.map((c, i) => h("div", {
      class: "combo-option" + (i === active ? " active" : "") + (c.value === value ? " selected" : ""),
      role: "option",
      id: "upload-owner-opt-" + i,
      "aria-selected": (c.value === value) ? "true" : "false",
      onclick: (e) => { e.preventDefault(); select(c); },
      onmouseenter: () => { active = i; draw(); },
    }, c.label)));
    const cur = list.children[active];
    if (cur) {
      filter.setAttribute("aria-activedescendant", cur.id);
      cur.scrollIntoView({ block: "nearest" });
    }
  }
  button.addEventListener("click", (e) => { e.preventDefault(); setOpen(!open); });
  button.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      setOpen(true);
    }
  });
  filter.addEventListener("input", () => { active = 0; draw(); });
  filter.addEventListener("keydown", (e) => {
    const rows = matches();
    if (e.key === "ArrowDown") { e.preventDefault(); if (rows.length) { active = Math.min(active + 1, rows.length - 1); draw(); } }
    else if (e.key === "ArrowUp") { e.preventDefault(); if (rows.length) { active = Math.max(active - 1, 0); draw(); } }
    else if (e.key === "Home") { e.preventDefault(); active = 0; draw(); }
    else if (e.key === "End") { e.preventDefault(); if (rows.length) { active = rows.length - 1; draw(); } }
    else if (e.key === "Enter") { e.preventDefault(); if (rows[active]) select(rows[active]); }
    else if (e.key === "Escape") { e.preventDefault(); setOpen(false); button.focus(); }
  });
  document.addEventListener("mousedown", function onDoc(e) {
    if (!document.body.contains(wrap)) {
      document.removeEventListener("mousedown", onDoc);
      return;
    }
    if (open && !wrap.contains(e.target)) setOpen(false);
  });
  return wrap;
}

async function pageUpload() {
  if (!me) return needLogin();
  if (!me.verified) {
    return render(h("h1", {}, "Upload a package"), h("div", { class: "card" },
      h("p", {}, "Verify your email before uploading. "), link("/account", "Account")));
  }
  const orgs = await api("GET", "/api/v1/orgs").catch(() => []);
  const choices = [{ value: "", label: `Your account (${me.name})` }].concat(
    orgs.filter((o) => o.role === "owner" || o.role === "admin" || o.role === "contributor")
      .map((o) => ({ value: o.name, label: o.name })));
  const f = h("form", { class: "stack", onsubmit: async (e) => {
    e.preventDefault();
    const file = f.elements.archive.files[0];
    if (!file) return flash("Choose a .tar.gz, .tar, or .zip.", "error");
    const body = new FormData();
    body.append("archive", file);
    if (val(f, "owner")) body.append("organization", val(f, "owner"));
    try {
      const res = await fetch("/api/v1/packages/upload", {
        method: "POST",
        headers: { Authorization: `Bearer ${token()}`, "X-CSRF-Token": token() },
        body,
      });
      const text = await res.text();
      let data = null;
      try { data = text ? JSON.parse(text) : null; } catch { data = null; }
      if (!res.ok) throw new Error((data && data.error) || `${res.status}`);
      history.pushState(null, "", `/packages/${enc(data.name)}`);
      await route();
      flash(`Published ${data.pkgbase} ${data.version}.`);
    } catch (err) { flash(err.message, "error"); }
  } },
    h("div", {}, h("label", { for: "upload-owner-toggle" }, "Owner"), ownerPicker(choices, "")),
    h("input", { name: "archive", type: "file", accept: ".tar,.tar.gz,.tgz,.zip,application/gzip,application/zip", required: true, "aria-label": "Package archive" }),
    h("button", {}, "Upload"));
  render(h("h1", {}, "Upload a package"), h("div", { class: "card" },
    h("p", {}, "Upload a .tar.gz, .tar, or .zip of one package directory. It needs PKGBUILD and .SRCINFO at the top, or inside a single folder. The server checks the recipe and does not run the PKGBUILD. A new package is adopted by the owner you pick: your account, or an organization where you can manage packages (Owner, Admin, or Contributor). The default is your account. Open the list and type in the filter at the top to narrow it. Updating a package does not move it to a different owner."),
    h("p", {}, link("/docs/publish", "Publish guide")),
    f));
}

async function pageRevision(name, rev) {
  let info;
  try { info = await api("GET", `/api/v1/packages/by-name/${enc(name)}`, undefined, false); }
  catch (e) { return render(msg(e.message)); }
  if (!info || !info.Name) return render(msg(`No package named ${name}.`));
  let revision;
  try { revision = await api("GET", `/api/v1/packages/${enc(info.PackageBase)}/revisions/${enc(rev)}`, undefined, false); }
  catch (e) { return render(h("h1", {}, "Revision not found"), msg(e.message)); }
  const rows = [
    ["Version", revision.version || "—"],
    ["pkgver", revision.pkgver || "—"],
    ["pkgrel", revision.pkgrel || "—"],
    ["Description", revision.description || "—"],
    ["Revision", h("code", {}, revision.id)],
    ["Committed", fmtTime(revision.committed_at)],
    ["Snapshot", h("a", { href: revision.snapshot }, "tarball")],
  ];
  render(
    h("p", {}, link(`/packages/${enc(info.Name)}`, `${info.Name} ${info.Version}`)),
    h("h1", {}, `${info.Name} ${revision.version || revision.id.slice(0, 12)}`),
    h("div", { class: "card" }, h("dl", {}, rows.map(([k, v]) => [h("dt", {}, k), h("dd", {}, v)])))
  );
}

// ---- router ----
async function route() {
  const path = decodeURIComponent(location.pathname);
  const params = new URLSearchParams(location.search);
  nav();
  let m;
  if (path === "/" || path === "/packages" || path === "/packages/") return pageSearch(params);
  if ((m = path.match(/^\/packages\/([^/]+)\/revisions\/([^/]+)\/?$/))) return pageRevision(m[1], m[2]);
  if ((m = path.match(/^\/packages\/([^/]+)\/?$/))) return pagePackage(m[1]);
  if ((m = path.match(/^\/users\/([^/]+)\/?$/))) return pageUser(m[1]);
  if (path === "/login") return pageLogin();
  if (path === "/register") return pageRegister();
  if (path === "/verify") return pageVerify(params);
  if (path === "/reset") return pageReset(params);
  if (path === "/account") return pageAccount();
  if (path === "/upload") return pageUpload();
  if (path === "/orgs") return pageOrgs(params);
  if ((m = path.match(/^\/orgs\/([^/]+)\/?$/))) return pageOrg(m[1]);
  if (path === "/requests") return pageRequests(params);
  if (path === "/audit") return pageAudit();
  if (path === "/docs") return pageDocs();
  if ((m = path.match(/^\/docs\/([^/]+)\/?$/))) return pageDoc(m[1]);
  if (path === "/admin") return pageAdmin(params);
  if (path === "/registries") return pageRegistries();
  if ((m = path.match(/^\/registries\/([^/]+)\/?$/))) return pageRegistry(m[1]);
  if (path === "/mirrors") return pageMirrors();
  if ((m = path.match(/^\/mirrors\/([^/]+)\/?$/))) return pageMirror(m[1]);
  render(msg("Page not found."));
}

function go(url) { history.pushState(null, "", url); route(); }

document.addEventListener("click", (e) => {
  const a = e.target.closest("a[data-link]");
  if (a && !e.metaKey && !e.ctrlKey && a.origin === location.origin) { e.preventDefault(); go(a.getAttribute("href")); }
});
window.addEventListener("popstate", route);
loadMe().then(route);
