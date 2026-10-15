/** Bundled identification marks only. Resolving an icon never contacts a website. */
export interface BrandIcon { slug: string; name: string }

const brands: readonly (BrandIcon & { domains: readonly string[]; apps?: readonly string[] })[] = [
  { slug: "github", name: "GitHub", domains: ["github.com"] },
  { slug: "discord", name: "Discord", domains: ["discord.com", "discordapp.com"], apps: ["discord"] },
  { slug: "steam", name: "Steam", domains: ["steampowered.com", "steamcommunity.com"], apps: ["steam"] },
  { slug: "riotgames", name: "Riot Games", domains: ["riotgames.com", "leagueoflegends.com", "playvalorant.com"], apps: ["riot games", "riot client", "league of legends", "valorant"] },
  { slug: "proton", name: "Proton", domains: ["proton.me", "protonmail.com", "protonvpn.com"] },
  { slug: "google", name: "Google", domains: ["google.com", "gmail.com"] },
  { slug: "youtube", name: "YouTube", domains: ["youtube.com", "youtu.be"] },
  { slug: "reddit", name: "Reddit", domains: ["reddit.com"] },
  { slug: "twitch", name: "Twitch", domains: ["twitch.tv"] },
  { slug: "spotify", name: "Spotify", domains: ["spotify.com"], apps: ["spotify"] },
  { slug: "facebook", name: "Facebook", domains: ["facebook.com"] },
  { slug: "instagram", name: "Instagram", domains: ["instagram.com"] },
  { slug: "telegram", name: "Telegram", domains: ["telegram.org", "t.me"], apps: ["telegram", "telegram desktop"] },
  { slug: "whatsapp", name: "WhatsApp", domains: ["whatsapp.com"], apps: ["whatsapp"] },
  { slug: "netflix", name: "Netflix", domains: ["netflix.com"] },
  { slug: "gitlab", name: "GitLab", domains: ["gitlab.com"] },
  { slug: "bitwarden", name: "Bitwarden", domains: ["bitwarden.com"], apps: ["bitwarden"] },
  { slug: "dropbox", name: "Dropbox", domains: ["dropbox.com"], apps: ["dropbox"] },
  { slug: "epicgames", name: "Epic Games", domains: ["epicgames.com"], apps: ["epic games", "epic games launcher"] },
];

/** A supplied website always wins over the title, including unknown or invalid URLs. */
export function resolveBrandIcon(url: string, title = ""): BrandIcon | null {
  if (/[\u0000-\u001f\u007f]/.test(url)) return null;
  const supplied = url.trim();
  let match: typeof brands[number] | undefined;
  if (supplied) {
    // Require an explicit normal URL: do not reinterpret malformed/bare domains or backslashes.
    if (!/^https?:\/\//i.test(supplied) || /[\\\u0000-\u0020\u007f]/.test(supplied)) return null;
    let website: URL;
    try { website = new URL(supplied); } catch { return null; }
    if (website.username || website.password || !website.hostname) return null;
    const host = website.hostname.toLowerCase();
    match = brands.find(brand => brand.domains.some(domain => host === domain || host.endsWith(`.${domain}`)));
  } else {
    const app = title.trim().normalize("NFKC").toLocaleLowerCase("en-US").replace(/\s+/gu, " ");
    match = brands.find(brand => brand.apps?.includes(app));
  }
  return match ? { slug: match.slug, name: match.name } : null;
}

/** Preserve a complete Unicode grapheme, including emoji and combining marks. */
export function initialFor(title: string): string {
  const label = title.replace(/[\u0000-\u001f\u007f\u200e\u200f\u202a-\u202e\u2066-\u2069]/gu, "").trim();
  if (!label) return "?";
  const first = new Intl.Segmenter(undefined, { granularity: "grapheme" }).segment(label)[Symbol.iterator]().next().value?.segment;
  return first ? first.toLocaleUpperCase() : "?";
}
