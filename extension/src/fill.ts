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
  const tokens = (input: HTMLInputElement) => input.autocomplete.toLowerCase().split(/\s+/);
  const identifiers = (input: HTMLInputElement) => [input.name, input.id]
    .map(value => value.replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase());
  const passwordField = (input: HTMLInputElement) => {
    if (input.type === "password") return true;
    if (input.type !== "text") return false;
    const hints = tokens(input);
    if (hints.some(hint => ["username", "email", "one-time-code"].includes(hint))) return false;
    // Reveal controls commonly switch the same input to text. Require explicit
    // password semantics; never treat an arbitrary visible text field as a secret.
    return hints.includes("current-password") || hints.includes("new-password") ||
      identifiers(input).some(value => /^(?:(?:login|current|account|user|new|confirm|repeat|retype)[^a-z0-9]+)?(?:password|passwd|pwd)$/.test(value));
  };
  const fields = Array.from(document.querySelectorAll<HTMLInputElement>("input")).filter(input => passwordField(input) && writable(input));
  // Never guess between a login, password change, or sign-up form.
  if (fields.length !== 1) return "ambiguous_form";
  const secret = fields[0];
  if (tokens(secret).includes("new-password") || identifiers(secret).some(value =>
    /(?:^|[^a-z0-9])(?:new|confirm|repeat|retype)[^a-z0-9]*(?:password|passwd|pwd)(?:$|[^a-z0-9])/.test(value))) return "ambiguous_form";
  const form = secret.form;
  if (!form) return "unsupported_form";
  const candidates = Array.from(form.elements).filter((item): item is HTMLInputElement =>
    item instanceof HTMLInputElement && item !== secret && !passwordField(item) &&
      ["text", "email", "tel"].includes(item.type) && writable(item));
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
