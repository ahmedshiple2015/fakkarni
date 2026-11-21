import test from "node:test";
import assert from "node:assert/strict";
import {createCaptureWorker} from "../dist/capture-worker.js";

function fixture(t) {
  let tab = {id: 3, url: "https://example.test/login"}, status = "pending";
  const proposals = [], notices = [], cleared = [];
  const worker = createCaptureWorker({
    active: async () => tab,
    tab: async () => tab,
    indicator: async () => {},
    notify: async (...args) => notices.push(args),
    clearNotice: async id => cleared.push(id),
    propose: async (...args) => { proposals.push(args); return status; },
  });
  t.after(() => worker.remove(3));
  return {worker, proposals, notices, cleared, setStatus: value => { status = value; },
    setUrl: url => { tab = {...tab, url}; },
    capture(overrides = {}, sender = {}) {
      worker.capture({username: "alice", password: "synthetic-secret", changeForm: false, ...overrides},
        {tab, url: tab.url, documentId: "document-a", frameId: 0, ...sender});
    },
    action(action) { return worker.popup({action, tabId: 3}); },
  };
}

test("automatic capture notifies without sending the password until review", async t => {
  const f = fixture(t);
  f.capture({}, {frameId: 2});
  f.capture({}, {documentId: undefined});
  f.capture({}, {url: "https://different.test/login"});
  assert.equal(f.notices.length, 0);
  f.capture();
  f.capture();
  assert.equal(f.notices.length, 1);
  const [[id, origin]] = f.notices;
  assert.equal(origin, "https://example.test");
  assert.deepEqual(await f.action("capture_status"), {state: "ready", username: "alice"});
  assert.equal(f.proposals.length, 0);
  f.setUrl("https://example.test/home");
  f.worker.updated(3, {status: "loading", url: "https://example.test/home"});
  assert.equal(await f.worker.reviewNotice("stale-notice"), "changed");
  assert.equal(await f.worker.reviewNotice(id), "pending");
  assert.deepEqual(f.proposals, [["https://example.test", "alice", "synthetic-secret", false]]);
  assert.deepEqual(f.cleared, [id]);
  assert.equal(await f.worker.reviewNotice(id), "changed");
});

test("cross-origin navigation, a second load, dismissal and tab close revoke capture", async t => {
  for (const mode of ["cross", "second", "discard", "close"]) {
    const f = fixture(t);
    f.capture({changeForm: true, username: ""});
    const [[id]] = f.notices;
    if (mode === "cross") f.worker.updated(3, {url: "https://other.test/"});
    if (mode === "second") {
      f.worker.updated(3, {status: "loading"});
      f.worker.updated(3, {status: "loading"});
    }
    if (mode === "discard") f.worker.dismiss(id);
    if (mode === "close") f.worker.remove(3);
    assert.equal(await f.worker.reviewNotice(id), "changed");
    assert.equal(f.proposals.length, 0);
  }
});

test("locked vault keeps the candidate for a deliberate retry", async t => {
  const f = fixture(t);
  f.capture();
  f.setStatus("locked");
  assert.equal(await f.worker.reviewNotice(f.notices[0][0]), "locked");
  assert.equal((await f.action("capture_status")).state, "ready");
  f.setStatus("pending");
  assert.equal((await f.action("capture_review")).state, "pending");
  assert.equal((await f.action("capture_status")).state, "idle");
});
