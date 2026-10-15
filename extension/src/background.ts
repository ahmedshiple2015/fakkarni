import { httpsOrigin, validateCredential, validateResponse } from "./validators.js";
import { fillLogin } from "./fill.js";

type Tab = { id?: number; url?: string; windowId?: number };
type Api = {
  tabs: {
    query(query: {active: boolean; currentWindow: boolean}): Promise<Tab[]>;
    get(id: number): Promise<Tab>;
    update(id: number, options: {active: boolean}): Promise<unknown>;
    onUpdated: {addListener(callback: (id: number, change: {url?: string; status?: string}) => void): void};
    onRemoved: {addListener(callback: (id: number) => void): void};
  };
  windows: {update(id: number, options: {focused: boolean}): Promise<unknown>};
  scripting: {executeScript(options: {target: {tabId: number; frameIds?: number[]; documentIds?: string[]}; func: typeof fillLogin | typeof capturePage; args: [string,string,string,string] | [string]}): Promise<{frameId: number; documentId?: string; result?: string | boolean}[]>};
  action: {setBadgeText(options: {tabId: number; text: string}): Promise<void>; setTitle(options: {tabId: number; title: string}): Promise<void>};
  runtime: {
    id: string; getURL(path: string): string;
    sendNativeMessage(host: string, message: unknown): Promise<unknown>;
    onMessage: {addListener(callback: (message: unknown, sender: {id?: string; url?: string}, respond: (response: unknown) => void) => boolean): void};
  };
};
declare const chrome: Api | undefined;
type Pending = { tabId: number; url: string; origin: string; documentId: string; requestId: string; cancelled: boolean };
const host = "org.fakkarni.bridge";
const isRecord = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
function capturePage(url: string): boolean { return window.top === window && location.href === url && location.protocol === "https:"; }

export function createCoordinator(api: Api, wait = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms))) {
  let pending: Pending | undefined;
  let startGeneration = 0;
  async function cancel(task: Pending) {
    task.cancelled = true;
    if (pending === task) pending = undefined;
    try { await api.runtime.sendNativeMessage(host, {version: 1, action: "cancel_fill", origin: task.origin, request_id: task.requestId}); } catch { /* No secret retained. */ }
  }
  async function current(task: Pending) {
    if (pending !== task || task.cancelled) return false;
    try { return (await api.tabs.get(task.tabId)).url === task.url && pending === task && !task.cancelled; } catch { return false; }
  }
  async function indicator(task: Pending, text: string, title: string) {
    try { await api.action.setBadgeText({tabId: task.tabId, text}); await api.action.setTitle({tabId: task.tabId, title}); } catch { /* Tab may have closed. */ }
  }
  async function run(task: Pending, id: string) {
    let delivered = false;
    try {
      const begun = validateResponse(await api.runtime.sendNativeMessage(host, {version: 1, action: "begin_fill", origin: task.origin, id, request_id: task.requestId}));
      if (!begun || begun.status !== "pending") throw new Error("request_failed");
      await indicator(task, "…", "Fakkarni: unlock and approve in the desktop app");
      const deadline = Date.now() + 120000;
      while (Date.now() < deadline && await current(task)) {
        await wait(500);
        if (!await current(task)) break;
        let raw: unknown = await api.runtime.sendNativeMessage(host, {version: 1, action: "poll_fill", origin: task.origin, request_id: task.requestId});
        const response = validateResponse(raw);
        if (!response) throw new Error("invalid_response");
        if (response.status === "pending") { raw = undefined; continue; }
        if (response.status !== "ok" || !isRecord(raw)) throw new Error("cancelled");
        let credential = validateCredential(raw.credential);
        raw = undefined;
        if (!credential || !await current(task)) { credential = null; throw new Error("page_changed"); }
        // Native handoff outlives the popup. Full URL checks prevent filling after navigation.
        const results = await api.scripting.executeScript({target: {tabId: task.tabId, documentIds: [task.documentId]}, func: fillLogin,
          args: [task.url, task.origin, credential.username, credential.password]});
        credential = null;
        delivered = true;
        const filled = results.length === 1 && results[0].frameId === 0 && results[0].documentId === task.documentId && results[0].result === "filled";
        if (!await current(task)) return;
        await indicator(task, filled ? "✓" : "!", filled ? "Fakkarni: filled; review and submit the form yourself" : "Fakkarni: form was ambiguous or unsupported; fill it manually");
        if (filled) {
          const tab = await api.tabs.get(task.tabId);
          if (tab.url === task.url && await current(task)) {
            await api.tabs.update(task.tabId, {active: true});
            if (typeof tab.windowId === "number") await api.windows.update(tab.windowId, {focused: true});
          }
        }
        return;
      }
      throw new Error("expired");
    } catch {
      if (pending === task && !task.cancelled) await indicator(task, "!", "Fakkarni: fill cancelled or unavailable. Reopen the extension to retry.");
    } finally {
      if (!delivered) await cancel(task);
      if (pending === task) pending = undefined;
    }
  }
  return {
    async start(message: unknown): Promise<{ok: boolean}> {
      if (!isRecord(message) || message.action !== "fill_account" || typeof message.id !== "string" || !message.id || message.id.length > 128 ||
          typeof message.tabId !== "number" || !Number.isInteger(message.tabId) || typeof message.url !== "string") return {ok: false};
      const generation = ++startGeneration;
      const [tab] = await api.tabs.query({active: true, currentWindow: true});
      const origin = httpsOrigin(tab?.url);
      if (!origin || tab.id !== message.tabId || tab.url !== message.url) return {ok: false};
      const documents = await api.scripting.executeScript({target: {tabId: tab.id, frameIds: [0]}, func: capturePage, args: [tab.url]});
      const documentId = documents.length === 1 && documents[0].frameId === 0 && documents[0].result === true ? documents[0].documentId : undefined;
      if (!documentId || generation !== startGeneration || (await api.tabs.get(tab.id)).url !== tab.url) return {ok: false};
      if (pending) await cancel(pending);
      if (generation !== startGeneration) return {ok: false};
      const requestId = Array.from(crypto.getRandomValues(new Uint8Array(16)), b => b.toString(16).padStart(2,"0")).join("");
      const task: Pending = {tabId: tab.id, url: tab.url, origin, documentId, requestId, cancelled: false};
      pending = task;
      void run(task, message.id);
      return {ok: true};
    },
    invalidate(tabId: number) { if (pending?.tabId === tabId) void cancel(pending); },
  };
}

if (typeof chrome !== "undefined") {
  const api = chrome;
  const coordinator = createCoordinator(api);
  api.tabs.onUpdated.addListener((id, change) => { if (change.url !== undefined || change.status === "loading") coordinator.invalidate(id); });
  api.tabs.onRemoved.addListener(id => coordinator.invalidate(id));
  api.runtime.onMessage.addListener((message, sender, respond) => {
    if (sender.id !== api.runtime.id || sender.url !== api.runtime.getURL("popup.html")) return false;
    void coordinator.start(message).then(respond, () => respond({ok: false}));
    return true;
  });
}
