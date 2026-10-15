import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { httpsOrigin, validateResponse, validateCredential } from "../dist/validators.js";

test("accepts only canonical HTTPS origins without embedded credentials", () => {
  assert.equal(
    httpsOrigin("https://Example.com/login?secret=never-send#fragment"),
    "https://example.com",
  );
  assert.equal(
    httpsOrigin("https://example.com:8443/login"),
    "https://example.com:8443",
  );
  for (const url of [
    "http://example.com",
    "brave://settings",
    "file:///tmp/a",
    "https://user:pass@example.com",
    "not a url",
    undefined,
  ])
    assert.equal(httpsOrigin(url), null);
});
test("copies only account suggestion fields from valid responses", () => {
  const result = validateResponse({
    version: 1,
    status: "ok",
    matches: [
      {
        id: "1",
        title: "<script>not HTML</script>",
        username: "me",
        url: "https://example.com",
        password: "discard",
      },
    ],
  });
  assert.equal(result.matches.length, 1);
  assert.equal(Object.hasOwn(result.matches[0], "password"), false);
});
test("rejects malformed or oversized native replies", () => {
  for (const value of [
    null,
    {},
    { version: 2, status: "ok", matches: [] },
    { version: 1, status: "unknown", matches: [] },
    { version: 1, status: "ok", matches: Array(51).fill({}) },
    { version: 1, status: "locked", matches: [{ id: "1" }] },
    {
      version: 1,
      status: "ok",
      matches: [{ id: "1", title: "x".repeat(257), username: "", url: "" }],
    },
    {
      version: 1,
      status: "ok",
      matches: [{ id: "1", title: "bad\u0000", username: "", url: "" }],
    },
  ])
    assert.equal(validateResponse(value), null);
});
test("accepts backend field limits and rejects each oversized field", () => {
  const match = {
    id: "1",
    title: "t".repeat(256),
    username: "u".repeat(4096),
    url: "x".repeat(2048),
  };
  const response = (entry) => ({ version: 1, status: "ok", matches: [entry] });
  assert.notEqual(validateResponse(response(match)), null);
  for (const field of ["title", "username", "url"]) {
    assert.equal(
      validateResponse(response({ ...match, [field]: match[field] + "x" })),
      null,
    );
  }
  const small = { id: "1", title: "", username: "", url: "" };
  assert.notEqual(
    validateResponse({
      version: 1,
      status: "ok",
      matches: Array(50).fill(small),
    }),
    null,
  );
  assert.equal(
    validateResponse({
      version: 1,
      status: "ok",
      matches: Array(51).fill(small),
    }),
    null,
  );
});
test("accepts all non-success statuses only with no matches", () => {
  for (const status of ["locked", "unavailable", "error"])
    assert.deepEqual(validateResponse({ version: 1, status, matches: [] }), {
      version: 1,
      status,
      matches: [],
    });
});
test("manifest identity matches native host registration and permissions stay minimal", async () => {
  const manifest = JSON.parse(
    await readFile(new URL("../manifest.json", import.meta.url), "utf8"),
  );
  const identity = JSON.parse(
    await readFile(new URL("../identity.json", import.meta.url), "utf8"),
  );
  const expected = createHash("sha256")
    .update(Buffer.from(manifest.key, "base64"))
    .digest("hex")
    .slice(0, 32)
    .replace(/[0-9a-f]/g, (c) => String.fromCharCode(97 + parseInt(c, 16)));
  assert.equal(identity.extensionId, expected);
  assert.equal(identity.nativeHost, "org.fakkarni.bridge");
  assert.deepEqual(manifest.permissions, ["activeTab", "nativeMessaging", "scripting"]);
  assert.deepEqual(manifest.background, {service_worker: "dist/background.js", type: "module"});
  assert.equal(manifest.host_permissions, undefined);
  assert.equal(manifest.content_scripts, undefined);
});

test("locked suggestions contain only bounded metadata; credential channel is separate", () => {
  const entry = {id: "1", title: "Example", username: "me", url: "https://example.com", password: "discard"};
  assert.deepEqual(validateResponse({version: 1, status: "locked", matches: [entry]}).matches[0],
    {id: "1", title: "Example", username: "me", url: "https://example.com"});
  assert.deepEqual(validateCredential({username: "me", password: "secret", extra: "discard"}), {username: "me", password: "secret"});
  for (const credential of [null, {}, {username: "me", password: ""}, {username: "me", password: "x".repeat(4097)}, {username: "\0", password: "secret"}])
    assert.equal(validateCredential(credential), null);
});
