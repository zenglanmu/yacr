/* yacr landing — progressive enhancement only.
   No frameworks, no external requests, no inline styles. */
(() => {
  'use strict';

  const LANG_KEY = 'yacr-landing-lang';
  const DEFAULT_LANG = 'zh-CN';

  const STRINGS = {
    'zh-CN': {
      title: 'yacr — Yet Another CAD in Rust · 用 Rust 写的开源 CAD',
      description:
        'yacr 是一个用 Rust 写核心、React + Tauri 写界面的开源 CAD 查看与测量系统。一个核心，多种载体。早期预览：尚无可交付产品，Linux 是当前唯一可运行的 GUI。',
      ogTitle: 'yacr — Yet Another CAD in Rust',
      ogDescription:
        'Rust 核心负责文档、几何、命令与 wgpu 渲染；React + Tauri 负责界面。一个核心，多种载体。早期预览，尚无可交付产品。',
      twitterDescription:
        '用 Rust 写核心、React + Tauri 写界面的开源 CAD 查看与测量系统。早期预览；Linux 是当前唯一可运行的 GUI。',
      navOpen: '打开导航菜单',
      navClose: '关闭导航菜单',
      copied: '命令已复制',
      copyFailed: '复制失败，请手动选择命令'
    },
    en: {
      title: 'yacr — Yet Another CAD in Rust · Open-source CAD',
      description:
        'yacr is an open-source CAD viewer and measurement system with a Rust core, React + Tauri interface, and one core across five surfaces. Early preview: no shippable product yet; Linux is the only runnable GUI.',
      ogTitle: 'yacr — Yet Another CAD in Rust',
      ogDescription:
        'A Rust core for documents, geometry, commands, and wgpu rendering. A React + Tauri interface. One core, many surfaces. Early preview; no shippable product yet.',
      twitterDescription:
        'Open-source CAD with a Rust core and a React + Tauri interface. Early preview; Linux is the only runnable GUI.',
      navOpen: 'Open navigation menu',
      navClose: 'Close navigation menu',
      copied: 'Command copied',
      copyFailed: 'Copy failed — select the command manually'
    }
  };

  const doc = document;
  const root = doc.documentElement;
  const liveRegion = doc.getElementById('live-region');
  const navToggle = doc.querySelector('.nav-toggle');
  const siteNav = doc.getElementById('site-nav');
  const header = doc.querySelector('.site-header');
  const prefersReducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');

  const currentLang = () => (root.dataset.lang === 'en' ? 'en' : 'zh-CN');
  const strings = () => STRINGS[currentLang()];

  /* ---------------------------------------------------------------- language */

  function readStoredLanguage() {
    try {
      const stored = window.localStorage.getItem(LANG_KEY);
      return stored === 'en' || stored === 'zh-CN' ? stored : null;
    } catch (_error) {
      return null;
    }
  }

  function storeLanguage(lang) {
    try {
      window.localStorage.setItem(LANG_KEY, lang);
    } catch (_error) {
      /* localStorage can be unavailable under file:// or private modes. */
    }
  }

  function setMeta(attribute, name, value) {
    const element = doc.head.querySelector(`meta[${attribute}="${name}"]`);
    if (element) element.setAttribute('content', value);
  }

  function setAriaLabels(lang) {
    doc.querySelectorAll('[data-aria-zh]').forEach((element) => {
      if (element === navToggle) return;
      const value = lang === 'en' ? element.dataset.ariaEn : element.dataset.ariaZh;
      if (value) element.setAttribute('aria-label', value);
    });
  }

  function setNavToggleLabel() {
    if (!navToggle) return;
    const open = navToggle.getAttribute('aria-expanded') === 'true';
    navToggle.setAttribute('aria-label', open ? strings().navClose : strings().navOpen);
  }

  function applyLanguage(lang, persist) {
    const normalized = lang === 'en' ? 'en' : 'zh-CN';
    const copy = STRINGS[normalized];

    root.lang = normalized === 'en' ? 'en' : 'zh-CN';
    root.dataset.lang = normalized;

    doc.title = copy.title;
    setMeta('name', 'description', copy.description);
    setMeta('property', 'og:title', copy.ogTitle);
    setMeta('property', 'og:description', copy.ogDescription);
    setMeta('property', 'og:locale', normalized === 'en' ? 'en_US' : 'zh_CN');
    setMeta('name', 'twitter:title', copy.ogTitle);
    setMeta('name', 'twitter:description', copy.twitterDescription);

    doc.querySelectorAll('[data-lang-set]').forEach((button) => {
      button.setAttribute('aria-pressed', String(button.dataset.langSet === normalized));
    });

    setAriaLabels(normalized);
    setNavToggleLabel();

    if (persist) storeLanguage(normalized);
  }

  doc.querySelectorAll('[data-lang-set]').forEach((button) => {
    button.addEventListener('click', () => {
      applyLanguage(button.dataset.langSet, true);
    });
  });

  applyLanguage(readStoredLanguage() || DEFAULT_LANG, false);

  /* -------------------------------------------------------------------- nav */

  function setNav(open, restoreFocus) {
    if (!navToggle || !siteNav) return;
    navToggle.setAttribute('aria-expanded', String(open));
    siteNav.dataset.open = open ? 'true' : 'false';
    doc.body.classList.toggle('nav-open', open);
    setNavToggleLabel();
    if (!open && restoreFocus) navToggle.focus();
  }

  if (navToggle && siteNav) {
    setNav(false, false);

    navToggle.addEventListener('click', () => {
      const open = navToggle.getAttribute('aria-expanded') !== 'true';
      setNav(open, false);
    });

    siteNav.addEventListener('click', (event) => {
      if (event.target.closest('a')) setNav(false, false);
    });

    doc.addEventListener('keydown', (event) => {
      if (event.key === 'Escape' && navToggle.getAttribute('aria-expanded') === 'true') {
        setNav(false, true);
      }
    });

    doc.addEventListener('click', (event) => {
      if (navToggle.getAttribute('aria-expanded') !== 'true') return;
      if (siteNav.contains(event.target) || navToggle.contains(event.target)) return;
      setNav(false, false);
    });

    window.addEventListener('resize', () => {
      if (window.innerWidth > 900) setNav(false, false);
    });
  }

  /* ----------------------------------------------------------------- header */

  let lastScrolled = false;
  function onScroll() {
    if (!header) return;
    const scrolled = window.scrollY > 8;
    if (scrolled !== lastScrolled) {
      lastScrolled = scrolled;
      header.classList.toggle('is-scrolled', scrolled);
    }
  }
  window.addEventListener('scroll', onScroll, { passive: true });
  onScroll();

  /* --------------------------------------------------------------- scrollspy */

  const navLinks = Array.from(doc.querySelectorAll('[data-nav]'));
  const spySections = navLinks
    .map((link) => doc.getElementById(link.dataset.nav))
    .filter(Boolean);

  if ('IntersectionObserver' in window && spySections.length) {
    const spy = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (!entry.isIntersecting) return;
          const id = entry.target.id;
          navLinks.forEach((link) => {
            if (link.dataset.nav === id) link.setAttribute('aria-current', 'true');
            else link.removeAttribute('aria-current');
          });
        });
      },
      { rootMargin: '-38% 0px -54% 0px', threshold: 0 }
    );
    spySections.forEach((section) => spy.observe(section));
  }

  /* ----------------------------------------------------------------- reveal */

  const revealItems = doc.querySelectorAll('.reveal');
  if (revealItems.length && 'IntersectionObserver' in window && !prefersReducedMotion.matches) {
    root.classList.add('has-reveal');
    const revealObserver = new IntersectionObserver(
      (entries, observer) => {
        entries.forEach((entry) => {
          if (!entry.isIntersecting) return;
          entry.target.classList.add('is-visible');
          observer.unobserve(entry.target);
        });
      },
      { threshold: 0.12, rootMargin: '0px 0px -6% 0px' }
    );
    revealItems.forEach((item) => revealObserver.observe(item));
  }

  /* -------------------------------------------------------------- downloads */

  function normalizeBase(value) {
    return typeof value === 'string' ? value.trim().replace(/\/+$/, '') : '';
  }

  function assetUrl(base, file) {
    const safeFile = String(file)
      .split('/')
      .map((part) => encodeURIComponent(part))
      .join('/');
    return `${base}/${safeFile}`;
  }

  async function hydrateDownloads() {
    let data;
    try {
      const response = await fetch('./downloads.json', { cache: 'no-store' });
      if (!response.ok) return;
      data = await response.json();
    } catch (_error) {
      /* file://, offline, or 404: keep the GitHub URLs written in the HTML. */
      return;
    }

    if (!data || typeof data !== 'object' || !data.assets) return;

    const cdnBase = normalizeBase(data.cdnBase);
    const githubBase = normalizeBase(data.githubBase);
    const assets = data.assets;

    doc.querySelectorAll('[data-download]').forEach((link) => {
      const asset = assets[link.dataset.download];
      if (!asset || typeof asset.file !== 'string') return;
      const base = cdnBase || githubBase;
      if (!base) return;
      link.href = assetUrl(base, asset.file);

      const badge = link.querySelector('.cdn-badge');
      if (badge) badge.hidden = !cdnBase;
    });

    if (!githubBase) return;

    doc.querySelectorAll('[data-github]').forEach((link) => {
      const asset = assets[link.dataset.github];
      if (asset && typeof asset.file === 'string') {
        link.href = assetUrl(githubBase, asset.file);
      }
    });

    doc.querySelectorAll('[data-sha]').forEach((link) => {
      const asset = assets[link.dataset.sha];
      if (asset && typeof asset.file === 'string') {
        link.href = assetUrl(githubBase, `${asset.file}.sha256`);
      }
    });
  }

  hydrateDownloads();

  /* -------------------------------------------------------------- copy text */

  function announce(message) {
    if (!liveRegion) return;
    liveRegion.textContent = '';
    window.setTimeout(() => {
      liveRegion.textContent = message;
    }, 20);
  }

  async function copyText(text) {
    if (navigator.clipboard && window.isSecureContext) {
      try {
        await navigator.clipboard.writeText(text);
        return true;
      } catch (_error) {
        /* Fall through to the selection-based fallback. */
      }
    }

    const helper = doc.createElement('textarea');
    helper.className = 'copy-helper';
    helper.value = text;
    helper.setAttribute('readonly', '');
    helper.setAttribute('aria-hidden', 'true');
    doc.body.appendChild(helper);
    helper.select();
    helper.setSelectionRange(0, helper.value.length);

    let copied = false;
    try {
      copied = doc.execCommand('copy');
    } catch (_error) {
      copied = false;
    }
    helper.remove();
    return copied;
  }

  doc.querySelectorAll('[data-copy]').forEach((button) => {
    button.addEventListener('click', async () => {
      const copied = await copyText(button.dataset.copy || '');
      if (copied) {
        button.classList.add('is-copied');
        announce(strings().copied);
        window.clearTimeout(button._copyTimer);
        button._copyTimer = window.setTimeout(() => {
          button.classList.remove('is-copied');
        }, 1900);
      } else {
        announce(strings().copyFailed);
      }
    });
  });
})();
