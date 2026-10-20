// Locale persistence.
//
// The language choice must survive a restart and reach a second document (the
// popup window, same origin). Storage is only ever a hint: an absent, blocked,
// throwing or unrecognised value falls through to the document's `lang` and
// then the default — never an exception, never an unsupported locale.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  DEFAULT_LOCALE,
  initialLocale,
  LOCALES,
  LOCALE_STORAGE_KEY,
  storeLocale,
  storedLocale,
} from '../src/i18n.mjs';
import { readModule } from './helpers.mjs';

/** A document stand-in with its own `lang` and a (shared) storage object. */
function fakeDocument(lang, storage) {
  return {
    documentElement: { lang },
    defaultView: storage === undefined ? undefined : { localStorage: storage },
  };
}

function memoryStorage(initial = {}) {
  const entries = new Map(Object.entries(initial));
  return {
    entries,
    getItem: key => (entries.has(key) ? entries.get(key) : null),
    setItem: (key, value) => { entries.set(key, String(value)); },
  };
}

test('the storage key is one stable name shared by both documents', () => {
  assert.equal(LOCALE_STORAGE_KEY, 'yacr.locale');
});

test('a stored supported locale wins over the document default', () => {
  const storage = memoryStorage({ [LOCALE_STORAGE_KEY]: 'en' });
  assert.equal(initialLocale(fakeDocument('zh-CN', storage)), 'en');
  // Without a stored choice the document's declared lang is used, as before.
  assert.equal(initialLocale(fakeDocument('en', memoryStorage())), 'en');
  assert.equal(initialLocale(fakeDocument('zh-CN', memoryStorage())), 'zh-CN');
});

test('an unknown or garbage stored value falls back instead of breaking boot', () => {
  for (const garbage of ['de', 'EN', 'en-US', '', '{"locale":"en"}', 'null']) {
    const storage = memoryStorage({ [LOCALE_STORAGE_KEY]: garbage });
    // Supported document default wins over the corrupt entry.
    assert.equal(initialLocale(fakeDocument('en', storage)), 'en', garbage);
    // An unsupported document default falls through to the build default.
    assert.equal(initialLocale(fakeDocument('de', storage)), DEFAULT_LOCALE, garbage);
    assert.equal(storedLocale(fakeDocument('de', storage)), null, garbage);
  }
});

test('a blocked or throwing storage is contained and never throws', () => {
  const throwingGetter = {
    get localStorage() { throw new Error('SecurityError'); },
  };
  assert.equal(initialLocale({ documentElement: { lang: 'en' }, defaultView: throwingGetter }), 'en');
  assert.equal(storeLocale('en', { documentElement: { lang: 'en' }, defaultView: throwingGetter }), false);

  const throwingItem = {
    getItem() { throw new Error('denied'); },
    setItem() { throw new Error('denied'); },
  };
  assert.equal(initialLocale(fakeDocument('zh-CN', throwingItem)), 'zh-CN');
  assert.equal(storeLocale('en', fakeDocument('zh-CN', throwingItem)), false);

  // No defaultView at all (SSR / unusual host) behaves the same way.
  assert.equal(initialLocale(fakeDocument('en')), 'en');
  assert.equal(storeLocale('en', fakeDocument('en')), false);
});

test('a second document reads the locale the first one stored (same origin)', () => {
  // "Restart" / second window: a fresh document with index.html's lang and the
  // same localStorage, exactly what a popup window sees.
  const storage = memoryStorage();
  const shell = fakeDocument('zh-CN', storage);
  assert.equal(storeLocale('en', shell), true);
  const popup = fakeDocument('zh-CN', storage);
  assert.equal(initialLocale(popup), 'en');
  assert.equal(storedLocale(popup), 'en');
});

test('storeLocale validates before writing and reports whether it wrote', () => {
  const storage = memoryStorage();
  const doc = fakeDocument('zh-CN', storage);
  assert.equal(storeLocale('de', doc), false);
  assert.equal(storage.entries.size, 0);
  for (const locale of LOCALES) {
    assert.equal(storeLocale(locale, doc), true);
    assert.equal(storage.entries.get(LOCALE_STORAGE_KEY), locale);
  }
  assert.equal(storeLocale(null, doc), false);
  assert.equal(storeLocale('', doc), false);
});

test('the shell persists the choice and keeps writing document.lang', async () => {
  const app = await readModule('App.jsx');
  assert.ok(app.includes('storeLocale(locale)'));
  assert.ok(app.includes('document.documentElement.lang = localeTag(locale)'));
  // The popup document boots from the same reader, so it sees the stored value.
  const popup = await readModule('components/PopupRoot.jsx');
  assert.ok(popup.includes('initialLocale'));
  assert.ok(popup.includes('document.documentElement.lang = localeTag(locale)'));
});
