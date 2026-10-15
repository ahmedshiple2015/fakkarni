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
