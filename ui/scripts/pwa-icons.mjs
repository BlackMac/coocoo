// Renders the web app icons in public/icons from the app icon (src-tauri/icons/icon.svg, drawn on
// the macOS icon grid: an 824px rounded square inset 100px in 1024px). Run: npm run icons
//
// - icon-*.png (purpose "any"): the macOS-shaped icon with its transparent margin. Browsers on
//   macOS show "any" icons as they are, so the Dock icon matches native apps.
// - maskable-*.png: full-bleed background, same glyph. macOS/Android/ChromeOS mask these into the
//   platform shape; the glyph stays inside the 80% safe zone.
// - apple-touch-icon.png: full-bleed 180px for the iOS home screen (iOS rounds the corners).

import { readFileSync, writeFileSync } from 'node:fs';
import { Resvg } from '@resvg/resvg-js';

const root = new URL('..', import.meta.url);
const svg = readFileSync(new URL('../src-tauri/icons/icon.svg', root), 'utf8');

// Full-bleed variant: the background rect fills the canvas, without corners or hairline outline.
const fullBleed = svg
  .replace(/<rect x="100" y="100" width="824" height="824" rx="186"/, '<rect x="0" y="0" width="1024" height="1024"')
  .replace(/\s*<rect x="101"[^>]*\/>/, '');
if (fullBleed === svg) throw new Error('icon.svg changed shape; update scripts/pwa-icons.mjs');

function png(source, size, file) {
  const out = new Resvg(source, { fitTo: { mode: 'width', value: size } }).render().asPng();
  writeFileSync(new URL(`public/icons/${file}`, root), out);
  console.log(`public/icons/${file} ${size}px ${out.length} B`);
}

writeFileSync(new URL('public/icons/icon.svg', root), svg);
png(svg, 192, 'icon-192.png');
png(svg, 512, 'icon-512.png');
png(svg, 1024, 'icon-1024.png');
png(fullBleed, 512, 'maskable-512.png');
png(fullBleed, 1024, 'maskable-1024.png');
png(fullBleed, 180, 'apple-touch-icon.png');
