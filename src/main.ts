import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./style.css";
import { entryFormPayload } from "./entry-form";

type Status = {
  exists: boolean;
  unlocked: boolean;
  path: string;
  idleMinutes: number;
};
type Entry = { id: string; title: string; username: string; url: string };
type FullEntry = Entry & { password: string; notes: string };
type HelloStatus = { available: boolean; enabled: boolean; reason: string | null };
let hello: HelloStatus = { available: false, enabled: false, reason: "Windows Hello is available only in the desktop app." };
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
const shield =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6"><path d="M12 3 4 6v6c0 5 8 9 8 9s8-4 8-9V6l-8-3Z"/><path d="m8 12 3 3 5-6"/></svg>';
const brand = `<div class="brand"><span class="brand-icon">${shield}</span><div>Fakkarni <span lang="ar" dir="rtl">فكرني</span><small>YOUR PASSWORDS. YOUR PC.</small></div></div>`;
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
  app.replaceChildren();
}
function shell(content: string) {
  app.innerHTML = `<aside>${brand}<div class="nav-label">WORKSPACE</div><div class="nav active">${shield} All passwords <span>${entries.length || "—"}</span></div><div class="sidebar-bottom"><span class="local-dot"></span> Stored on this device<small>No cloud. No tracking. Just yours.</small></div></aside><main>${content}</main>`;
}
function locked() {
  shell(
    `<div class="topbar"><span>Personal vault</span><span class="pill">${native ? "LOCAL VAULT" : "DESIGN PREVIEW"}</span></div><section class="welcome"><div class="large-shield">${shield}</div><div class="eyebrow">A LITTLE PEACE OF MIND</div><h1>${status.exists ? "Welcome back." : "Remember less.<br>Keep more safe."}</h1><p>${native ? (status.exists ? "Unlock your vault to pick up where you left off." : "One master password. A private home for all your logins.") : "This browser preview has no vault connection. Open the desktop app to create or unlock your encrypted vault."}</p><form id="unlock"><label>Master password<input id="master" type="password" minlength="${status.exists ? 1 : 12}" required autocomplete="${status.exists ? "current-password" : "new-password"}" ${native ? "" : "disabled"} placeholder="${status.exists ? "Enter your master password" : "At least 12 characters"}"></label>${!status.exists ? `<label>Confirm master password<input id="confirm" type="password" required autocomplete="new-password" ${native ? "" : "disabled"} placeholder="Repeat your master password"></label><small class="hint">Keep it somewhere safe. There is no password reset.</small>` : ""}<button class="primary wide" ${native ? "" : "disabled"}>${status.exists ? "Unlock vault" : "Create my vault"} <span>→</span></button><div id="message" role="status"></div></form><div class="privacy">${shield} Encrypted locally. Always under your control.</div></section>`,
  );
  const unlockForm = document.querySelector<HTMLFormElement>("#unlock")!;
  renderBrowserFill();
  if (status.exists && hello.enabled) {
    const button = document.createElement("button");
    button.id = "hello-unlock";
    button.className = "primary wide hello-unlock";
    button.textContent = "Unlock with Windows Hello";
    button.disabled = !native || !hello.available;
    unlockForm.before(button);
    const fallback = document.createElement("p");
    fallback.className = "hint";
    fallback.textContent = "Or use your master password as a recovery fallback.";
    unlockForm.prepend(fallback);
    button.addEventListener("click", async () => {
      if (!native) return;
      const generation = epoch;
      button.disabled = true;
      message("Complete the Windows Hello prompt…");
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
    ? "You can enable Windows Hello after creating your vault. Keep your master password for recovery."
    : hello.reason || (hello.enabled ? "Windows handles your PIN, fingerprint, or face. Fakkarni never receives your Windows PIN." : "Enable Windows Hello from inside your vault for quicker unlocking.");
  unlockForm.after(helper);
  if (native) {
    const restore = document.createElement("button");
    restore.className = "text-button";
    restore.type = "button";
    restore.textContent = "Import or restore an encrypted vault";
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
        document.querySelector<HTMLButtonElement>("#unlock button")!;
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
        available: false, enabled: false, reason: `Windows Hello is unavailable: ${String(error)}`,
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
    `<header class="topbar"><div><span class="eyebrow">PERSONAL VAULT</span><h2>All passwords <span class="count">${entries.length}</span></h2></div><button id="lock" class="secondary">Lock vault</button></header><div class="toolbar"><input id="search" type="search" placeholder="Search your vault…" aria-label="Search passwords"><button id="add" class="primary">＋ New login</button></div><div class="vault-grid"><section class="entry-list" aria-label="Saved passwords"></section><section id="detail" class="detail"><div class="empty"><div class="large-shield">${shield}</div><h3>${entries.length ? "A place for every login." : "Your vault starts here."}</h3><p>${entries.length ? "Select an account to see its details." : "Add your first login and give your memory a break."}</p></div></section></div><footer>${shield} End-to-device privacy <span>Auto-lock after ${status.idleMinutes} minutes of inactivity</span></footer><div id="message" role="status"></div>`,
  );
  const settings = document.createElement("section");
  settings.className = "hello-settings";
  settings.setAttribute("aria-label", "Windows Hello settings");
  settings.innerHTML = `<div><strong>Windows Hello</strong><p>${esc(hello.enabled ? "Enabled on this PC. Your master password remains your recovery fallback." : hello.reason || "Unlock with your Windows PIN, fingerprint, or face after setup.")}</p></div><button class="secondary" id="hello-settings" ${native && (hello.available || hello.enabled) ? "" : "disabled"}>${hello.enabled ? "Disable Windows Hello" : "Enable Windows Hello"}</button>`;
  document.querySelector("footer")!.before(settings);
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
  document.querySelector<HTMLInputElement>("#search")?.focus();
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
  panel.innerHTML = `<span class="eyebrow">BROWSER AUTOFILL REQUEST</span><h2>${esc(request.title || "Saved account")}</h2><p class="fill-origin">${esc(request.origin)}</p><p>${esc(request.username || "No username")}</p><p class="hint">${status.unlocked ? "Filling the selected account in your original browser tab…" : "Unlock below with your master password or Windows Hello. The selected account will fill automatically in the original browser tab. No form will be submitted."}</p><button class="secondary" id="reject-browser-fill">Cancel fill</button>`;
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
  section.innerHTML = `<h3>Browser autofill</h3><p class="hint">Select an account in Brave or Edge to fill it immediately when unlocked. If locked, Fakkarni comes forward and fills after you unlock. Autofill never submits a form.</p><div class="desktop-toggles"><label><input type="checkbox" id="locked-suggestions" ${browserSettings.showAccountsWhenLocked ? "checked" : ""} ${browserSettingsError ? "disabled" : ""}><span>Show saved accounts in the browser while my vault is locked</span></label></div><p class="hint">When enabled, account names, usernames, and website origins are stored in a separate unencrypted file on this PC, so they remain visible while locked and to programs running as your Windows user. Passwords and notes stay encrypted. Turning this off removes the local index.</p><p id="browser-preferences-status" class="hint" role="status">${esc(browserSettingsError || "")}</p>`;
  document.querySelector("footer")!.before(section);
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
  section.className = "desktop-settings";
  section.setAttribute("aria-label", "Backup and PC transfer");
  section.innerHTML = `<h3>Backup &amp; PC transfer</h3><p class="hint">Export a local encrypted copy for backup or to transfer to another PC. You need the master password used when the backup was made; Windows Hello alone cannot open it on another PC. Choose a folder outside cloud sync if you want it to stay on this device.</p><div class="fill-actions"><button class="secondary" id="create-backup">Export encrypted vault</button><button class="text-button" id="restore-backup">Import / restore vault</button></div><p class="hint">Restore validates the selected file before replacing your vault and preserves an encrypted recovery copy of the current file. Restore ends the current session and resets device unlock and the browser account index.</p><p id="backup-status" role="status"></p>`;
  document.querySelector("footer")!.before(section);
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
      form.innerHTML = `<div class="modal-heading"><h2>Review import</h2></div><p><strong>${esc(result.fileName)}</strong></p><p>${result.entryCount} saved ${result.entryCount === 1 ? "login" : "logins"}</p><p class="hint">${result.hasExistingVault ? "This will replace the current vault. An encrypted recovery copy must be saved successfully before replacement." : "This will create the local vault from this backup."} The restored vault stays locked. Use the backup's master password to reopen it and set up Windows Hello again if wanted.</p><div class="desktop-toggles"><label><input type="checkbox" id="confirm-restore"><span>I understand that restoring replaces the current vault; it does not merge logins.</span></label></div><p id="restore-status" role="status"></p><div class="fill-actions"><button type="button" id="restore-back" class="secondary">Cancel</button><button type="button" id="commit-restore" class="primary" disabled>Import this vault</button></div>`;
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
  const choices: [keyof AppSettings, string][] = [
    ["startWithWindows", "Start with Windows"],
    ["closeToTray", "Keep running in system tray when I close the window"],
    ["startMinimized", "Open in system tray"],
  ];
  section.innerHTML = `<h3 id="desktop-settings-title">Desktop preferences</h3><p class="hint">Brave suggestions require Fakkarni to stay running. Auto-lock still applies. Closing to the tray keeps your current session until it locks automatically; use Lock vault to lock immediately.</p><div class="desktop-toggles">${choices.map(([key, label]) => `<label><input type="checkbox" name="${key}" ${appSettings[key] ? "checked" : ""} ${disabled ? "disabled" : ""}><span>${label}</span></label>`).join("")}</div><p class="hint" id="startup-hint" ${appSettings.startWithWindows ? "" : "hidden"}>Windows startup uses this app's current executable location. Keep it in place; this build has no installer.</p><p class="hint" id="desktop-settings-status" role="status">${esc(settingsError || (!native ? "Desktop preferences are available in the desktop app." : ""))}</p>`;
  document.querySelector("footer")!.before(section);
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
  dialog.innerHTML = `<form><div class="modal-heading"><h2 id="hello-title">Enable Windows Hello</h2><button type="button" id="hello-cancel" class="text-button" aria-label="Close">✕</button></div><p id="hello-description" class="hint">Windows Hello protects quick unlock on this PC. Keep your master password: you need it for recovery or another PC. Windows will ask for your PIN, fingerprint, or face in its own prompt. Fakkarni never receives your Windows PIN.</p><label>Confirm master password<input name="password" type="password" required autocomplete="current-password" placeholder="Enter your master password"></label><div id="hello-error" role="status"></div><button class="primary wide">Continue to Windows Hello</button></form>`;
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
    dialog.querySelector("#hello-error")!.textContent = "Complete the Windows Hello prompt…";
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
          `<button class="entry ${selected === entry.id ? "selected" : ""}" data-id="${esc(entry.id)}"><span class="avatar">${esc((entry.title || "?").slice(0, 1).toUpperCase())}</span><span><strong>${esc(entry.title)}</strong><small>${esc(entry.username || "No username")}</small></span><span class="chevron">›</span></button>`,
      )
      .join("") || '<p class="list-empty">No logins found.</p>';
  document
    .querySelectorAll<HTMLButtonElement>(".entry")
    .forEach((el) =>
      el.addEventListener("click", () => detail(el.dataset.id!)),
    );
}
function detail(id: string) {
  const entry = entries.find((e) => e.id === id);
  if (!entry) return;
  window.clearTimeout(hideTimer);
  selected = id;
  renderList();
  document.querySelectorAll<HTMLButtonElement>(".entry").forEach((button) => {
    const active = button.dataset.id === id;
    button.setAttribute("aria-pressed", String(active));
    if (active) button.focus();
  });
  document.querySelector("#detail")!.innerHTML =
    `<div class="detail-heading"><span class="avatar big">${esc((entry.title || "?").slice(0, 1).toUpperCase())}</span><h2>${esc(entry.title)}</h2><p>LOGIN DETAILS</p></div><div class="field"><label>Username</label><div>${esc(entry.username || "—")}</div></div><div class="field"><label>Password</label><div class="password-row"><span id="password">••••••••••••</span><button id="reveal" class="text-button">Reveal</button></div></div><div class="field"><label>Website</label><div class="url">${esc(entry.url || "—")}</div></div><button id="edit" class="secondary wide">Edit login</button><p class="hint">Passwords hide automatically after 15 seconds.</p>`;
  document.querySelector("#reveal")!.addEventListener("click", async () => {
    const generation = epoch;
    const target = document.querySelector("#password")!;
    const button = document.querySelector<HTMLButtonElement>("#reveal")!;
    if (button.textContent === "Hide") {
      target.textContent = "••••••••••••";
      button.textContent = "Reveal";
      window.clearTimeout(hideTimer);
      return;
    }
    button.disabled = true;
    try {
      const password = await invoke<string>("reveal_password", { id });
      if (generation !== epoch || selected !== id || !target.isConnected)
        return;
      target.textContent = password;
      button.textContent = "Hide";
      hideTimer = window.setTimeout(() => {
        target.textContent = "••••••••••••";
        button.textContent = "Reveal";
      }, 15000);
    } catch (error) {
      if (generation === epoch && selected === id) message(String(error));
    } finally {
      if (generation === epoch && selected === id && button.isConnected)
        button.disabled = false;
    }
  });
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
function editor(entry?: FullEntry) {
  if (document.querySelector("dialog")) return;
  const origin = document.activeElement as HTMLElement | null;
  const dialog = document.createElement("dialog");
  dialog.setAttribute("aria-label", entry ? "Edit login" : "New login");
  dialog.innerHTML = `<form id="entry-form"><div class="modal-heading"><div><span class="eyebrow">YOUR PRIVATE VAULT</span><h2>${entry ? "Edit login" : "New login"}</h2></div><button type="button" id="cancel" class="text-button" aria-label="Close">✕</button></div><label>Title<input name="title" required maxlength="200" autocomplete="off"></label><label>Username<input name="username" autocomplete="off"></label><label>Website<input name="url" placeholder="https://example.com" autocomplete="off"></label>${entry ? `<div class="desktop-toggles"><label><input id="change-password" type="checkbox"><span>Change password</span></label></div><p class="hint" id="keep-password-hint">Your saved password will stay unchanged.</p>` : ""}<fieldset id="password-fields" class="password-editor" ${entry ? "disabled hidden" : ""}><label>${entry ? "New password" : "Password"}<div class="generator-row"><input name="password" type="password" required autocomplete="new-password"><button type="button" id="generate" class="secondary">Generate</button></div></label><label>Confirm password<input name="confirmPassword" type="password" required autocomplete="new-password"></label></fieldset><label>Notes<textarea name="notes" rows="3"></textarea></label><div id="modal-error" role="status"></div><button class="primary wide" id="save">Save login</button></form>`;
  app.append(dialog);
  const form = dialog.querySelector<HTMLFormElement>("form")!;
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
    passwordFields.querySelectorAll<HTMLInputElement>("input").forEach(field => { field.value = ""; });
    if (changing) passwordFields.querySelector<HTMLInputElement>("input")?.focus();
  });
  const close = () => {
    form.reset();
    dialog.remove();
    if (origin?.isConnected) origin.focus();
  };
  dialog.querySelector("#cancel")!.addEventListener("click", close);
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
