import { httpsOrigin, validateResponse } from "./validators.js";

const host = "org.fakkarni.bridge";
const browserName = /Edg\//.test(navigator.userAgent) ? "Edge" : "Brave";
const registrationScript = browserName === "Edge" ? "register-edge.ps1" : "register-brave.ps1";
const extensionsPage = browserName === "Edge" ? "edge://extensions" : "brave://extensions";
const originLabel = document.querySelector<HTMLElement>("#origin")!;
const state = document.querySelector<HTMLElement>("#state")!;
const list = document.querySelector<HTMLElement>("#matches")!;
const refresh = document.querySelector<HTMLButtonElement>("#refresh")!;
const connection = document.querySelector<HTMLElement>("#connection")!;
let generation = 0;
const setConnection = (status: "checking" | "connected" | "locked" | "unavailable", label: string) => {
  connection.dataset.status = status;
  connection.textContent = label;
};
const setState = (message: string) => {
  state.textContent = message;
};

async function load() {
  const current = ++generation;
  list.replaceChildren();
  originLabel.textContent = "Checking active site…";
  refresh.disabled = true;
  setConnection("checking", "Checking");
  setState("Looking for accounts on this website…");
  if (
    typeof chrome === "undefined" ||
    !chrome.tabs?.query ||
    !chrome.runtime?.sendNativeMessage
  ) {
    originLabel.textContent = "Extension preview";
    setConnection("unavailable", "Not connected");
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
      setConnection("unavailable", "HTTPS required");
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
      setConnection("unavailable", "Page changed");
      setState("Refresh to check the current page.");
      return;
    }
    const response = validateResponse(raw);
    if (!response) {
      setConnection("unavailable", "Connection error");
      setState(
        "The desktop connection returned an invalid response. No accounts were displayed.",
      );
      return;
    }
    if (response.status === "locked") {
      setConnection("locked", "Vault locked");
      if (!response.matches.length) {
        setState("Unlock your vault in the desktop app, then refresh. To show accounts while locked, enable locked account suggestions in desktop preferences.");
        return;
      }
    }
    if (response.status === "unavailable") {
      setConnection("unavailable", "Not connected");
      setState(
        connectionError || "The browser bridge is installed, but it cannot reach Fakkarni. Start the desktop app, then refresh. (DESKTOP_UNAVAILABLE)",
      );
      return;
    }
    if (!["ok", "locked"].includes(response.status)) {
      setConnection("unavailable", "Connection error");
      setState(
        "The desktop vault could not complete this request. Try refreshing.",
      );
      return;
    }
    if (response.status === "ok") setConnection("connected", "Unlocked");
    setState(response.status === "locked" ? "Vault locked. Choose an account to open Fakkarni and unlock, then this page fills automatically." :
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
      copy.className = "account-copy";
      const title = document.createElement("strong");
      title.textContent = match.title || "Untitled login";
      const username = document.createElement("span");
      username.textContent = match.username || "No username";
      copy.append(title, username);
      const button = document.createElement("button");
      button.type = "button";
      button.className = "account-button";
      const action = document.createElement("small");
      action.textContent = response.status === "locked" ? "Unlock to fill" : "Fill login";
      copy.append(action);
      button.append(icon, copy);
      const arrow = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      arrow.setAttribute("viewBox", "0 0 24 24");
      arrow.setAttribute("fill", "none");
      arrow.setAttribute("stroke", "currentColor");
      arrow.setAttribute("stroke-width", "1.6");
      arrow.setAttribute("stroke-linecap", "round");
      arrow.setAttribute("stroke-linejoin", "round");
      arrow.setAttribute("aria-hidden", "true");
      arrow.classList.add("account-arrow");
      const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
      path.setAttribute("d", "M5 12h14m-6-6 6 6-6 6");
      arrow.append(path);
      button.append(arrow);
      button.addEventListener("click", async () => {
        button.disabled = true;
        button.setAttribute("aria-busy", "true");
        action.textContent = "Starting fill…";
        try {
          const result = await api.runtime.sendMessage({action: "fill_account", tabId: tab.id, url: tab.url, id: match.id});
          if (current !== generation) return;
          setState(result && typeof result === "object" && "ok" in result && result.ok === true
            ? "Filling this account. Unlock Fakkarni if needed; filling continues when this popup closes."
            : "The page changed or filling could not start. Refresh and try again.");
        } catch (error) {
          if (current !== generation) return;
          const message = error instanceof Error ? error.message : "";
          if (/message (port|channel) closed|before a response|asynchronous response/i.test(message))
            setState("The popup closed during handoff. Check whether the form filled, then reopen this popup to retry if needed. (HANDOFF_CLOSED)");
          else if (/receiving end does not exist|could not establish connection|context invalidated/i.test(message))
            setState("The extension background connection is unavailable. Reload Fakkarni at the browser’s extensions page. (WORKER_UNAVAILABLE)");
          else setState("The browser could not send the fill request. Reopen this popup and retry. (REQUEST_FAILED)");
        }
        finally {
          if (current === generation) {
            button.disabled = false;
            button.removeAttribute("aria-busy");
            action.textContent = response.status === "locked" ? "Unlock to fill" : "Fill login";
          }
        }
      });
      row.append(button);
      list.append(row);
    }
  } catch {
    if (current === generation) {
      setConnection("unavailable", "Not connected");
      setState(
        "Unable to read the active tab. Reopen the extension on an HTTPS website.",
      );
    }
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
