import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.FAKKARNI_PLAYWRIGHT || "playwright");
const source = readFileSync(new URL("../extension/dist/capture.js", import.meta.url), "utf8");
const fixtures = {"update":"<input type=\"password\" autocomplete=\"current-password\"><input type=\"password\" autocomplete=\"new-password\"><input type=\"password\" name=\"confirmPassword\">","login":"<input type=\"email\" autocomplete=\"email\"><input type=\"password\" autocomplete=\"current-password\">","revealed":"<input type=\"text\" name=\"username\"><input type=\"text\" name=\"password\">","signup":"<input name=\"username\"><input type=\"password\" autocomplete=\"new-password\"><input type=\"password\" name=\"confirmPassword\">"};
const mock = "(() => {\n  window.isTauri = true;\n  window.qa = {unlocked:true, pending:null, calls:[]};\n  window.__TAURI_INTERNALS__ = { transformCallback:()=>1, invoke:async (cmd,args) => {\n    if (cmd==='vault_status') return {exists:true, unlocked:qa.unlocked};\n    if (cmd==='hello_status') return {available:true, enabled:true, reason:null};\n    if (cmd==='get_app_settings' || cmd==='browser_settings') throw 'Use fixture defaults';\n    if (cmd==='list_entries') return [];\n    if (cmd==='browser_save_pending') return qa.pending;\n    if (cmd==='export_recovery_key') { qa.calls.push({cmd,args}); return 'ab'.repeat(32); }\n    if (cmd==='recover_master_password') { qa.calls.push({cmd,args}); qa.unlocked=false; return {warnings:[]}; }\n    if (cmd==='resolve_browser_save') { qa.calls.push({cmd,args}); qa.pending=null; return; }\n    return null;\n  }};\n})();";
const browser = await chromium.launch({executablePath: process.env.FAKKARNI_TEST_CHROMIUM});
try {
  for (const [kind, rejection] of [
    ["login"], ["revealed"], ["signup"], ["update"], ["signup", "mismatch"],
    ["login", "cross"], ["login", "synthetic"], ["ajaxLogin"], ["ajaxSignup"],
  ]) {
    const page = await browser.newPage();
    const action = rejection === "cross" ? "https://other.test/" : "/submit";
    await page.route("**/*", route => route.fulfill({
      contentType: "text/html",
      body: '<form method="post" action="' + action + '">' +
        (kind === "ajaxLogin" ? '<input name="username"><input type="password" name="password">' :
          kind === "ajaxSignup" ? '<input type="email" name="email"><input name="username"><input type="password" name="password"><input type="password" name="password_compare">' :
          fixtures[kind]) + '<button>Send</button></form>',
    }));
    await page.goto("https://synthetic.example/form");
    await page.evaluate(ajax => {
      window.captured = [];
      window.chrome = {runtime: {sendMessage: async value => captured.push(value)}};
      const form = document.querySelector("form");
      form.addEventListener("submit", event => {
        event.preventDefault();
        if (ajax) throw new Error("AJAX fixture unexpectedly dispatched a DOM submit");
      });
      if (ajax) form.querySelector("button").addEventListener("click", event => event.preventDefault());
    }, kind.startsWith("ajax"));
    await page.evaluate(source);
    const inputs = page.locator("input");
    const count = await inputs.count();
    for (let index = 0; index < count; index++) {
      let value = "new-synthetic";
      if (index === 0) value = kind === "update" ? "old-synthetic" : "alice@example.test";
      if ((kind === "ajaxSignup" && index === 1) || (kind === "ajaxLogin" && index === 0)) value = "alice";
      if (rejection === "mismatch" && index === count - 1) value = "mismatch";
      const input = inputs.nth(index);
      if (rejection === "synthetic") {
        await input.evaluate((field, text) => {
          field.value = text;
          field.dispatchEvent(new Event("input", {bubbles: true}));
        }, value);
      } else {
        await input.fill(value);
      }
    }
    await page.getByRole("button", { name: "Send" }).click();
    assert.deepEqual(await page.evaluate(() => captured), rejection ? [] : [{
      action: "capture_candidate",
      username: kind === "update" ? "" : kind.startsWith("ajax") ? "alice" : "alice@example.test",
      password: "new-synthetic", changeForm: kind === "update",
    }], kind + " " + (rejection || "capture"));
    await page.close();
  }

  const page = await browser.newPage({viewport: {width:1180, height:800}});
  await page.addInitScript(mock);
  await page.goto(process.env.FAKKARNI_TEST_URL || "http://127.0.0.1:1420");
  const button = name => page.getByRole("button", {name, exact: true});
  await page.locator("#nav-settings").click();
  await button("Create recovery key").click();
  await page.locator("#recovery-secret").fill("synthetic old master");
  await button("Save recovery file").click();
  assert.equal(await page.getByRole("textbox", {name: "Recovery key", exact: true}).inputValue(), "ab".repeat(32));
  await button("I saved the recovery key").click();
  await page.locator("dialog").waitFor({state: "detached"});
  await page.evaluate(() => {
    qa.pending = {token:"test", origin:"https://example.test", username:"alice", accounts:[
      {id:"one", title:"First", username:"alice", url:"https://example.test/"},
      {id:"two", title:"Second", username:"alice", url:"https://example.test/other"},
    ]};
  });
  await button("Update password").click();
  assert.equal(await page.evaluate(() => qa.calls.length), 1);
  await page.getByLabel("Saved account to update").selectOption("two");
  await button("Update password").click();
  await page.locator("#browser-save").waitFor({state: "detached"});
  assert.equal(await page.evaluate(() => qa.calls[1].args.entryId), "two");
  await page.locator("#nav-settings").click();
  await button("Reset master password with PIN").click();
  await page.locator("#recovery-new").fill("synthetic new master");
  await page.locator("#recovery-repeat").fill("synthetic new master");
  await button("Verify PIN and reset").click();
  await page.locator("#master").waitFor({state: "visible"});
  assert.equal(await page.evaluate(() => qa.calls[2].args.recoveryKey), null);
  await page.evaluate(() => {
    qa.pending = {token:"locked-save", origin:"https://example.test", username:"alice", accounts:null};
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = (command, args) => {
      if (command === "unlock_vault") {
        qa.unlocked = true;
        qa.pending.accounts = [];
        return Promise.resolve();
      }
      return invoke(command, args);
    };
  });
  await page.getByRole("heading", {name:"Unlock to review this login"}).waitFor();
  assert.equal(await button("Save login").count(), 0);
  await page.locator("#master").fill("synthetic new master");
  await button("Unlock vault").click();
  await page.getByRole("heading", {name:"Review new login"}).waitFor();
  assert.equal(await page.evaluate(() => qa.calls.length), 3);
  await button("Save login").click();
  await page.locator("#browser-save").waitFor({state:"detached"});
  assert.equal(await page.evaluate(() => qa.calls[3].args.token), "locked-save");
} finally {
  await browser.close();
}
console.log("Passed form capture, explicit account approval, and recovery dialogs.");

