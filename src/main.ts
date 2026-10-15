import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./style.css";

type Status = {
  exists: boolean;
  unlocked: boolean;
  path: string;
  idleMinutes: number;
};
type Entry = { id: string; title: string; username: string; url: string };
type FullEntry = Entry & { password: string; notes: string };
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
  if (native) document.querySelector<HTMLInputElement>("#master")?.focus();
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
    const next = await invoke<Status>("vault_status");
    if (generation !== epoch) return;
    status = next;
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
  dialog.innerHTML = `<form id="entry-form"><div class="modal-heading"><div><span class="eyebrow">YOUR PRIVATE VAULT</span><h2>${entry ? "Edit login" : "New login"}</h2></div><button type="button" id="cancel" class="text-button" aria-label="Close">✕</button></div><label>Title<input name="title" required maxlength="200" autocomplete="off"></label><label>Username<input name="username" autocomplete="off"></label><label>Website<input name="url" placeholder="https://example.com" autocomplete="off"></label><label>Password<div class="generator-row"><input name="password" type="password" required autocomplete="new-password"><button type="button" id="generate" class="secondary">Generate</button></div></label><label>Notes<textarea name="notes" rows="3"></textarea></label><div id="modal-error" role="status"></div><button class="primary wide" id="save">Save login</button></form>`;
  app.append(dialog);
  const form = dialog.querySelector<HTMLFormElement>("form")!;
  if (entry)
    for (const key of [
      "title",
      "username",
      "url",
      "password",
      "notes",
    ] as const)
      (form.elements.namedItem(key) as HTMLInputElement).value = entry[key];
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
      if (generation === epoch && dialog.isConnected)
        (form.elements.namedItem("password") as HTMLInputElement).value =
          password;
    } catch (error) {
      if (generation === epoch && dialog.isConnected)
        dialog.querySelector("#modal-error")!.textContent = String(error);
    }
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const generation = epoch;
    const data = new FormData(form);
    const input = {
      id: entry?.id ?? null,
      title: String(data.get("title")),
      username: String(data.get("username")),
      url: String(data.get("url")),
      password: String(data.get("password")),
      notes: String(data.get("notes")),
    };
    const button = dialog.querySelector<HTMLButtonElement>("#save")!;
    button.disabled = true;
    try {
      await invoke("save_entry", { input });
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
