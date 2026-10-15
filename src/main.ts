import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "../extension/theme.css";
import "./style.css";
import { entryFormPayload } from "./entry-form";
import { icon } from "./icons";
import { initialFor, resolveBrandIcon } from "../extension/src/brand-icons";

type Status = {
  exists: boolean;
  unlocked: boolean;
  path: string;
  idleMinutes: number;
};
type Entry = { id: string; title: string; username: string; url: string };
type FullEntry = Entry & { password: string; notes: string };
type HelloStatus = { available: boolean; enabled: boolean; reason: string | null };
let hello: HelloStatus = { available: false, enabled: false, reason: "PIN is available only in the desktop app." };
type AppSettings = { startWithWindows: boolean; closeToTray: boolean; startMinimized: boolean };
let appSettings: AppSettings = { startWithWindows: false, closeToTray: true, startMinimized: false };
let settingsError: string | null = null;
type BrowserSettings = { showAccountsWhenLocked: boolean };
type BrowserFill = { requestId: string; origin: string; title: string; username: string };
let browserSettings: BrowserSettings = { showAccountsWhenLocked: false };
let browserSettingsError: string | null = null;
let pendingFill: BrowserFill | null = null;
let pendingRead = false;
let backupBusy = false;
let view: "vault" | "settings" = "vault";
type RestorePreview = { token: string; entryCount: number; fileName: string; hasExistingVault: boolean };
type RestoreReport = { entryCount: number; recoveryPath: string | null; warnings: string[] };
const app = document.querySelector<HTMLDivElement>("#app")!;
const native = isTauri();
let status: Status = {
  exists: false,
  unlocked: false,
  path: "",
  idleMinutes: 5,
};
let entries: Entry[] = [],
  selected: string | null = null,
  epoch = 0,
  hideTimer = 0;
const esc = (s: string) =>
  s.replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ]!,
  );
