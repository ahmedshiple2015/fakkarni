/** Serialized by executeScript: keep this function self-contained. Runs in the isolated world. */
export function fillLogin(expectedUrl: string, expectedOrigin: string, username: string, password: string): string {
  if (window.top !== window || location.href !== expectedUrl || location.origin !== expectedOrigin ||
      location.protocol !== "https:") return "page_changed";
  const writable = (input: HTMLInputElement) => {
    const style = getComputedStyle(input);
    let ancestor: Element | null = input;
    while (ancestor) {
      if (Number(getComputedStyle(ancestor).opacity) === 0) return false;
      ancestor = ancestor.parentElement;
    }
    const bounds = input.getBoundingClientRect();
    return !input.disabled && !input.readOnly && !input.closest("[inert]") &&
      style.visibility === "visible" && style.display !== "none" &&
      bounds.width > 0 && bounds.height > 0 && input.getClientRects().length > 0;
  };
  const fields = Array.from(document.querySelectorAll<HTMLInputElement>('input[type="password"]')).filter(writable);
  // Never guess between a login, password change, or sign-up form.
  if (fields.length !== 1) return "ambiguous_form";
  const secret = fields[0];
  if (secret.autocomplete.split(/\s+/).includes("new-password")) return "ambiguous_form";
  const form = secret.form;
  if (!form) return "unsupported_form";
  const candidates = Array.from(form.elements).filter((item): item is HTMLInputElement =>
    item instanceof HTMLInputElement && ["text", "email", "tel"].includes(item.type) && writable(item));
  const named = candidates.filter(item => item.autocomplete.split(/\s+/).includes("username"));
  const likely = candidates.filter(item => item.type === "email" || /^(user(name)?|email|login)([_-]?(name|id|email))?$/i.test(item.name || item.id));
  const choices = named.length ? named : likely.length ? likely : candidates;
  if (choices.length !== 1) return "ambiguous_username";
  const account = choices[0];
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  if (!setter) return "unsupported_form";
  // Inspect everything before making either change. Never submit or click a page control.
  if (location.href !== expectedUrl || location.origin !== expectedOrigin) return "page_changed";
  setter.call(account, username);
  setter.call(secret, password);
  for (const input of [account, secret]) {
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  }
  return "filled";
}
