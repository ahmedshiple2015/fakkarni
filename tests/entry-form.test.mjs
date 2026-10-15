import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "typescript";
const source = await readFile(new URL("../src/entry-form.ts", import.meta.url), "utf8");
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText;
const { entryFormPayload } = await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);
function form(password = "synthetic-secret", confirmation = password) {
  const data = new FormData();
  for (const [key, value] of Object.entries({ title: "Synthetic", username: "person@example.test", url: "https://example.test", password, confirmPassword: confirmation, notes: "note" })) data.set(key, value);
  return data;
}
test("ordinary edits ignore password controls even if autofill populated them", () => {
  const result = entryFormPayload(form("person@example.test", "different"), "existing-id", false);
  assert.equal(result.replacePassword, false);
  assert.equal(result.input.password, "");
  assert.equal(result.input.username, "person@example.test");
});
test("new logins and explicit password changes require matching confirmation", () => {
  for (const id of [null, "existing-id"]) {
    assert.throws(() => entryFormPayload(form("secret", "email@example.test"), id, true), /confirmation/);
    assert.throws(() => entryFormPayload(form(""), id, true), /Enter a password/);
    const result = entryFormPayload(form(), id, true);
    assert.equal(result.replacePassword, true);
    assert.equal(result.input.password, "synthetic-secret");
    assert.notEqual(result.input.password, result.input.username);
  }
});
test("new login cannot use metadata-only mode to bypass password validation", () => {
  assert.throws(() => entryFormPayload(form(""), null, false), /Enter a password/);
});

test("matching email autofill in both password controls cannot turn a metadata edit into a password replacement", () => {
  const data = form("person@example.test", "person@example.test");
  data.set("title", "Updated site title");
  data.set("url", "https://example.test/new-login");
  const result = entryFormPayload(data, "existing-id", false);
  assert.equal(result.replacePassword, false);
  assert.equal(result.input.password, "");
  assert.equal(result.input.title, "Updated site title");
  assert.equal(result.input.url, "https://example.test/new-login");
});

test("disabled or absent password controls preserve the password for existing entries", () => {
  const data = form();
  data.delete("password");
  data.delete("confirmPassword");
  const result = entryFormPayload(data, "existing-id", false);
  assert.equal(result.replacePassword, false);
  assert.equal(result.input.password, "");
  assert.equal(result.input.username, "person@example.test");
});

test("turning password replacement off discards stale edited values and re-enabling validates again", () => {
  const data = form("intentional-new-secret");
  const explicit = entryFormPayload(data, "existing-id", true);
  assert.equal(explicit.replacePassword, true);
  assert.equal(explicit.input.password, "intentional-new-secret");
  data.set("password", "autofilled@example.test");
  data.set("confirmPassword", "stale previous confirmation");
  const cancelled = entryFormPayload(data, "existing-id", false);
  assert.equal(cancelled.replacePassword, false);
  assert.equal(cancelled.input.password, "");
  assert.throws(() => entryFormPayload(data, "existing-id", true), /confirmation/);
  assert.equal(explicit.input.password, "intentional-new-secret", "later form changes must not mutate a previously constructed request");
});

test("successive metadata edits remain password-preserving after unrelated input autofill", () => {
  const first = form();
  first.delete("password");
  first.delete("confirmPassword");
  const second = form("person@example.test");
  second.set("notes", "Edited after filling the website twice");
  for (const data of [first, second, form("synthetic-secret")]) {
    const result = entryFormPayload(data, "existing-id", false);
    assert.equal(result.replacePassword, false);
    assert.equal(result.input.password, "");
  }
});
