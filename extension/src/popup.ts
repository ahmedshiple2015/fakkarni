import { httpsOrigin, validateResponse } from "./validators.js";

const host = "org.fakkarni.bridge";
const browserName = /Edg\//.test(navigator.userAgent) ? "Edge" : "Brave";
const registrationScript = browserName === "Edge" ? "register-edge.ps1" : "register-brave.ps1";
const extensionsPage = browserName === "Edge" ? "edge://extensions" : "brave://extensions";
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
      "Load this extension in Brave or Edge to connect to your local desktop vault. This preview cannot access accounts.",
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
    let connectionError = "";
    try {
      raw = await api.runtime.sendNativeMessage(host, {
        version: 1,
        action: "match",
        origin,
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : "";
      if (/not found|not registered/i.test(message)) {
        connectionError = `${browserName} cannot find the Fakkarni browser bridge. Run scripts/${registrationScript}, restart ${browserName}, then retry. (HOST_NOT_FOUND)`;
      } else if (/forbidden|not allowed|permission/i.test(message)) {
        connectionError = `${browserName} denied this extension access to the bridge. Check its ID at ${extensionsPage} and reinstall the bridge. (HOST_DENIED)`;
      } else if (/exited|disconnect|communicat|launch|start/i.test(message)) {
        connectionError = `${browserName} could not start or communicate with the Fakkarni bridge. Reinstall the bridge and restart ${browserName}. (HOST_CONNECTION_FAILED)`;
      } else {
        connectionError = `${browserName} could not connect to the Fakkarni browser bridge. Restart ${browserName} and retry. (NATIVE_MESSAGE_FAILED)`;
      }
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
        connectionError || "The browser bridge is installed, but it cannot reach Fakkarni. Start the desktop app, then refresh. (DESKTOP_UNAVAILABLE)",
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