const shield = icon("shield");
const brand = `<div class="brand"><span class="brand-icon">${icon("keyhole")}</span><div>Fakkarni<small lang="ar" dir="rtl">فكرني</small></div></div>`;
const brandAssets = import.meta.glob("../extension/icons/*.svg", { eager: true, query: "?url", import: "default" }) as Record<string, string>;
function accountAvatar(entry: Entry, large = false) {
  const brand = resolveBrandIcon(entry.url, entry.title);
  const asset = brand ? brandAssets[`../extension/icons/${brand.slug}.svg`] : undefined;
  return `<span class="avatar${large ? " big" : ""}" aria-hidden="true"><span>${esc(initialFor(entry.title))}</span>${asset ? `<img src="${esc(asset)}" alt="" hidden>` : ""}</span>`;
}
function attachIconFallbacks(root: Element) {
  root.querySelectorAll<HTMLImageElement>(".avatar img").forEach(img => {
    const fallback = () => { img.remove(); };
    const loaded = () => { img.hidden = false; };
    img.addEventListener("load", loaded, { once: true });
    img.addEventListener("error", fallback, { once: true });
    if (img.complete) { if (img.naturalWidth > 0) loaded(); else fallback(); }
  });
}
function siteLabel(url: string) {
  try { return new URL(url).hostname; } catch { return "Saved login"; }
}
function message(text: string) {
  const el = document.querySelector("#message");
  if (el) el.textContent = text;
}
function clearSecrets() {
  epoch++;
  backupBusy = false;
  window.clearTimeout(hideTimer);
  document
    .querySelectorAll<HTMLInputElement | HTMLTextAreaElement>("input,textarea")
    .forEach((el) => (el.value = ""));
  document.querySelector("dialog")?.remove();
  entries = [];
  selected = null;
  view = "vault";
  app.replaceChildren();
}
function shell(content: string) {
  app.dataset.locked = String(!status.unlocked);
  app.innerHTML = `<aside>${brand}<nav aria-label="Workspace"><button type="button" class="nav active" id="nav-vault" aria-label="Passwords" title="Passwords" aria-current="page" ${status.unlocked ? "" : "disabled"}>${icon("key")}<span class="nav-text">Passwords</span></button><button type="button" class="nav" id="nav-settings" aria-label="Settings" title="Settings" ${status.unlocked ? "" : "disabled"}>${icon("settings")}<span class="nav-text">Settings</span></button></nav><div class="sidebar-bottom"><span>${icon("monitor")}<span class="device-label">On this PC</span></span><button class="icon-button" id="lock" aria-label="Lock vault" title="Lock vault" ${status.unlocked ? "" : "disabled"}>${icon("lock")}</button></div></aside><main>${content}</main>`;
  document.querySelector("#nav-vault")!.addEventListener("click", () => selectView("vault"));
  document.querySelector("#nav-settings")!.addEventListener("click", () => selectView("settings"));
}
function selectView(next: "vault" | "settings", focus = true) {
  if (!status.unlocked) return;
  view = next;
  const settings = next === "settings";
  document.querySelector<HTMLElement>("#vault-panel")!.hidden = settings;
  document.querySelector<HTMLElement>("#settings-panel")!.hidden = !settings;
  document.querySelector("#page-title")!.textContent = settings ? "Settings" : "Passwords";
  document.querySelector<HTMLElement>("#workspace-heading")!.hidden = !settings;
  app.dataset.view = next;
  document.querySelector<HTMLElement>("#page-count")!.hidden = settings;
  document.querySelector("#page-description")!.textContent = settings ? "Unlock, browser, and desktop preferences." : "";
  for (const [id, active] of [["nav-vault", !settings], ["nav-settings", settings]] as const) {
    const nav = document.querySelector<HTMLButtonElement>(`#${id}`)!;
    nav.classList.toggle("active", active);
    if (active) nav.setAttribute("aria-current", "page");
    else nav.removeAttribute("aria-current");
  }
  // Leaving account details must hide a password that was temporarily revealed.
  if (settings) {
    window.clearTimeout(hideTimer);
    const password = document.querySelector("#password");
    if (password) password.textContent = "••••••••••••";
    const reveal = document.querySelector("#reveal");
    if (reveal) reveal.textContent = "Show";
  }
  if (focus) document.querySelector<HTMLElement>(settings ? "#page-title" : "#search")?.focus();
}
function passwordControl(id: string, attributes: string, label: string) {
  return `<div class="password-control"><input id="${id}" type="password" ${attributes}><button type="button" class="reveal-control" data-password-toggle="${id}" data-password-label="${label}" aria-label="Show ${label}" aria-pressed="false">${icon("eye")}</button></div>`;
}
function attachPasswordControls(root: Element) {
  root.querySelectorAll<HTMLButtonElement>("[data-password-toggle]").forEach(button => {
    button.addEventListener("click", () => {
      const field = root.querySelector<HTMLInputElement>(`#${button.dataset.passwordToggle}`);
      if (!field || field.disabled) return;
      const show = field.type === "password";
      field.type = show ? "text" : "password";
      button.setAttribute("aria-pressed", String(show));
      button.setAttribute("aria-label", `${show ? "Hide" : "Show"} ${button.dataset.passwordLabel}`);
    });
  });
}
function locked() {
  shell(
    `<div class="topbar"><span class="breadcrumb">Personal vault</span><span class="pill">${icon("lock")}${native ? "Vault locked" : "Design preview"}</span></div><section class="welcome"><div class="auth-heading"><div class="large-shield">${icon("keyhole")}</div><h1>${status.exists ? "Your vault is locked" : "Create your vault"}</h1><p>${native ? (status.exists ? "Unlock to access your saved logins." : "Choose a master password to protect your logins.") : "Preview only. Open the desktop app to access your vault."}</p></div><form id="unlock"><div class="auth-field"><label for="master">Master password</label>${passwordControl("master", `minlength="${status.exists ? 1 : 12}" required autocomplete="${status.exists ? "current-password" : "new-password"}" ${native ? "" : "disabled"} placeholder="${status.exists ? "Enter your master password" : "At least 12 characters"}"`, "master password")}</div>${!status.exists ? `<div class="auth-field"><label for="confirm">Confirm master password</label>${passwordControl("confirm", `required autocomplete="new-password" ${native ? "" : "disabled"} placeholder="Repeat your master password"`, "master password confirmation")}</div><small class="hint">Keep it somewhere safe. There is no password reset.</small>` : ""}<button id="unlock-submit" type="submit" class="primary wide" ${native ? "" : "disabled"}><span>${status.exists ? "Unlock vault" : "Create vault"}</span>${icon("arrow")}</button><div id="message" role="status"></div></form><div class="privacy">${icon("lock")}Stored on this PC</div></section>`,
  );
  const unlockForm = document.querySelector<HTMLFormElement>("#unlock")!;
  renderBrowserFill();
  attachPasswordControls(unlockForm);
  if (status.exists && hello.enabled) {
    unlockForm.querySelector("#unlock-submit")!.classList.replace("primary", "secondary");
    const button = document.createElement("button");
    button.id = "hello-unlock";
    button.className = "primary wide hello-unlock";
    button.innerHTML = `${icon("windows")}<span>Unlock with PIN</span>${icon("arrow")}`;
    button.disabled = !native || !hello.available;
    unlockForm.before(button);
    const fallback = document.createElement("p");
    fallback.className = "auth-divider";
    fallback.textContent = "or use your master password";
    unlockForm.before(fallback);
    button.addEventListener("click", async () => {
      if (!native) return;
      const generation = epoch;
      button.disabled = true;
      message("Complete the PIN prompt…");
      try {
        await invoke("unlock_hello");
        if (generation !== epoch) return;
        clearSecrets();
        await refresh();
      } catch (error) {
        if (generation === epoch && button.isConnected) {
          message(String(error));
          button.disabled = !hello.available;
        }
      }
    });
  }
  const helper = document.createElement("p");
  helper.className = "hint hello-hint";
  helper.textContent = !status.exists
    ? "You can enable PIN after creating your vault. Keep your master password for recovery."
    : hello.reason || (hello.enabled ? "Windows handles your PIN, fingerprint, or face. Fakkarni never receives your Windows PIN." : "Enable PIN from inside your vault for quicker unlocking.");
  unlockForm.after(helper);
  if (native) {
    const restore = document.createElement("button");
    restore.className = "text-button import-action";
    restore.type = "button";
    restore.innerHTML = `${icon("folder")}<span>Import or restore an encrypted vault</span>`;
    helper.after(restore);
    restore.addEventListener("click", restoreBackup);
  }
  if (native) {
    if (status.exists && hello.enabled && hello.available)
      document.querySelector<HTMLButtonElement>("#hello-unlock")?.focus();
    else document.querySelector<HTMLInputElement>("#master")?.focus();
  }
  document
    .querySelector("#unlock")!
    .addEventListener("submit", async (event) => {
      event.preventDefault();
      if (!native) return;
      const password = (document.querySelector("#master") as HTMLInputElement)
        .value;
      if (
        !status.exists &&
        password !==
          (document.querySelector("#confirm") as HTMLInputElement).value
      ) {
        message("Passwords do not match.");
        return;
      }
      const generation = epoch;
      (document.querySelector("#master") as HTMLInputElement).value = "";
      const confirmation = document.querySelector<HTMLInputElement>("#confirm");
      if (confirmation) confirmation.value = "";
      const button =
        document.querySelector<HTMLButtonElement>("#unlock-submit")!;
      button.disabled = true;
      message("Opening your vault…");
      try {
        await invoke(status.exists ? "unlock_vault" : "create_vault", {
          password,
        });
        if (generation !== epoch) return;
        clearSecrets();
        await refresh();
      } catch (error) {
        if (generation === epoch) {
          message(String(error));
          button.disabled = false;
        }
      }
    });
}
async function refresh() {
  const generation = epoch;
  try {
    const [next, nextHello, nextSettings, nextBrowserSettings] = await Promise.all([
      invoke<Status>("vault_status"),
      invoke<HelloStatus>("hello_status").catch((error): HelloStatus => ({
        available: false, enabled: false, reason: `PIN is unavailable: ${String(error)}`,
      })),
      invoke<AppSettings>("get_app_settings").then(
        (settings) => ({ settings, error: null }),
        (error) => ({ settings: appSettings, error: `Could not load desktop settings: ${String(error)}` }),
      ),
      invoke<BrowserSettings>("browser_settings").then(
        (settings) => ({ settings, error: null }),
        (error) => ({ settings: browserSettings, error: `Browser preferences unavailable: ${String(error)}` }),
      ),
    ]);
    if (generation !== epoch) return;
    status = next;
    hello = nextHello;
    appSettings = nextSettings.settings;
    settingsError = nextSettings.error;
    browserSettings = nextBrowserSettings.settings;
    browserSettingsError = nextBrowserSettings.error;
    await refreshBrowserFill();
    if (generation !== epoch) return;
    if (!status.unlocked) {
      clearSecrets();
      locked();
      return;
    }
    const result = await invoke<Entry[]>("list_entries");
    if (generation !== epoch) return;
    entries = result;
    vault();
  } catch (error) {
    if (generation === epoch) {
      clearSecrets();
      status.unlocked = false;
      locked();
      message(String(error));
    }
  }
}
function vault() {
  shell(
    `<header class="topbar" id="workspace-heading" hidden><div><div class="page-heading"><h1 id="page-title" tabindex="-1">Settings</h1><span class="count" id="page-count" hidden>${entries.length}</span></div><p id="page-description">Unlock, browser, and desktop preferences.</p></div></header><div id="vault-panel"><div class="vault-grid"><section class="accounts-panel" aria-label="Saved passwords"><div class="collection-heading"><h1>Passwords <span class="count" id="list-count">${entries.length}</span></h1><button id="add" class="primary">${icon("plus")}New login</button></div><div class="toolbar"><div class="search-wrap">${icon("search")}<input id="search" type="search" placeholder="Search logins…" aria-label="Search passwords"></div></div><div class="entry-list"></div></section><section id="detail" class="detail" aria-label="Login details"><div class="empty"><div class="large-shield">${icon("keyhole")}</div><h2>${entries.length ? "Select a login" : "No saved logins"}</h2><p>${entries.length ? "Choose an account to view its details." : "Use New login to add your first account."}</p></div></section></div></div><div id="settings-panel" hidden><section class="settings-group" id="security-settings" aria-labelledby="security-settings-title"><div class="settings-group-heading"><h2 id="security-settings-title">${icon("shield")}Security &amp; browser</h2></div></section><section class="settings-group" id="desktop-settings-group" aria-labelledby="desktop-group-title"><div class="settings-group-heading"><h2 id="desktop-group-title">${icon("monitor")}Desktop &amp; backup</h2></div></section></div><footer><span>Encrypted on this PC</span><span>Auto-lock · ${status.idleMinutes} minutes</span></footer><div id="message" role="status"></div>`,
  );
  const settings = document.createElement("section");
  settings.className = "hello-settings";
  settings.setAttribute("aria-label", "PIN settings");
  settings.innerHTML = `<div class="setting-row"><div class="setting-copy"><h3>PIN</h3><p>${esc(hello.enabled ? "Enabled on this PC. Your master password remains your recovery fallback." : hello.reason || "Use your Windows PIN, fingerprint, or face to unlock.")}</p></div><button class="secondary" id="hello-settings" ${native && (hello.available || hello.enabled) ? "" : "disabled"}>${hello.enabled ? "Disable PIN" : "Enable PIN"}</button></div>`;
  document.querySelector("#security-settings")!.append(settings);
  settings.querySelector("button")!.addEventListener("click", () => {
    if (!native) return;
    if (!hello.enabled) { helloSetup(); return; }
    void disableHello();
  });
  desktopSettings();
  browserPreferences();
  backupPreferences();
  renderBrowserFill();
  document.querySelector("#lock")!.addEventListener("click", async () => {
    clearSecrets();
    status.unlocked = false;
    locked();
    try {
      await invoke("lock_vault");
    } catch {
      message(
        "Could not confirm vault lock. Close the desktop app to end this session.",
      );
    }
  });
  document.querySelector("#add")!.addEventListener("click", () => editor());
  document.querySelector("#search")!.addEventListener("input", renderList);
  renderList();
  if (entries.length) detail(entries.some(entry => entry.id === selected) ? selected! : entries[0].id, false);
  selectView(view, false);
  if (view === "vault") document.querySelector<HTMLInputElement>("#search")?.focus();
}
async function refreshBrowserFill() {
  if (!native || pendingRead) return;
  pendingRead = true;
  try {
    const next = await invoke<BrowserFill | null>("browser_pending");
    const unchanged = JSON.stringify(next) === JSON.stringify(pendingFill);
    pendingFill = next;
    if (!unchanged || !document.querySelector("#browser-fill")) renderBrowserFill();
  } catch {
    pendingFill = null;
    renderBrowserFill();
  } finally { pendingRead = false; }
}
function renderBrowserFill() {
  document.querySelector("#browser-fill")?.remove();
  if (!pendingFill || !native) return;
  const request = pendingFill;
  const panel = document.createElement("section");
  panel.id = "browser-fill";
  panel.className = "browser-fill";
  panel.setAttribute("aria-label", "Browser autofill request");
  panel.innerHTML = `<span class="eyebrow">BROWSER AUTOFILL REQUEST</span><h2>${esc(request.title || "Saved account")}</h2><p class="fill-origin">${esc(request.origin)}</p><p>${esc(request.username || "No username")}</p><p class="hint">${status.unlocked ? "Filling the selected account in your original browser tab…" : "Unlock below with your master password or PIN. The selected account will fill automatically in the original browser tab. No form will be submitted."}</p><button class="secondary" id="reject-browser-fill">Cancel fill</button>`;
  document.querySelector("main .topbar")?.after(panel);
  panel.querySelector("#reject-browser-fill")!.addEventListener("click", async () => {
    await invoke("reject_browser_fill", { requestId: request.requestId }).catch(() => {});
    await refreshBrowserFill();
  });
}
function browserPreferences() {
  const section = document.createElement("section");
  section.className = "desktop-settings";
  section.setAttribute("aria-label", "Browser privacy settings");
  section.innerHTML = `<label class="setting-toggle"><span class="setting-copy"><strong>Show accounts while locked</strong><small>See matching browser accounts before unlocking.</small><small id="locked-privacy" class="critical-hint">Names, usernames, and websites are stored outside the encrypted vault. Passwords stay encrypted.</small></span><input type="checkbox" role="switch" class="switch" id="locked-suggestions" aria-label="Show accounts while locked" aria-describedby="locked-privacy" ${browserSettings.showAccountsWhenLocked ? "checked" : ""} ${browserSettingsError ? "disabled" : ""}></label><details class="privacy-disclosure"><summary>About locked account suggestions</summary><p class="hint">Choose an account in Brave or Edge to fill it. Unlock Fakkarni first if needed; forms are never submitted automatically. The local index is an unencrypted file readable by programs running as your Windows user. Passwords and notes stay encrypted. Turning this option off removes the index.</p></details><p id="browser-preferences-status" class="hint" role="status">${esc(browserSettingsError || "")}</p>`;
  document.querySelector("#security-settings")!.append(section);
  const checkbox = section.querySelector<HTMLInputElement>("#locked-suggestions")!;
  checkbox.addEventListener("change", async () => {
    const generation = epoch;
    const settings = { showAccountsWhenLocked: checkbox.checked };
    checkbox.disabled = true;
    const feedback = section.querySelector("#browser-preferences-status")!;
    try {
      await invoke("set_browser_settings", { settings });
      if (generation !== epoch || !section.isConnected) return;
      browserSettings = settings;
      feedback.textContent = settings.showAccountsWhenLocked ? "Local account index enabled. Passwords still require vault unlock." : "Locked account index removed.";
    } catch (error) {
      if (generation !== epoch || !section.isConnected) return;
      checkbox.checked = browserSettings.showAccountsWhenLocked;
      feedback.textContent = String(error);
    } finally {
      if (generation === epoch && section.isConnected) checkbox.disabled = false;
    }
  });
}
function backupPreferences() {
  const section = document.createElement("section");
  section.className = "desktop-settings backup-settings";
  section.setAttribute("aria-label", "Backup and PC transfer");
  section.innerHTML = `<div class="backup-copy"><h3>Backup &amp; restore</h3><p class="hint">Export an encrypted copy for backup or transfer to another PC. You will need the backup's master password.</p></div><div class="fill-actions"><button class="secondary" id="create-backup">${icon("upload")}Export encrypted vault</button><button class="secondary" id="restore-backup">${icon("download")}Import / restore vault</button></div><p class="hint">Import replaces this vault, locks it, and resets PIN and locked account suggestions. An encrypted recovery copy is kept.</p><details class="privacy-disclosure"><summary>Keeping backups local</summary><p class="hint">Choose a folder outside cloud sync to keep the exported copy on this device. PIN alone cannot open it on another PC.</p></details><p id="backup-status" role="status"></p>`;
  document.querySelector("#desktop-settings-group")!.append(section);
  section.querySelector("#restore-backup")!.addEventListener("click", restoreBackup);
  section.querySelector<HTMLButtonElement>("#create-backup")!.addEventListener("click", async (event) => {
    if (backupBusy || !native || !status.unlocked) return;
    backupBusy = true;
    const button = event.currentTarget as HTMLButtonElement;
    button.disabled = true;
    const feedback = section.querySelector("#backup-status")!;
    feedback.textContent = "Choose a new backup filename. Existing files will not be overwritten.";
    try {
      const result = await invoke<{fileName: string; path: string} | null>("create_backup");
      if (feedback.isConnected) feedback.textContent = result ? `Encrypted backup saved: ${result.path}` : "Backup cancelled.";
    } catch (error) {
      if (feedback.isConnected) feedback.textContent = String(error);
    } finally {
      backupBusy = false;
      if (button.isConnected) button.disabled = false;
    }
  });
}
function restoreBackup() {
  if (!native || backupBusy || document.querySelector("dialog")) return;
  backupBusy = true;
  let preview: RestorePreview | null = null;
  let committing = false;
  const dialog = document.createElement("dialog");
  dialog.setAttribute("aria-label", "Import encrypted vault");
  dialog.innerHTML = `<form><div class="modal-heading"><h2>Import encrypted vault</h2><button type="button" id="cancel-restore" class="text-button" aria-label="Close">✕</button></div><p class="hint">Enter the exported vault’s master password, then choose its .kdbx file. Validation and preview do not change your current vault.</p><label>Backup master password<input name="backupPassword" type="password" required maxlength="1024" autocomplete="off"></label><p id="restore-status" role="status"></p><button class="primary wide">Choose exported file and preview</button></form>`;
  app.append(dialog);
  const close = () => {
    if (committing) return;
    if (preview) void invoke("cancel_restore", {token: preview.token}).catch(() => {});
    dialog.querySelector<HTMLFormElement>("form")?.reset();
    dialog.remove();
    backupBusy = false;
  };
  dialog.querySelector("#cancel-restore")!.addEventListener("click", close);
  dialog.addEventListener("cancel", event => { event.preventDefault(); close(); });
  const form = dialog.querySelector<HTMLFormElement>("form")!;
  form.addEventListener("submit", async event => {
    event.preventDefault();
    if (preview || committing) return;
    const field = form.elements.namedItem("backupPassword") as HTMLInputElement;
    const password = field.value;
    field.value = "";
    const button = form.querySelector<HTMLButtonElement>("button.primary")!;
    button.disabled = true;
    const feedback = form.querySelector("#restore-status")!;
    feedback.textContent = "Choose the backup to validate…";
    try {
      const result = await invoke<RestorePreview | null>("prepare_restore", {password});
      if (!dialog.isConnected) {
        if (result) void invoke("cancel_restore", {token: result.token}).catch(() => {});
        backupBusy = false;
        return;
      }
      if (!result) { feedback.textContent = "File selection cancelled."; button.disabled = false; return; }
      preview = result;
      form.reset();
      form.innerHTML = `<div class="modal-heading"><h2>Review import</h2></div><p><strong>${esc(result.fileName)}</strong></p><p>${result.entryCount} saved ${result.entryCount === 1 ? "login" : "logins"}</p><p class="hint">${result.hasExistingVault ? "This will replace the current vault. An encrypted recovery copy must be saved successfully before replacement." : "This will create the local vault from this backup."} The restored vault stays locked. Use the backup's master password to reopen it and set up PIN again if wanted.</p><div class="desktop-toggles"><label><input type="checkbox" id="confirm-restore"><span>I understand that restoring replaces the current vault; it does not merge logins.</span></label></div><p id="restore-status" role="status"></p><div class="fill-actions"><button type="button" id="restore-back" class="secondary">Cancel</button><button type="button" id="commit-restore" class="primary" disabled>Import this vault</button></div>`;
      form.querySelector("#restore-back")!.addEventListener("click", close);
      const commit = form.querySelector<HTMLButtonElement>("#commit-restore")!;
      form.querySelector<HTMLInputElement>("#confirm-restore")!.addEventListener("change", event => { commit.disabled = !(event.currentTarget as HTMLInputElement).checked; });
      commit.addEventListener("click", async () => {
        if (!preview || committing) return;
        committing = true;
        form.querySelectorAll<HTMLButtonElement | HTMLInputElement>("button,input").forEach(control => { control.disabled = true; });
        form.querySelector("#restore-status")!.textContent = "Preserving the current encrypted vault and restoring…";
        try {
          const report = await invoke<RestoreReport>("commit_restore", {token: preview.token});
          preview = null;
          clearSecrets();
          await refresh();
          message(`Restored ${report.entryCount} logins. Unlock with the backup's master password.${report.recoveryPath ? ` Previous encrypted vault preserved at: ${report.recoveryPath}` : ""}${report.warnings.length ? ` ${report.warnings.join(" ")}` : ""}`);
        } catch (error) {
          if (dialog.isConnected) {
            form.querySelector("#restore-status")!.textContent = `${String(error)} Close this preview and choose the backup again to retry.`;
            form.querySelector<HTMLButtonElement>("#restore-back")!.disabled = false;
          } else message(String(error));
        } finally { committing = false; backupBusy = false; }
      });
    } catch (error) {
      if (dialog.isConnected) { feedback.textContent = String(error); button.disabled = false; }
      else backupBusy = false;
    }
  });
  dialog.showModal();
  dialog.querySelector<HTMLInputElement>("input")!.focus();
}
function desktopSettings() {
  const section = document.createElement("section");
  section.className = "desktop-settings";
  section.setAttribute("aria-labelledby", "desktop-settings-title");
  const disabled = !native || settingsError !== null;
  const choices: [keyof AppSettings, string, string][] = [
    ["startWithWindows", "Start with Windows", "Open Fakkarni automatically when Windows starts."],
    ["closeToTray", "Keep running in system tray", "Closing the window keeps browser autofill available. Auto-lock still applies."],
    ["startMinimized", "Open in system tray", "Start with the window hidden in the tray."],
  ];
  section.innerHTML = `<h3 id="desktop-settings-title" class="visually-hidden">Desktop preferences</h3><div class="desktop-toggles">${choices.map(([key, label, description]) => `<label class="setting-toggle"><span class="setting-copy"><strong>${label}</strong><small>${description}</small></span><input type="checkbox" role="switch" class="switch" name="${key}" ${appSettings[key] ? "checked" : ""} ${disabled ? "disabled" : ""}></label>`).join("")}</div><p class="hint" id="startup-hint" ${appSettings.startWithWindows ? "" : "hidden"}>Startup uses this executable's current location. Keep it in place; this build has no installer.</p><p class="hint" id="desktop-settings-status" role="status">${esc(settingsError || (!native ? "Desktop preferences are available in the desktop app." : ""))}</p>`;
  document.querySelector("#desktop-settings-group")!.append(section);
  section.querySelectorAll<HTMLInputElement>('input[type="checkbox"]').forEach((checkbox) => {
    checkbox.addEventListener("change", async () => {
      if (!native || !status.unlocked) return;
      const generation = epoch;
      const next = { ...appSettings, [checkbox.name]: checkbox.checked };
      const controls = section.querySelectorAll<HTMLInputElement>("input");
      const feedback = section.querySelector("#desktop-settings-status")!;
      controls.forEach((control) => { control.disabled = true; });
      feedback.textContent = "Saving desktop preferences…";
      try {
        await invoke("set_app_settings", { settings: next });
        if (generation !== epoch || !section.isConnected) return;
        appSettings = next;
        section.querySelector<HTMLElement>("#startup-hint")!.hidden = !next.startWithWindows;
        feedback.textContent = "Desktop preferences saved.";
      } catch (error) {
        if (generation !== epoch || !section.isConnected) return;
        controls.forEach((control) => { control.checked = appSettings[control.name as keyof AppSettings]; });
        feedback.textContent = `Could not save desktop preferences: ${String(error)}`;
      } finally {
        if (generation === epoch && section.isConnected)
          controls.forEach((control) => { control.disabled = false; });
      }
    });
  });
}
async function disableHello() {
  const generation = epoch;
  const button = document.querySelector<HTMLButtonElement>("#hello-settings")!;
  button.disabled = true;
  try {
    await invoke("disable_hello");
    if (generation !== epoch) return;
    if (!document.querySelector("dialog")) await refresh();
  } catch (error) {
    if (generation === epoch && button.isConnected) {
      message(String(error));
      button.disabled = false;
    }
  }
}
function helloSetup() {
  if (!native || !status.unlocked || document.querySelector("dialog")) return;
  const origin = document.activeElement as HTMLElement | null;
  const dialog = document.createElement("dialog");
  dialog.setAttribute("aria-labelledby", "hello-title");
  dialog.setAttribute("aria-describedby", "hello-description");
  dialog.innerHTML = `<form><div class="modal-heading"><h2 id="hello-title">Enable PIN</h2><button type="button" id="hello-cancel" class="text-button" aria-label="Close">✕</button></div><p id="hello-description" class="hint">PIN protects quick unlock on this PC. Keep your master password: you need it for recovery or another PC. Windows will ask for your PIN, fingerprint, or face in its own prompt. Fakkarni never receives your Windows PIN.</p><label>Confirm master password<input name="password" type="password" required autocomplete="current-password" placeholder="Enter your master password"></label><div id="hello-error" role="status"></div><button class="primary wide">Continue to PIN</button></form>`;
  app.append(dialog);
  const form = dialog.querySelector<HTMLFormElement>("form")!;
  const close = () => {
    form.reset();
    dialog.remove();
    if (origin?.isConnected) origin.focus();
  };
  dialog.querySelector("#hello-cancel")!.addEventListener("click", close);
  dialog.addEventListener("cancel", (event) => { event.preventDefault(); close(); });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const generation = epoch;
    const field = form.elements.namedItem("password") as HTMLInputElement;
    const password = field.value;
    field.value = "";
    const button = form.querySelector<HTMLButtonElement>("button.primary")!;
    button.disabled = true;
    dialog.querySelector("#hello-error")!.textContent = "Complete the PIN prompt…";
    try {
      await invoke("enable_hello", { password });
      if (generation !== epoch) return;
      if (!dialog.isConnected) {
        if (!document.querySelector("dialog")) await refresh();
        return;
      }
      close();
      await refresh();
    } catch (error) {
      if (generation === epoch && dialog.isConnected) {
        dialog.querySelector("#hello-error")!.textContent = String(error);
        button.disabled = false;
        field.focus();
      }
    }
  });
  dialog.showModal();
  dialog.querySelector<HTMLInputElement>("input")!.focus();
}
function renderList() {
  const query = (
    document.querySelector("#search") as HTMLInputElement
  ).value.toLowerCase();
  const list = entries.filter((entry) =>
    `${entry.title} ${entry.username} ${entry.url}`
      .toLowerCase()
      .includes(query),
  );
  document.querySelector(".entry-list")!.innerHTML =
    list
      .map(
        (entry) =>
          `<button class="entry ${selected === entry.id ? "selected" : ""}" aria-pressed="${selected === entry.id}" data-id="${esc(entry.id)}">${accountAvatar(entry)}<span class="entry-copy"><strong>${esc(entry.title)}</strong><small>${esc(entry.username || "No username")}</small></span></button>`,
      )
      .join("") || `<div class="list-empty">${icon("search")}<strong>${entries.length ? "No matching logins" : "No logins yet"}</strong><p>${entries.length ? "Try another name, username, or website." : "Add a login to start filling from your browser."}</p></div>`;
  document.querySelector("#list-count")!.textContent = String(list.length);
  attachIconFallbacks(document.querySelector(".entry-list")!);
  document
    .querySelectorAll<HTMLButtonElement>(".entry")
    .forEach((el) =>
      el.addEventListener("click", () => detail(el.dataset.id!)),
    );
}
function detail(id: string, focus = true) {
  const entry = entries.find((e) => e.id === id);
  if (!entry) return;
  window.clearTimeout(hideTimer);
  selected = id;
  renderList();
  document.querySelectorAll<HTMLButtonElement>(".entry").forEach((button) => {
    const active = button.dataset.id === id;
    button.setAttribute("aria-pressed", String(active));
    if (active && focus) button.focus();
  });
  document.querySelector("#detail")!.innerHTML =
    `<div class="detail-heading">${accountAvatar(entry, true)}<div><h2>${esc(entry.title)}</h2><p>${esc(siteLabel(entry.url))}</p></div></div><div class="detail-fields"><div class="field"><span class="field-label">Username</span><div>${esc(entry.username || "—")}</div></div><div class="field"><span class="field-label">Password</span><div class="password-row"><span id="password">••••••••••••</span><button id="reveal" class="text-button">Show</button></div></div><div class="field"><span class="field-label">Website</span><div class="url"><span>${esc(entry.url || "—")}</span></div></div></div><div class="detail-actions"><button id="edit" class="primary">${icon("edit")}Edit login</button><button id="delete-login" class="danger">${icon("trash")}Delete login</button></div><p class="hint detail-hint">Passwords hide after 15 seconds.</p>`;
  attachIconFallbacks(document.querySelector("#detail")!);
  document.querySelector("#reveal")!.addEventListener("click", async () => {
    const generation = epoch;
    const target = document.querySelector("#password")!;
    const button = document.querySelector<HTMLButtonElement>("#reveal")!;
    if (button.textContent === "Hide") {
      target.textContent = "••••••••••••";
      button.textContent = "Show";
      window.clearTimeout(hideTimer);
      return;
    }
    button.disabled = true;
    try {
      const password = await invoke<string>("reveal_password", { id });
      if (generation !== epoch || selected !== id || view !== "vault" || !target.isConnected)
        return;
      target.textContent = password;
      button.textContent = "Hide";
      hideTimer = window.setTimeout(() => {
        target.textContent = "••••••••••••";
        button.textContent = "Show";
      }, 15000);
    } catch (error) {
      if (generation === epoch && selected === id) message(String(error));
    } finally {
      if (generation === epoch && selected === id && button.isConnected)
        button.disabled = false;
    }
  });
  document.querySelector("#delete-login")!.addEventListener("click", () => confirmDelete(entry));
  document.querySelector("#edit")!.addEventListener("click", async () => {
    const generation = epoch;
    try {
      const entry = await invoke<FullEntry>("edit_entry", { id });
      if (generation === epoch && selected === id) editor(entry);
    } catch (error) {
      if (generation === epoch) message(String(error));
    }
  });
}
function confirmDelete(entry: Entry) {
  if (!native || !status.unlocked || document.querySelector("dialog")) return;
  const generation = epoch;
  const dialog = document.createElement("dialog");
  dialog.setAttribute("aria-labelledby", "delete-title");
  dialog.innerHTML = `<div class="modal-heading"><h2 id="delete-title">Delete this login?</h2></div><p><strong>${esc(entry.title)}</strong></p><p>${esc(entry.username || "No username")}</p><p class="hint">This removes the login from your current vault and browser suggestions. Exported backups retain their copies. There is no undo inside this vault.</p><p id="delete-status" role="status"></p><div class="fill-actions"><button class="secondary" id="cancel-delete">Cancel</button><button class="danger" id="confirm-delete">Delete login</button></div>`;
  app.append(dialog);
  let deleting = false;
  const close = () => { if (!deleting) { dialog.remove(); document.querySelector<HTMLButtonElement>("#delete-login")?.focus(); } };
  dialog.querySelector("#cancel-delete")!.addEventListener("click", close);
  dialog.addEventListener("cancel", event => { event.preventDefault(); close(); });
  dialog.querySelector<HTMLButtonElement>("#confirm-delete")!.addEventListener("click", async () => {
    if (deleting) return;
    deleting = true;
    dialog.querySelectorAll<HTMLButtonElement>("button").forEach(button => { button.disabled = true; });
    try {
      const warning = await invoke<string | null>("delete_entry", {id: entry.id});
      if (generation === epoch || !document.querySelector("dialog")) {
        clearSecrets();
        await refresh();
      }
      message(`Login deleted.${warning ? ` ${warning}` : ""}`);
    } catch (error) {
      if (generation === epoch && dialog.isConnected) {
        dialog.querySelector("#delete-status")!.textContent = String(error);
        dialog.querySelectorAll<HTMLButtonElement>("button").forEach(button => { button.disabled = false; });
      } else message(String(error));
    } finally { deleting = false; }
  });
  dialog.showModal();
  dialog.querySelector<HTMLButtonElement>("#cancel-delete")!.focus();
}
function editor(entry?: FullEntry) {
  if (document.querySelector("dialog")) return;
  const origin = document.activeElement as HTMLElement | null;
  const dialog = document.createElement("dialog");
  dialog.setAttribute("aria-label", entry ? "Edit login" : "New login");
  dialog.className = "entry-dialog";
  dialog.innerHTML = `<form id="entry-form"><div class="modal-heading"><div class="editor-avatar">${accountAvatar(entry || {id: "", title: "New login", username: "", url: ""})}<div><h2>${entry ? "Edit login" : "New login"}</h2><p>${entry ? "Update the details for this login." : "Add a website or app to your vault."}</p></div></div><button type="button" id="cancel" class="icon-button" aria-label="Close">${icon("close")}</button></div><div class="editor-fields"><div class="form-row"><label class="field-name" for="entry-title">Title</label><div class="form-control"><input id="entry-title" name="title" required maxlength="200" autocomplete="off" placeholder="e.g. GitHub"></div></div><div class="form-row"><label class="field-name" for="entry-username">Username</label><div class="form-control"><input id="entry-username" name="username" autocomplete="off" placeholder="Username or email"></div></div><div class="form-row"><label class="field-name" for="entry-url">Website</label><div class="form-control"><input id="entry-url" name="url" placeholder="https://example.com" autocomplete="off"></div></div>${entry ? `<div class="password-summary"><span>${icon("key")}Password</span><span id="password-preserved" class="password-saved">Password saved</span><label class="secondary change-password-control"><input id="change-password" type="checkbox" aria-label="Change password"><span>Change password</span></label></div><p class="hint" id="keep-password-hint">Your saved password will stay unchanged.</p>` : ""}<fieldset id="password-fields" class="password-editor" ${entry ? "disabled hidden" : ""}><div class="form-row"><label class="field-name" for="entry-password">${entry ? "New password" : "Password"}</label><div class="form-control"><div class="generator-row">${passwordControl("entry-password", 'name="password" required autocomplete="new-password"', "password")}<button type="button" id="generate" class="secondary">${icon("refresh")}Generate</button></div></div></div><div class="form-row"><label class="field-name" for="entry-confirm">Confirm password</label><div class="form-control">${passwordControl("entry-confirm", 'name="confirmPassword" required autocomplete="new-password"', "password confirmation")}</div></div></fieldset><div class="form-row notes-row"><label class="field-name" for="entry-notes">Notes <small>(optional)</small></label><div class="form-control"><textarea id="entry-notes" name="notes" rows="3" placeholder="Add any notes about this login…"></textarea></div></div></div><div id="modal-error" role="status"></div><div class="modal-actions"><button type="button" id="cancel-editor" class="secondary">Cancel</button><button type="submit" class="primary" id="save">${entry ? "Save changes" : "Save login"}</button></div></form>`;
  app.append(dialog);
  const form = dialog.querySelector<HTMLFormElement>("form")!;
  attachPasswordControls(form);
  attachIconFallbacks(dialog);
  form.addEventListener("input", event => {
    const field = event.target as HTMLInputElement;
    if (!["title", "url"].includes(field.name)) return;
    const title = (form.elements.namedItem("title") as HTMLInputElement).value;
    const url = (form.elements.namedItem("url") as HTMLInputElement).value;
    const avatar = dialog.querySelector(".editor-avatar .avatar");
    if (avatar) avatar.outerHTML = accountAvatar({ id: entry?.id || "", title, url, username: "" });
    attachIconFallbacks(dialog.querySelector(".editor-avatar")!);
  });
  if (entry)
    for (const key of [
      "title",
      "username",
      "url",
      "notes",
    ] as const)
      (form.elements.namedItem(key) as HTMLInputElement).value = entry[key];
  const passwordFields = form.querySelector<HTMLFieldSetElement>("#password-fields")!;
  form.querySelector<HTMLInputElement>("#change-password")?.addEventListener("change", (event) => {
    const changing = (event.currentTarget as HTMLInputElement).checked;
    passwordFields.disabled = !changing;
    passwordFields.hidden = !changing;
    form.querySelector<HTMLElement>("#keep-password-hint")!.hidden = changing;
    passwordFields.querySelectorAll<HTMLInputElement>("input").forEach(field => { field.value = ""; field.type = "password"; });
    passwordFields.querySelectorAll<HTMLButtonElement>("[data-password-toggle]").forEach(button => {
      button.setAttribute("aria-pressed", "false");
      button.setAttribute("aria-label", `Show ${button.dataset.passwordLabel}`);
    });
    form.querySelector("#password-preserved")!.textContent = changing ? "Replacing password" : "Password saved";
    form.querySelector(".change-password-control > span")!.textContent = changing ? "Cancel change" : "Change password";
    (event.currentTarget as HTMLInputElement).setAttribute("aria-label", changing ? "Cancel change" : "Change password");
    if (changing) passwordFields.querySelector<HTMLInputElement>("input")?.focus();
  });
  const close = () => {
    form.reset();
    dialog.remove();
    if (origin?.isConnected) origin.focus();
  };
  dialog.querySelector("#cancel")!.addEventListener("click", close);
  dialog.querySelector("#cancel-editor")!.addEventListener("click", close);
  dialog.addEventListener("cancel", (event) => {
    event.preventDefault();
    close();
  });
  dialog.querySelector("#generate")!.addEventListener("click", async () => {
    const generation = epoch;
    try {
      const password = await invoke<string>("generate_password", {
        length: 24,
      });
      if (generation === epoch && dialog.isConnected && !passwordFields.disabled) {
        (form.elements.namedItem("password") as HTMLInputElement).value = password;
        (form.elements.namedItem("confirmPassword") as HTMLInputElement).value = password;
        passwordFields.querySelectorAll<HTMLInputElement>("input").forEach(field => { field.type = "password"; });
        passwordFields.querySelectorAll<HTMLButtonElement>("[data-password-toggle]").forEach(button => {
          button.setAttribute("aria-pressed", "false");
          button.setAttribute("aria-label", `Show ${button.dataset.passwordLabel}`);
        });
      }
    } catch (error) {
      if (generation === epoch && dialog.isConnected)
        dialog.querySelector("#modal-error")!.textContent = String(error);
    }
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const generation = epoch;
    let payload: ReturnType<typeof entryFormPayload>;
    try {
      payload = entryFormPayload(new FormData(form), entry?.id ?? null,
        form.querySelector<HTMLInputElement>("#change-password")?.checked ?? false);
    } catch (error) {
      dialog.querySelector("#modal-error")!.textContent = String(error);
      return;
    }
    const button = dialog.querySelector<HTMLButtonElement>("#save")!;
    button.disabled = true;
    try {
      await invoke("save_entry", payload);
      if (generation !== epoch) return;
      // A cancelled form must not close a newer dialog or discard its inputs.
      if (!dialog.isConnected) {
        if (!document.querySelector("dialog")) await refresh();
        return;
      }
      close();
      await refresh();
    } catch (error) {
      if (generation === epoch && dialog.isConnected) {
        dialog.querySelector("#modal-error")!.textContent = String(error);
        button.disabled = false;
      }
    }
  });
  dialog.showModal();
  dialog.querySelector<HTMLInputElement>('input[name="title"]')?.focus();
}
if (native) {
  await listen("browser-fill-request", () => { void refreshBrowserFill(); });
  window.setInterval(() => { void refreshBrowserFill(); }, 1000);
  await listen("vault-locked", () => {
    clearSecrets();
    status.unlocked = false;
    locked();
    // A creation may have reached disk before its response was revoked.
    // Re-read metadata, including after Windows unlocks, without retaining secrets.
    void refresh();
  });
  let lastTouch = 0;
  for (const event of ["pointerdown", "keydown"])
    document.addEventListener(event, () => {
      if (status.unlocked && Date.now() - lastTouch > 15000) {
        lastTouch = Date.now();
        void invoke("touch_vault").catch(() => {});
      }
    });
  await refresh();
} else locked();
