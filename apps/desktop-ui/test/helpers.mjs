// Shared test helpers: the app source (all shipped modules) and the catalogs.
import { readFile, readdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const SRC = new URL('../src/', import.meta.url);

async function collect(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const url = new URL(`${entry.name}${entry.isDirectory() ? '/' : ''}`, dir);
    if (entry.isDirectory()) files.push(...await collect(url));
    else if (/\.(jsx|mjs|js)$/.test(entry.name)) files.push(url);
  }
  return files;
}

/** Concatenated source of every shipped module under `src/`. */
export async function readAppSource() {
  const files = await collect(SRC);
  const parts = await Promise.all(files.map(async url => `\n/* ${fileURLToPath(url)} */\n${await readFile(url, 'utf8')}`));
  return parts.join('');
}

/** Concatenated source of the rendered region components only. */
export async function readComponentsSource() {
  const files = await collect(new URL('components/', SRC));
  const parts = await Promise.all(files.map(async url => await readFile(url, 'utf8')));
  return parts.join('\n');
}

export function readModule(name) {
  return readFile(new URL(name, SRC), 'utf8');
}

export function readCss() {
  return readFile(new URL('style.css', SRC), 'utf8');
}

export function catalogPath(locale) {
  return new URL(`../../../crates/cad-ui-core/i18n/${locale}.json`, import.meta.url);
}

export async function readCatalog(locale) {
  return JSON.parse(await readFile(catalogPath(locale), 'utf8'));
}
