/** Runs in the extension's isolated world on top-level HTTPS pages. */
(() => {
  if (window.top !== window || location.protocol !== "https:" || !chrome?.runtime) return;
  const runtime = chrome.runtime;
  let typed = new WeakMap<HTMLInputElement, string>();
  window.addEventListener("pagehide", () => { typed = new WeakMap(); });
  const visible = (input: HTMLInputElement) => !input.matches(":disabled") && !input.readOnly &&
    !input.closest("[inert]") && input.getClientRects().length > 0 && getComputedStyle(input).visibility === "visible";
  const tokens = (input: HTMLInputElement) => input.autocomplete.toLowerCase().split(/\s+/);
  const name = (input: HTMLInputElement) => (input.name || input.id)
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase();
  const role = (input: HTMLInputElement) => {
    if (input.type !== "text" && input.type !== "password") return "";
    const id = name(input);
    if (/(?:confirm|repeat|retype|verify)[_-]?(?:password|passwd|pwd)|(?:password|passwd|pwd)[_-]?(?:confirm|compare|repeat|again|2)$/.test(id)) return "confirm";
    if (tokens(input).includes("new-password") || /^new[_-]?(?:password|passwd|pwd)$/.test(id)) return "new";
    if (tokens(input).includes("current-password") || /^(?:old|current)[_-]?(?:password|passwd|pwd)$/.test(id)) return "current";
    if (input.type === "password" || (input.type === "text" && /^(?:login[_-]?)?(?:password|passwd|pwd)$/.test(id))) return "login";
    return "";
  };
  const options = { capture: true };
  document.addEventListener("input", event => {
    const field = event.target;
    if (!event.isTrusted || !(field instanceof HTMLInputElement) || !role(field)) return;
    if (field.value && field.value.length <= 1024) typed.set(field, field.value);
    else typed.delete(field);
  }, options);
  function captureForm(form: HTMLFormElement, submitter: HTMLElement | null) {
    const action = submitter?.getAttribute("formaction") || form.getAttribute("action") || location.href;
    const method = submitter?.getAttribute("formmethod") || form.getAttribute("method") || "get";
    const target = submitter?.getAttribute("formtarget") || form.getAttribute("target") || document.querySelector("base[target]")?.getAttribute("target") || "_self";
    try {
      const destination = new URL(action, document.baseURI);
      if (destination.origin !== location.origin || destination.username || destination.password ||
          method.toLowerCase() !== "post" || target.toLowerCase() !== "_self") return;
    } catch { return; }
    const controls = Array.from(form.elements);
    if (controls.some(field => (field instanceof HTMLInputElement || field instanceof HTMLSelectElement ||
        field instanceof HTMLTextAreaElement) && field.willValidate && !field.validity.valid)) return;
    const fields = controls.filter((field): field is HTMLInputElement =>
      field instanceof HTMLInputElement && visible(field));
    if (fields.some(field => field.getAttribute("aria-invalid") === "true")) return;
    const secrets = fields.filter(field => role(field));
    const usernames = fields.filter(field => !role(field) && ["text", "email", "tel"].includes(field.type)
      && !tokens(field).includes("one-time-code"));
    const marked = usernames.filter(field => tokens(field).some(t => t === "username" || t === "email"));
    const isUsername = (field: HTMLInputElement) => tokens(field).includes("username") ||
      /^(?:user(?:name)?|login)(?:[_-]?(?:name|id))?$/.test(name(field));
    const isEmail = (field: HTMLInputElement) => field.type === "email" || tokens(field).includes("email") ||
      /^(?:email|login[_-]?email)$/.test(name(field));
    const likely = usernames.filter(field => isUsername(field) || isEmail(field));
    const choices = marked.length ? marked : likely;
    const fresh = secrets.filter(field => role(field) === "new");
    const confirm = secrets.filter(field => role(field) === "confirm");
    let identifier = choices[0];
    if (choices.length > 1) {
      const handles = choices.filter(field => isUsername(field) && !isEmail(field));
      const emails = choices.filter(field => isEmail(field) && !isUsername(field));
      if (!(fresh.length || confirm.length) || choices.length !== 2 ||
          handles.length !== 1 || emails.length !== 1) return;
      identifier = handles[0];
    }
    const username = identifier?.value.trim() ?? "";
    const current = secrets.filter(field => role(field) === "current");
    const login = secrets.filter(field => role(field) === "login");
    let password: HTMLInputElement | undefined;
    let changeForm = false;
    if (fresh.length && fresh.length <= 2 && confirm.length <= 1 && current.length <= 1 && !login.length) {
      password = fresh[0];
      if ([...fresh, ...confirm].some(field => field.value !== password!.value)) return;
      changeForm = current.length > 0 || !username;
    } else if (login.length + current.length === 1 && !fresh.length && confirm.length <= 1) {
      password = login[0] ?? current[0];
      if (current.length && /^old[_-]?/.test(name(password))) return;
      if (confirm.some(field => field.value !== password!.value)) return;
    }
    if (!password?.value || typed.get(password) !== password.value || (!username && !changeForm)) return;
    const encoder = new TextEncoder();
    if (encoder.encode(username).length > 512 || encoder.encode(password.value).length > 1024) return;
    for (const field of secrets) typed.delete(field);
    void runtime.sendMessage({action: "capture_candidate", username,
      password: password.value, changeForm}).catch(() => {});
  }
  document.addEventListener("submit", event => {
    if (event.isTrusted && event.target instanceof HTMLFormElement) {
      captureForm(event.target, event instanceof SubmitEvent ? event.submitter : null);
    }
  }, options);
  // Some AJAX forms intercept the submit button and never dispatch a DOM submit.
  document.addEventListener("click", event => {
    if (!event.isTrusted || !(event.target instanceof Element)) return;
    const button = event.target.closest("button,input");
    if ((button instanceof HTMLButtonElement || button instanceof HTMLInputElement) &&
        button.type === "submit" && !button.matches(":disabled") && button.form) {
      captureForm(button.form, button);
    }
  }, options);
})();
