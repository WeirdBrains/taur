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
  const end = src.indexOf("\nfunction msg(");
  if (start < 0 || end < 0) throw new Error("could not find h()/render() in app.js");
  return src.slice(start, end);
}

const src = fs.readFileSync(path.join(__dirname, "../web/app.js"), "utf8");
const ctx = { document, Node };
vm.createContext(ctx);
vm.runInContext(extractHelpers(src) + "\nthis.h = h; this.render = render;", ctx);
const { h, render } = ctx;

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

console.log("ok");
