// Fixed local SVGs: no remote icon or favicon requests.
const paths = {
  close: '<path d="m6 6 12 12M18 6 6 18"/>',
  folder: '<path d="M3 7V5h6l2 2h10v13H3V7Z"/>',
  download: '<path d="M12 3v12m-4-4 4 4 4-4M4 16v5h16v-5"/>',
  upload: '<path d="M12 16V4m-4 4 4-4 4 4M4 16v5h16v-5"/>',
  refresh: '<path d="M20 7v5h-5M4 17v-5h5M6 7a7 7 0 0 1 12-1l2 3M4 15l2 3a7 7 0 0 0 12-1"/>',
  fingerprint: '<path d="M6 19c1-2 1-4 1-7a5 5 0 0 1 10 0c0 4-1 7-2 9M4 15v-3a8 8 0 0 1 16 0m-9 9c1-3 2-6 2-9a1 1 0 0 0-2 0c0 3 0 5-1 7M7 5a8 8 0 0 1 10 0M3 8a10 10 0 0 1 18 0"/>',
  keyhole: '<path fill="currentColor" stroke="none" d="M12 2a6 6 0 0 0-3.5 10.9L6 22h12l-2.5-9.1A6 6 0 0 0 12 2Z"/>',
  monitor: '<rect x="3" y="4" width="18" height="13" rx="1"/><path d="M8 21h8m-4-4v4"/>',
  shield: '<path d="M12 3 4 6v6c0 5 8 9 8 9s8-4 8-9V6l-8-3Z"/><path d="m8 12 3 3 5-6"/>',
  key: '<circle cx="8" cy="9" r="5"/><path d="m12 13 8 8m-4-4 3-3m-6 0 3-3"/>',
  settings: '<path d="M4 7h16M4 17h16"/><circle cx="9" cy="7" r="3"/><circle cx="15" cy="17" r="3"/>',
  lock: '<rect x="5" y="10" width="14" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3m-4 5v2"/>',
  search: '<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 4 4"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  arrow: '<path d="M5 12h14m-6-6 6 6-6 6"/>',
  globe: '<circle cx="12" cy="12" r="9"/><ellipse cx="12" cy="12" rx="4" ry="9"/><path d="M3 12h18"/>',
  eye: '<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/>',
  edit: '<path d="m15 4 5 5-11 11-6 1 1-6L15 4Zm-9 9 5 5m2-12 5 5"/>',
  trash: '<path d="M4 7h16M9 7V4h6v3m-9 0 1 14h10l1-14M10 11v6m4-6v6"/>',
  chevron: '<path d="m9 6 6 6-6 6"/>',
} as const;
export function icon(name: keyof typeof paths) {
  return `<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">${paths[name]}</svg>`;
}
