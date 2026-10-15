import { httpsOrigin, validateResponse } from "./validators.js";

const host = "org.fakkarni.bridge";
const originLabel = document.querySelector<HTMLElement>("#origin")!;
const state = document.querySelector<HTMLElement>("#state")!;
const list = document.querySelector<HTMLElement>("#matches")!;
const refresh = document.querySelector<HTMLButtonElement>("#refresh")!;
let generation = 0;
const setState = (message: string) => {
  state.textContent = message;
};

async function load() {
  const current = ++generation;
  list.replaceChildren();
  originLabel.textContent = "Checking active site…";
  refresh.disabled = true;
  if (
    typeof chrome === "undefined" ||
    !chrome.tabs?.query ||
    !chrome.runtime?.sendNativeMessage
  ) {
    originLabel.textContent = "Extension preview";
    setState(
      "Load this extension in Brave to connect to your local desktop vault. This preview cannot access accounts.",
    );
    return;
  }
  const api = chrome;
  try {
    const [tab] = await api.tabs.query({ active: true, currentWindow: true });
    if (current !== generation) return;
    const origin = httpsOrigin(tab?.url);
    if (!tab || typeof tab.id !== "number" || !origin) {
      originLabel.textContent = "No supported site";
      setState(
        "Open an HTTPS website, then refresh to find matching accounts.",
      );
      return;
    }
    originLabel.textContent = origin;
    setState("Checking your local vault…");
    let raw: unknown;
    try {
      raw = await api.runtime.sendNativeMessage(host, {
        version: 1,
        action: "match",
        origin,
      });
    } catch {
      raw = { version: 1, status: "unavailable", matches: [] };
    }
    if (current !== generation) return;
    // Navigation or a tab switch invalidates even a valid native reply.
    const [active] = await api.tabs.query({
      active: true,
      currentWindow: true,
    });
    if (current !== generation) return;
    if (active?.id !== tab.id || active.url !== tab.url) {
      originLabel.textContent = "Active page changed";
      setState("Refresh to check the current page.");
      return;
    }
    const response = validateResponse(raw);
    if (!response) {
      setState(
        "The desktop connection returned an invalid response. No accounts were displayed.",
      );
      return;
    }
    if (response.status === "locked") {
      setState("Unlock your vault in the Fakkarni desktop app, then refresh.");
      return;
    }
    if (response.status === "unavailable") {
      setState(
        "Start the Fakkarni desktop app and check that the browser bridge is installed, then refresh.",
      );
      return;
    }
    if (response.status === "error") {
      setState(
        "The desktop vault could not complete this request. Try refreshing.",
      );
      return;
    }
    setState(
      response.matches.length
        ? `${response.matches.length} matching ${response.matches.length === 1 ? "account" : "accounts"}`
        : "No saved accounts match this site. Add a login in the desktop app.",
    );
    for (const match of response.matches) {
      const row = document.createElement("li");
      const icon = document.createElement("span");
      icon.className = "account-icon";
      icon.setAttribute("aria-hidden", "true");
      icon.textContent = (match.title || "?").slice(0, 1).toUpperCase();
      const copy = document.createElement("div");
      const title = document.createElement("strong");
      title.textContent = match.title || "Untitled login";
      const username = document.createElement("span");
      username.textContent = match.username || "No username";
      copy.append(title, username);
      row.append(icon, copy);
      list.append(row);
    }
  } catch {
    if (current === generation)
      setState(
        "Unable to read the active tab. Reopen the extension on an HTTPS website.",
      );
  } finally {
    if (current === generation) refresh.disabled = false;
  }
}
refresh.addEventListener("click", () => {
  void load();
});
window.addEventListener("pagehide", () => {
  generation++;
  list.replaceChildren();
});
void load();
