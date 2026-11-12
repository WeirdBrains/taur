#!/usr/bin/env node
// Exercises h() and render() from the real app.js against a minimal DOM.
// Regression: a missing optional archived banner must not become a literal
// "null"/"undefined" text node above the package title.
"use strict";

const fs = require("fs");
const path = require("path");
const vm = require("vm");

class Node {}
class TextNode extends Node {
  constructor(text) {
    super();
    this.nodeType = 3;
    this.data = String(text);
  }
  get textContent() { return this.data; }
}
class Element extends Node {
  constructor(tag) {
    super();
    this.nodeType = 1;
    this.tagName = String(tag).toUpperCase();
    this.children = [];
    this.attributes = {};
    this.className = "";
    this.listeners = {};
  }
  setAttribute(k, v) { this.attributes[k] = String(v); }
  addEventListener(type, fn) { this.listeners[type] = fn; }
  append(...nodes) { for (const n of nodes) this.children.push(n); }
  prepend(...nodes) { this.children.unshift(...nodes); }
  replaceChildren(...nodes) { this.children = [...nodes]; }
  get textContent() {
    return this.children.map((c) => (c && c.textContent !== undefined ? c.textContent : String(c))).join("");
  }
}

const appEl = new Element("div");
appEl.id = "app";
const document = {
  createElement: (tag) => new Element(tag),
  createTextNode: (t) => new TextNode(t),
  getElementById: (id) => (id === "app" ? appEl : null),
};

function extractHelpers(src) {
  const start = src.indexOf("function h(tag, attrs, ...kids) {");
  const end = src.indexOf("\n// ---- pages ----");
  if (start < 0 || end < 0) throw new Error("could not find helpers in app.js");
  return src.slice(start, end);
}

const src = fs.readFileSync(path.join(__dirname, "../web/app.js"), "utf8");
const ctx = { document, Node, URLSearchParams };
vm.createContext(ctx);
vm.runInContext(extractHelpers(src) + "\nthis.h = h; this.render = render; this.indexHref = indexHref; this.pager = pager;", ctx);
const { h, render, indexHref, pager } = ctx;

function assert(cond, msg) {
  if (!cond) {
    console.error("FAIL:", msg);
    console.error("app text:", JSON.stringify(appEl.textContent));
    console.error("children:", appEl.children.map((c) => (c && c.tagName) || typeof c + ":" + c));
    process.exit(1);
  }
}

// Same shape as pagePackage: optional archived banner, then the title.
const title = h("h1", {}, "taur 1.6.0-1");
const outAbsent = [false ? h("p", { class: "flagged" }, "Archived since never") : null, title];
render(...outAbsent);
assert(!appEl.textContent.includes("null"), "absent archived banner must not render literal null");
assert(!appEl.textContent.includes("undefined"), "must not render literal undefined");
assert(appEl.textContent.includes("taur 1.6.0-1"), "title must remain");
assert(appEl.children.length === 1 && appEl.children[0].tagName === "H1", "only the h1 should be present");

render(h("p", { class: "flagged" }, "Archived since 2026-01-02"), h("h1", {}, "taur 1.6.0-1"));
assert(appEl.textContent.includes("Archived since 2026-01-02"), "real archived banner must stay visible");
assert(appEl.textContent.includes("taur 1.6.0-1"), "title must remain beside a real banner");
assert(appEl.children.length === 2, "banner and title");

assert(indexHref("popularity", 2) === "/?sort=popularity&page=2", "pager retains sort and page");
assert(indexHref("downloaded", 3) === "/?sort=downloaded&page=3", "downloaded sort is retained in navigation");
assert(indexHref("popularity", 1) === "/?sort=popularity", "changing sort resets to page 1 (no page query)");
assert(indexHref("updated", 1) === "/?sort=updated", "updated at page 1 keeps the sort");

const nav = pager(2, 25, 20, (n) => indexHref("popularity", n));
assert(nav.textContent.includes("Page 2 of 2"), "pager labels the current page");
assert(nav.children[0].attributes.href === "/?sort=popularity", "previous keeps sort and goes to page 1");
assert(nav.children[2].tagName === "SPAN", "next is inert on the last page");

assert(src.includes('link("/orgs", "Organizations")'), "Organizations is in the public nav");
assert(src.includes('href: "https://github.com/WeirdBrains/taur"'), "GitHub is in the public nav");
assert(src.includes(', "GitHub"'), "GitHub nav label is present");
assert(!src.includes('link("https://github.com/WeirdBrains/taur"'), "GitHub is not a data-link SPA route");
assert(src.includes('const PROJECT_SITE = "https://taur.weirdbrains.com/";'), "project site URL is defined");
assert(src.includes('h("a", { href: PROJECT_SITE }, "Project site")'), "Project site is an external nav link");
assert(src.includes('link("/stats", "Stats")'), "Stats is in the public nav");
assert(!/reportView\([^)]*search/.test(src), "page views never send the query string");
assert(src.includes("/api/v1/organizations"), "directory uses the public organizations API");
assert(!src.includes("Download counts are not recorded"), "downloaded sort must not fall back or disclose a missing counter");
assert(src.includes("DownloadCount"), "UI displays the download count");

assert(src.includes("/packages/${enc(info.Organization)}".replace("info.Organization", "info.Organization")) || src.includes('`/orgs/${enc(info.Organization)}`'), "package org name links to the profile");

console.log("ok");