if (process.env.FAKKARNI_EXTENSION_CHROMIUM) {
  const extension = fileURLToPath(new URL("../extension", import.meta.url));
  const context = await chromium.launchPersistentContext("", {
    executablePath: process.env.FAKKARNI_EXTENSION_CHROMIUM,
    ignoreDefaultArgs: ["--disable-extensions"],
    args: [`--disable-extensions-except=${extension}`, `--load-extension=${extension}`],
  });
  try {
    const isFakkarni = worker => worker.url().endsWith("/dist/background.js");
    const worker = context.serviceWorkers().find(isFakkarni) ??
      await context.waitForEvent("serviceworker", {predicate: isFakkarni, timeout: 5000});
    const page = context.pages()[0];
    await page.route("https://synthetic.example/**", route => route.fulfill({
      contentType: "text/html",
      body: new URL(route.request().url()).pathname !== "/form" ? "<p>Signed in</p>" :
        '<form method="post" action="/submit">' + fixtures.login + '<button>Send</button></form>' +
        '<script>document.querySelector("button").onclick = event => {' +
        'event.preventDefault(); fetch("/submit", {method:"POST"}).then(() => location.assign("/signed-in"));' +
        '};</script>',
    }));
    await page.goto("https://synthetic.example/form");
    await page.locator('input[type="email"]').fill("alice@example.test");
    await page.locator('input[type="password"]').fill("synthetic-extension-password");
    await page.getByRole("button", {name: "Send"}).click();
    await page.getByText("Signed in").waitFor();
    const offer = await worker.evaluate(async () => {
      let details;
      for (let attempt = 0; attempt < 50; attempt++) {
        const tabs = await chrome.tabs.query({url: "https://synthetic.example/*"});
        const notices = await chrome.notifications.getAll();
        const badge = tabs.length ? await chrome.action.getBadgeText({tabId: tabs[0].id}) : "";
        const id = Object.keys(notices).find(key => key.startsWith("fakkarni-save-"));
        if (id && badge === "Save") return {id, badge};
        details = {badge, notices: Object.keys(notices), tabCount: tabs.length,
          permission: await chrome.notifications.getPermissionLevel()};
        await new Promise(resolve => setTimeout(resolve, 50));
      }
      return details;
    });
    assert.ok(offer.id, `Expected a notification without opening the popup. Got ${JSON.stringify(offer)}`);
  } finally {
    await context.close();
  }
  console.log("Passed automatic capture and notification creation with the unpacked extension installed.");
}
