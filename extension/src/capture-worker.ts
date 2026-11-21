import { httpsOrigin } from "./validators.js";

type Tab = { id?: number; url?: string };
export type CaptureSender = { id?: string; url?: string; documentId?: string; frameId?: number; tab?: Tab };
type Candidate = {
  tabId: number; origin: string; username: string; password: string;
  changeForm: boolean; noticeId: string; expires: number; navigations: number; reviewing: boolean;
};

export function createCaptureWorker(api: {
  active: () => Promise<Tab | undefined>;
  tab: (id: number) => Promise<Tab>;
  indicator: (id: number, text: string) => Promise<void>;
  notify: (id: string, origin: string) => Promise<unknown>;
  clearNotice: (id: string) => Promise<unknown>;
  propose: (origin: string, username: string, password: string, changeForm: boolean) => Promise<string>;
}) {
  let candidate: Candidate | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  function clear() {
    const previous = candidate;
    candidate = undefined;
    clearTimeout(timer);
    if (!previous) return;
    void api.indicator(previous.tabId, "").catch(() => {});
    void api.clearNotice(previous.noticeId).catch(() => {});
  }
  function current() {
    if (candidate && Date.now() >= candidate.expires) clear();
    return candidate;
  }
  async function review(ready: Candidate) {
    if (current() !== ready || ready.reviewing) return "changed";
    ready.reviewing = true;
    try {
      const tab = await api.tab(ready.tabId);
      if (current() !== ready || httpsOrigin(tab.url) !== ready.origin) return "changed";
      const state = await api.propose(ready.origin, ready.username, ready.password, ready.changeForm);
      if (candidate === ready && state === "pending") clear();
      return state;
    } catch {
      return "unavailable";
    } finally {
      ready.reviewing = false;
    }
  }
  return {
    capture(message: Record<string, unknown>, sender: CaptureSender) {
      const origin = httpsOrigin(sender.url);
      const tabId = sender.tab?.id;
      if (!origin || tabId === undefined || sender.frameId !== 0 || !sender.documentId ||
          httpsOrigin(sender.tab?.url) !== origin) return;
      if (typeof message.username !== "string" || typeof message.password !== "string" ||
          typeof message.changeForm !== "boolean" || !message.password ||
          (!message.changeForm && !message.username) || /[\x00-\x1f\x7f]/.test(message.username) ||
          message.password.includes("\0")) return;
      const bytes = new TextEncoder();
      if (bytes.encode(message.username).length > 512 || bytes.encode(message.password).length > 1024) return;
      const previous = current();
      if (previous?.reviewing || (previous?.tabId === tabId && previous.origin === origin &&
          previous.username === message.username && previous.password === message.password)) return;
      clear();
      const ready: Candidate = {tabId, origin, username: message.username, password: message.password,
        changeForm: message.changeForm, noticeId: `fakkarni-save-${crypto.randomUUID()}`,
        expires: Date.now() + 25_000, navigations: 0, reviewing: false};
      candidate = ready;
      timer = setTimeout(clear, 25_000);
      void api.indicator(tabId, "Save").catch(() => {});
      void api.notify(ready.noticeId, origin).then(() => {
        if (candidate !== ready) return api.clearNotice(ready.noticeId);
      }).catch(() => {});
    },
    updated(id: number, change: {url?: string; status?: string}, tab?: Tab) {
      const ready = current();
      if (ready?.tabId !== id) return;
      if ((change.url !== undefined && httpsOrigin(change.url) !== ready.origin) ||
          (change.status === "complete" && httpsOrigin(tab?.url) !== ready.origin) ||
          (change.status === "loading" && ++ready.navigations > 1)) {
        clear();
      } else if (change.status === "complete") {
        // Chromium resets per-tab badge text on navigation.
        void api.indicator(id, "Save").catch(() => {});
      }
    },
    remove(id: number) { if (current()?.tabId === id) clear(); },
    dismiss(noticeId: string) { if (current()?.noticeId === noticeId) clear(); },
    noticeClosed(noticeId: string) {
      const ready = current();
      if (ready?.noticeId === noticeId && !ready.reviewing) clear();
    },
    async reviewNotice(noticeId: string) {
      const ready = current();
      return ready?.noticeId === noticeId ? review(ready) : "changed";
    },
    async popup(message: Record<string, unknown>): Promise<{ state: string; username?: string }> {
      const tab = await api.active();
      const ready = current();
      if (!ready || tab?.id !== message.tabId || ready.tabId !== tab?.id ||
          ready.origin !== httpsOrigin(tab.url)) return {state: "idle"};
      if (message.action === "capture_discard") {
        clear();
        return {state: "idle"};
      }
      if (message.action === "capture_review") {
        return {state: await review(ready), username: ready.username};
      }
      return {state: "ready", username: ready.username};
    },
  };
}
