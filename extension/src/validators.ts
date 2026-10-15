export type Match = {
  id: string;
  title: string;
  username: string;
  url: string;
};
export type MatchResponse = {
  version: 1;
  status: "ok" | "locked" | "unavailable" | "error" | "pending" | "cancelled";
  matches: Match[];
};

/** Use only a URL supplied by tabs.query, never a page-provided URL. */
export function httpsOrigin(value: unknown): string | null {
  if (typeof value !== "string" || value.length > 16384) return null;
  try {
    const url = new URL(value);
    if (
      url.protocol !== "https:" ||
      !url.hostname ||
      url.username ||
      url.password
    )
      return null;
    return url.origin;
  } catch {
    return null;
  }
}

const record = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === "object" && !Array.isArray(value);
const text = (value: unknown, max: number): value is string =>
  typeof value === "string" &&
  value.length <= max &&
  !/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(value);

export function validateResponse(value: unknown): MatchResponse | null {
  if (
    !record(value) ||
    value.version !== 1 ||
    typeof value.status !== "string" ||
    !["ok", "locked", "unavailable", "error", "pending", "cancelled"].includes(value.status) ||
    !Array.isArray(value.matches) ||
    value.matches.length > 50
  )
    return null;
  if (!["ok", "locked"].includes(value.status) && value.matches.length !== 0) return null;
  const matches: Match[] = [];
  for (const entry of value.matches) {
    if (
      !record(entry) ||
      !text(entry.id, 128) ||
      !entry.id ||
      !text(entry.title, 256) ||
      !text(entry.username, 4096) ||
      !text(entry.url, 2048)
    )
      return null;
    // Copy only these fields. A host response can never add passwords to UI state.
    matches.push({
      id: entry.id,
      title: entry.title,
      username: entry.username,
      url: entry.url,
    });
  }
  return {
    version: 1,
    status: value.status as MatchResponse["status"],
    matches,
  };
}

/** Credentials are accepted only by the background's approved poll path. */
export function validateCredential(value: unknown): { username: string; password: string } | null {
  if (!record(value) || typeof value.username !== "string" || typeof value.password !== "string" ||
      value.username.length > 4096 || value.password.length > 4096 || !value.password.length ||
      value.username.includes("\0") || value.password.includes("\0")) return null;
  return { username: value.username, password: value.password };
}
