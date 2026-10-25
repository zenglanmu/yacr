// The host-resolved locale: the single source of the system default.
//
// The Rust host resolves the locale once (YACR_LOCALE, else the POSIX
// LC_ALL/LC_MESSAGES/LANGUAGE/LANG precedence) and injects the canonical tag
// (`zh-CN` or `en`) as `window.__YACR_HOST_LOCALE__` before the document is
// parsed. The chrome must use that tag, not `navigator`, so a popup's chrome and
// the native dialog labels can never disagree. Precedence: a valid stored choice,
// then the injected tag, then navigator, then English.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  HOST_LOCALE_GLOBAL,
  injectedHostLocale,
  initialLocale,
  LOCALE_STORAGE_KEY,
  resolveLocale,
} from '../src/i18n.ts';
import { readModule } from './helpers.ts';

/** A document stand-in with a (shared) storage object. */
function fakeDocument(storage) {
  return {
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

/** Run `body` with `window[HOST_LOCALE_GLOBAL]` set to `tag`. */
function withInjectedHostLocale(tag, body) {
  const previous = globalThis.window;
  globalThis.window = { [HOST_LOCALE_GLOBAL]: tag };
  try {
    return body();
  } finally {
    if (previous === undefined) delete globalThis.window;
    else globalThis.window = previous;
  }
}

test('the injected host locale wins over navigator.languages', () => {
  assert.deepEqual(
    resolveLocale(fakeDocument(memoryStorage()), fakeNavigator('en-US'), 'zh-CN'),
    { locale: 'zh-CN', source: 'system' },
  );
  assert.deepEqual(
    resolveLocale(fakeDocument(memoryStorage()), fakeNavigator('zh-CN'), 'en'),
    { locale: 'en', source: 'system' },
  );
  // `YACR_LOCALE=zh-CN` on an English system: Chinese chrome and Chinese native
  // labels, instead of English chrome next to Chinese dialogs.
  assert.equal(
    initialLocale(fakeDocument(memoryStorage()), fakeNavigator('en-US'), 'zh-CN'),
    'zh-CN',
  );
});

test('an unmappable injected tag is English, never a fall-through to navigator', () => {
  for (const tag of ['fr', 'de-DE', 'zh-TW', 'zh-Hant']) {
    assert.deepEqual(
      resolveLocale(fakeDocument(memoryStorage()), fakeNavigator('zh-CN'), tag),
      { locale: 'en', source: 'system' },
      tag,
    );
  }
  // The value is the host's already-resolved tag, so a normalisable spelling is
  // still mapped through the shared table.
  assert.deepEqual(
    resolveLocale(fakeDocument(memoryStorage()), fakeNavigator('en-US'), 'zh_CN.UTF-8'),
    { locale: 'zh-CN', source: 'system' },
  );
});

test('a stored supported locale still beats the injected host locale', () => {
  const storage = memoryStorage({ [LOCALE_STORAGE_KEY]: 'en' });
  assert.deepEqual(
    resolveLocale(fakeDocument(storage), fakeNavigator('en-US'), 'zh-CN'),
    { locale: 'en', source: 'stored' },
  );
  // An invalid stored value is treated as absent, so the injected tag applies.
  for (const garbage of ['de', 'EN', 'en-US', '']) {
    assert.deepEqual(
      resolveLocale(fakeDocument(memoryStorage({ [LOCALE_STORAGE_KEY]: garbage })), fakeNavigator('en-US'), 'zh-CN'),
      { locale: 'zh-CN', source: 'system' },
      garbage,
    );
  }
});

test('an absent injected tag leaves the navigator path unchanged', () => {
  assert.deepEqual(
    resolveLocale(fakeDocument(memoryStorage()), fakeNavigator('zh-CN'), undefined),
    { locale: 'zh-CN', source: 'system' },
  );
  assert.deepEqual(
    resolveLocale(fakeDocument(memoryStorage()), fakeNavigator('fr-FR', 'de-DE'), undefined),
    { locale: 'en', source: 'fallback' },
  );
  assert.deepEqual(
    resolveLocale(fakeDocument(memoryStorage()), null, undefined),
    { locale: 'en', source: 'fallback' },
  );
  // No injected global at all (a plain browser / node): nothing to read.
  assert.equal(injectedHostLocale(), undefined);
});

test('the injected tag is read from the global the host sets before load', () => {
  withInjectedHostLocale('zh-CN', () => {
    assert.equal(injectedHostLocale(), 'zh-CN');
    // The default argument is the injected global, so the call sites
    // (`useState(initialLocale)` in both documents) need no extra wiring.
    assert.deepEqual(
      resolveLocale(fakeDocument(memoryStorage()), fakeNavigator('en-US')),
      { locale: 'zh-CN', source: 'system' },
    );
  });
  // A non-string global is not an opinion; navigator applies again.
  withInjectedHostLocale(undefined, () => {
    assert.equal(injectedHostLocale(), undefined);
    assert.deepEqual(
      resolveLocale(fakeDocument(memoryStorage()), fakeNavigator('en-US')),
      { locale: 'en', source: 'system' },
    );
  });
});

test('the single document boots from the host-resolved locale', async () => {
  // There is one document now: the shell resolves the host locale once and the
  // overlay shares its i18n context, so there is no second resolution path.
  const app = await readModule('App.tsx');
  assert.ok(app.includes('useState(initialLocale)'), 'the shell must boot from the resolver');
  assert.ok(app.includes('document.documentElement.lang = localeTag(locale)'));
  const overlay = await readModule('components/PopupOverlay.tsx');
  assert.ok(overlay.includes('useI18n()'), 'the overlay uses the shell i18n context');
  assert.equal(overlay.includes('initialLocale'), false, 'the overlay must not re-resolve the locale');
  assert.equal(overlay.includes('createTranslator'), false, 'the overlay must not build a translator');
});
