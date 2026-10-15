import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import ts from "typescript";

const source = readFileSync(new URL("../src/brand-icons.ts", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText;
const { resolveBrandIcon, initialFor } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString("base64")}`);

test("known service hosts and their real subdomains use local icon identifiers", () => {
  assert.equal(resolveBrandIcon("https://github.com/login").slug, "github");
  assert.equal(resolveBrandIcon("https://ACCOUNTS.GOOGLE.COM/signin?x=1").slug, "google");
  assert.equal(resolveBrandIcon("http://store.steampowered.com:80/login").slug, "steam");
  assert.equal(resolveBrandIcon("https://login.riotgames.com/").slug, "riotgames");
  assert.equal(resolveBrandIcon("https://www.youtube.com/").name, "YouTube");
});

test("lookalike, malformed, credential-bearing and unknown websites never use the title", () => {
  for (const site of ["https://github.com.evil.test", "https://notgithub.com", "https://github-com.test", "https://gіthub.com", "https://user@github.com", "https://user:pass@github.com", "github.com", "//github.com", "javascript:alert(1)", "file:///github.com", "https:\\github.com", "https://github.com/\n", "https://unknown.test", "http://", "https://github.com."]) {
    assert.equal(resolveBrandIcon(site, "Discord"), null, site);
  }
  assert.equal(resolveBrandIcon("https://reddit.com", "Discord").slug, "reddit");
});

test("only an empty website permits exact normalized app title identification", () => {
  assert.equal(resolveBrandIcon("", "  DISCORD  ").slug, "discord");
  assert.equal(resolveBrandIcon(" ", "Riot   Client").slug, "riotgames");
  assert.equal(resolveBrandIcon("", "Telegram Desktop").slug, "telegram");
  assert.equal(resolveBrandIcon("", "Discord scam"), null);
  assert.equal(resolveBrandIcon("", "Google"), null);
  assert.equal(resolveBrandIcon("", ""), null);
});

test("unknown services retain complete Unicode initials and empty fallback", () => {
  assert.equal(initialFor("  witAnime "), "W");
  assert.equal(initialFor("فكرني"), "ف");
  assert.equal(initialFor("e\u0301clair"), "E\u0301");
  assert.equal(initialFor("👩🏽‍💻 work"), "👩🏽‍💻");
  assert.equal(initialFor(" \n\t "), "?");
  assert.equal(initialFor("\u202e Steam"), "S");
});

test("bundled SVG assets contain only static local geometry", () => {
  const directory = new URL("../icons/", import.meta.url);
  const files = readdirSync(directory).filter(file => file.endsWith(".svg"));
  assert.equal(files.length, 19);
  for (const file of files) {
    const svg = readFileSync(new URL(file, directory), "utf8");
    assert.ok(svg.length < 16_384, file);
    assert.match(svg, /^<svg\s/);
    assert.match(svg, /viewBox="0 0 24 24"/);
    assert.doesNotMatch(svg, /<(?!\/?(?:svg|title|path)\b)|\bon\w+\s*=|\b(?:href|style|src)\s*=|<!|&/i, file);
    assert.match(svg, /<path d="[Mm0-9.,\sA-Za-z+\-]+"\s*\/>/);
    for (const tag of svg.matchAll(/<(svg|path|title)\b([^>]*)>/g)) {
      const allowed = tag[1] === "svg" ? ["role", "viewBox", "xmlns"] : tag[1] === "path" ? ["d"] : [];
      const attributes = tag[2].replace(/\/?\s*$/, "");
      for (const attribute of attributes.matchAll(/([^\s=]+)="([^"]*)"/g)) assert.ok(allowed.includes(attribute[1]), `${file}: ${attribute[1]}`);
      assert.equal(attributes.replace(/[^\s=]+="[^"]*"/g, "").trim(), "", file);
    }
  }
});
