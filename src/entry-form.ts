/** Existing passwords are never submitted during an ordinary metadata edit. */
export function entryFormPayload(data: FormData, id: string | null, changePassword: boolean) {
  const replacePassword = id === null || changePassword;
  const password = replacePassword ? String(data.get("password") ?? "") : "";
  if (replacePassword) {
    if (!password) throw new Error("Enter a password.");
    if (password !== String(data.get("confirmPassword") ?? "")) {
      throw new Error("The password confirmation does not match.");
    }
  }
  return {
    replacePassword,
    input: {
      id,
      title: String(data.get("title") ?? ""),
      username: String(data.get("username") ?? ""),
      url: String(data.get("url") ?? ""),
      password,
      notes: String(data.get("notes") ?? ""),
    },
  };
}
