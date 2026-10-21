// Locale resolution and persistence.
//
// Order: persisted explicit choice, then the OS/WebView UI language, then
// English. `document.documentElement.lang` is deliberately not a source: it is
// static, so reading it would lock the default. The app writes it from the
// resolved locale instead. Storage is only ever a hint: an absent, blocked,
// throwing or unrecognised value falls through to the next source — never an
// exception, never an unsupported locale.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  DEFAULT_LOCALE,
  initialLocale,
  LOCALES,
  LOCALE_STORAGE_KEY,
  resolveLocale,
  storeLocale,
  storedLocale,
  systemLocaleFor,
} from '../src/i18n.mjs';
import { readModule } from './helpers.mjs';

/** A document stand-in with a (shared) storage object. */
function fakeDocument(storage) {
  return {
    // Static markup value: a system-source regression would pick this up.
    documentElement: { lang: 'zh-CN' },
    defaultView: storage === undefined ? undefined : { localStorage: storage },
  };
}

/** A navigator stand-in: `languages` in preference order, `language` the first. */
function fakeNavigator(...languages) {
  return { languages, language: languages[0] };
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
  assert.equal(DEFAULT_LOCALE, 'en');
});

test('a stored supported locale wins over the system locale', () => {
  const en = memoryStorage({ [LOCALE_STORAGE_KEY]: 'en' });
  assert.deepEqual(
    resolveLocale(fakeDocument(en), fakeNavigator('zh-CN')),
    { locale: 'en', source: 'stored' },
  );
  const zh = memoryStorage({ [LOCALE_STORAGE_KEY]: 'zh-CN' });
  assert.deepEqual(
    resolveLocale(fakeDocument(zh), fakeNavigator('en-US')),
    { locale: 'zh-CN', source: 'stored' },
  );
  assert.equal(initialLocale(fakeDocument(en), fakeNavigator('zh-CN')), 'en');
});

test('a stale or hand-edited stored value is ignored, not applied', () => {
  // `en-US` is not a catalog locale: the key must be an exact LOCALES member.
  for (const garbage of ['de', 'EN', 'en-US', '', '{"locale":"en"}', 'null']) {
    const storage = memoryStorage({ [LOCALE_STORAGE_KEY]: garbage });
    assert.deepEqual(
      resolveLocale(fakeDocument(storage), fakeNavigator('zh-CN')),
      { locale: 'zh-CN', source: 'system' },
      garbage,
    );
    assert.equal(resolveLocale(fakeDocument(storage), fakeNavigator('fr')).locale, 'en', garbage);
    assert.equal(storedLocale(fakeDocument(storage)), null, garbage);
  }
});

test('the system locale table maps exactly as the Rust host does', () => {
  const cases = [
    // Simplified Chinese (any region) selects zh-CN.
    ['zh', 'zh-CN', 'system'],
    ['zh-CN', 'zh-CN', 'system'],
    ['zh_CN', 'zh-CN', 'system'],
    ['zh-Hans', 'zh-CN', 'system'],
    ['zh-Hans-CN', 'zh-CN', 'system'],
    ['zh-SG', 'zh-CN', 'system'],
    // English (any region) selects en.
    ['en', 'en', 'system'],
    ['en-US', 'en', 'system'],
    ['en_GB', 'en', 'system'],
    // No Hant catalog exists, so Traditional Chinese is not served Simplified.
    ['zh-TW', 'en', 'fallback'],
    ['zh-TW.UTF-8', 'en', 'fallback'],
    ['zh-HK', 'en', 'fallback'],
    ['zh-Hant', 'en', 'fallback'],
    ['zh-Hant-TW', 'en', 'fallback'],
    // Everything else falls back to English.
    ['pt-BR', 'en', 'fallback'],
    ['fr', 'en', 'fallback'],
    ['de-DE', 'en', 'fallback'],
    ['', 'en', 'fallback'],
    ['--', 'en', 'fallback'],
  ];
  for (const [tag, locale, source] of cases) {
    assert.deepEqual(
      resolveLocale(fakeDocument(memoryStorage()), fakeNavigator(tag)),
      { locale, source },
      tag,
    );
  }
});

test('navigator.languages order is respected and the first supported tag wins', () => {
  const empty = memoryStorage();
  assert.deepEqual(
    resolveLocale(fakeDocument(empty), fakeNavigator('fr-FR', 'zh-CN', 'en-US')),
    { locale: 'zh-CN', source: 'system' },
  );
  assert.deepEqual(
    resolveLocale(fakeDocument(empty), fakeNavigator('de-DE', 'en-GB')),
    { locale: 'en', source: 'system' },
  );
  // Nothing in the list is shipped: English is reported as a fallback, not as a
  // "system" match.
  assert.deepEqual(
    resolveLocale(fakeDocument(empty), fakeNavigator('fr-FR', 'de-DE')),
    { locale: 'en', source: 'fallback' },
  );
  // `navigator.language` is used when `languages` is absent or empty.
  assert.equal(resolveLocale(fakeDocument(empty), { language: 'zh-CN' }).locale, 'zh-CN');
  assert.equal(resolveLocale(fakeDocument(empty), { languages: [], language: 'en-GB' }).locale, 'en');
});

test('a missing navigator and the static document.lang both fall back to English', () => {
  // The document's declared lang is static markup and must not decide.
  assert.deepEqual(resolveLocale(fakeDocument(memoryStorage()), null), {
    locale: 'en',
    source: 'fallback',
  });
  assert.deepEqual(resolveLocale(fakeDocument(memoryStorage()), fakeNavigator()), {
    locale: 'en',
    source: 'fallback',
  });
  assert.equal(systemLocaleFor('zh-TW'), null);
});

test('a blocked or throwing storage is contained and never throws', () => {
  const throwingGetter = {
    get localStorage() { throw new Error('SecurityError'); },
  };
  assert.equal(
    initialLocale({ documentElement: { lang: 'zh-CN' }, defaultView: throwingGetter }, fakeNavigator('zh-CN')),
    'zh-CN',
  );
  assert.equal(
    storeLocale('en', { documentElement: { lang: 'en' }, defaultView: throwingGetter }),
    false,
  );

  const throwingItem = {
    getItem() { throw new Error('denied'); },
    setItem() { throw new Error('denied'); },
  };
  assert.equal(initialLocale(fakeDocument(throwingItem), fakeNavigator('zh-CN')), 'zh-CN');
  assert.equal(storeLocale('en', fakeDocument(throwingItem)), false);

  // No defaultView at all (SSR / unusual host) behaves the same way.
  assert.equal(initialLocale(fakeDocument(undefined), fakeNavigator('zh-CN')), 'zh-CN');
  assert.equal(storeLocale('en', fakeDocument(undefined)), false);
});

test('a second document reads the locale the first one stored (same origin)', () => {
  // "Restart" / second window: a fresh document with the same localStorage,
  // exactly what a popup window sees. The system locale differs to prove the
  // stored choice wins.
  const storage = memoryStorage();
  assert.equal(storeLocale('en', fakeDocument(storage)), true);
  const popup = fakeDocument(storage);
  assert.equal(initialLocale(popup, fakeNavigator('zh-CN')), 'en');
  assert.equal(storedLocale(popup), 'en');
});

test('storeLocale validates before writing and reports whether it wrote', () => {
  const storage = memoryStorage();
  const doc = fakeDocument(storage);
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
  // The popup document boots from the same resolver, so it cannot diverge.
  const popup = await readModule('components/PopupRoot.jsx');
  assert.ok(popup.includes('initialLocale'));
  assert.ok(popup.includes('document.documentElement.lang = localeTag(locale)'));
});
