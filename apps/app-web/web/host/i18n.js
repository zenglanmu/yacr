// Shared-catalog HTML localization and status messages (N01 host sync).

const LOCALE_STORAGE_KEY = "yacr.cad.locale";
const DEFAULT_LOCALE = "zh-CN";

export function createI18n(getWasmModule) {
  let catalog = {};
  let currentLocale = DEFAULT_LOCALE;
  // Last host status as a catalog key + args, so a language switch can re-render
  // it. Raw strings pushed by the Rust host are shown verbatim.
  let lastState = null;

  const element = (id) => document.getElementById(id);

  // --- i18n -------------------------------------------------------------------

  /// Map any accepted tag (`en-US`, `zh-Hans`) onto a shipped stable catalog.
  function normalizeLocale(tag) {
    const primary = String(tag || "")
      .trim()
      .toLowerCase()
      .split(/[-_]/)[0];
    if (primary === "en") return "en";
    if (primary === "zh") return DEFAULT_LOCALE;
    return null;
  }

  function storedLocale() {
    try {
      return normalizeLocale(localStorage.getItem(LOCALE_STORAGE_KEY));
    } catch (error) {
      return null;
    }
  }

  /// Look up a catalog key, substituting `{name}` placeholders. A missing key
  /// renders as `⟦key⟧` (never an empty or invented string), matching the Rust
  /// `Message::Missing` policy so omissions are visible.
  function t(key, args = {}) {
    const template = catalog[key];
    if (typeof template !== "string") return `\u27e6${key}\u27e7`;
    return template.replace(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g, (match, name) =>
      Object.prototype.hasOwnProperty.call(args, name)
        ? String(args[name])
        : match,
    );
  }

  /// Fetch the canonical catalog that `build-web.sh` copied out of
  /// `crates/cad-ui-slint/i18n/`. This is the same JSON the Rust `MessageSource`
  /// embeds, so host and UI strings cannot drift.
  async function loadCatalog(tag) {
    const normalized = normalizeLocale(tag) || DEFAULT_LOCALE;
    if (normalized === currentLocale && Object.keys(catalog).length > 0) {
      return normalized;
    }
    const response = await fetch(`i18n/${normalized}.json`, {
      cache: "no-store",
    });
    if (!response.ok)
      throw new Error(`catalog ${normalized}: HTTP ${response.status}`);
    const parsed = await response.json();
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      throw new Error(`catalog ${normalized}: not a JSON object`);
    }
    catalog = parsed;
    currentLocale = normalized;
    return normalized;
  }

  /// Apply the active language to the HTML chrome (N01 §6): `lang`, `title`,
  /// `noscript` and the language selector.
  function applyDocumentLocale() {
    document.documentElement.lang = currentLocale;
    if (typeof catalog["app.title"] === "string") {
      document.title = catalog["app.title"];
    }
    const noscript = document.querySelector("noscript");
    if (noscript) noscript.textContent = t("host.noscript");
    const label = element("language-label");
    if (label) label.textContent = t("host.language_label");
    const select = element("language");
    if (select) select.value = currentLocale;
  }

  function setStateText(text) {
    lastState = null;
    const node = element("host-state");
    if (node) node.textContent = text;
  }

  function setStateKey(key, args = {}) {
    lastState = { key, args };
    const node = element("host-state");
    if (node) node.textContent = t(key, args);
  }

  function rerenderState() {
    if (!lastState) return;
    const node = element("host-state");
    if (node) node.textContent = t(lastState.key, lastState.args);
  }

  // --- language switch (N01) --------------------------------------------------

  /// Persist the choice, localize the DOM, and re-apply the catalog to the Slint
  /// shell. Only chrome changes: the document, camera, annotations and undo
  /// history are untouched.
  async function setLocale(requested) {
    const normalized = await loadCatalog(requested);
    currentLocale = normalized;
    try {
      localStorage.setItem(LOCALE_STORAGE_KEY, normalized);
    } catch (error) {
      // A read-only storage does not prevent the in-memory switch.
    }
    const wasmModule = getWasmModule();
    if (wasmModule && typeof wasmModule.web_set_locale === "function") {
      const resolved = await wasmModule.web_set_locale(normalized);
      currentLocale = normalizeLocale(resolved) || normalized;
    }
    applyDocumentLocale();
    rerenderState();
    return currentLocale;
  }

  function wireLanguageSelector() {
    const select = element("language");
    if (!select) return;
    select.addEventListener("change", async () => {
      try {
        await setLocale(select.value);
      } catch (error) {
        console.error("yacr: language switch failed", error);
        setStateKey("host.locale_failed", { error: String(error) });
      }
    });
  }

  async function initialize() {
    try {
      currentLocale = await loadCatalog(storedLocale() || DEFAULT_LOCALE);
    } catch (error) {
      console.error("yacr: catalog load failed", error);
    }
    wireLanguageSelector();
    applyDocumentLocale();
  }

  return {
    initialize,
    t,
    setStateKey,
    setStateText,
    setLocale,
    currentLocale: () => currentLocale,
  };
}
