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
        'yacr 是一个用 Rust 写核心、React + Tauri 写界面的开源 CAD 查看与测量系统。一个核心，多种载体。预览版可直接下载：Linux、Windows、macOS、Android 与 Web 均有构建；Linux/Wayland 是主开发目标。它不是生产级 CAD。',
      ogTitle: 'yacr — Yet Another CAD in Rust',
      ogDescription:
        'Rust 核心负责文档、几何、命令与 wgpu 渲染；React + Tauri 负责界面。一个核心，多种载体。预览版已可直接下载，但 DWG 兼容性、真实图纸与性能验收尚未完成。',
      twitterDescription:
        '用 Rust 写核心、React + Tauri 写界面的开源 CAD 查看与测量系统。预览版可直接下载：Linux、Windows、macOS、Android 与 Web 均有构建；Linux/Wayland 是主开发目标。',
      navOpen: '打开导航菜单',
      navClose: '关闭导航菜单',
      copied: '命令已复制',
      copyFailed: '复制失败，请手动选择命令'
    },
    en: {
      title: 'yacr — Yet Another CAD in Rust · Open-source CAD',
      description:
        'yacr is an open-source CAD viewer and measurement system with a Rust core, React + Tauri interface, and one core across five surfaces. Preview builds are downloadable for Linux, Windows, macOS, Android, and the Web; Linux/Wayland is the primary development target. It is not a production-grade CAD tool.',
      ogTitle: 'yacr — Yet Another CAD in Rust',
      ogDescription:
        'A Rust core for documents, geometry, commands, and wgpu rendering. A React + Tauri interface. One core, many surfaces. Preview builds are downloadable; DWG compatibility, real drawings, and performance acceptance are not finished.',
      twitterDescription:
        'Open-source CAD with a Rust core and a React + Tauri interface. Preview builds are downloadable for Linux, Windows, macOS, Android, and the Web; Linux/Wayland is the primary development target.',
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

  function setAltText(lang) {
    doc.querySelectorAll('[data-alt-zh]').forEach((element) => {
      const value = lang === 'en' ? element.dataset.altEn : element.dataset.altZh;
      if (value) element.setAttribute('alt', value);
    });
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
    setAltText(normalized);
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

  /* ------------------------------------------------------- release metadata */

  function normalizeBase(value) {
    if (typeof value !== 'string') return '';
    const trimmed = value.trim().replace(/\/+$/, '');
    return /^https:\/\//i.test(trimmed) ? trimmed : '';
  }

  function assetUrl(base, file) {
    const safeFile = String(file)
      .split('/')
      .map((part) => encodeURIComponent(part))
      .join('/');
    return `${base}/${safeFile}`;
  }

  function httpsUrl(value) {
    if (typeof value !== 'string' || !value.trim()) return '';
    try {
      const url = new URL(value.trim());
      return url.protocol === 'https:' ? url.href : '';
    } catch (_error) {
      return '';
    }
  }

  function releaseTagUrl(release) {
    if (typeof release !== 'string' || !release.trim()) return '';
    return `https://github.com/zenglanmu/yacr/releases/tag/${encodeURIComponent(release.trim())}`;
  }

  function formatMiB(bytes) {
    return (bytes / 1048576).toFixed(1);
  }

  function setReleaseVersion(value) {
    if (typeof value !== 'string' || !value.trim()) return;
    const version = value.trim();
    doc.querySelectorAll('[data-release-version]').forEach((node) => {
      node.textContent = version;
    });
  }

  function setReleaseDate(value) {
    if (typeof value !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(value.trim())) return;
    const date = value.trim();
    doc.querySelectorAll('[data-release-date]').forEach((node) => {
      node.textContent = date;
      node.setAttribute('datetime', date);
    });
  }

  function setReleaseUrl(value) {
    const url = httpsUrl(value);
    if (!url) return;
    doc.querySelectorAll('[data-release-url]').forEach((node) => {
      node.href = url;
    });
  }

  function setReleaseCount(count) {
    if (!Number.isFinite(count) || count <= 0) return;
    doc.querySelectorAll('[data-release-assets]').forEach((node) => {
      node.textContent = String(count);
    });
  }

  function setReleaseTotal(bytes) {
    if (!Number.isFinite(bytes) || bytes <= 0) return;
    doc.querySelectorAll('[data-release-total]').forEach((node) => {
      node.textContent = `≈ ${formatMiB(bytes)} MiB`;
    });
  }

  function updateCommandTemplates(assets) {
    doc.querySelectorAll('[data-cmd-template]').forEach((code) => {
      const asset = assets[code.dataset.cmdFile];
      if (!asset || typeof asset.file !== 'string' || !asset.file) return;
      code.textContent = code.dataset.cmdTemplate.replace('{file}', asset.file);
      const command = code.closest('.cmd');
      const button = command ? command.querySelector('.copy-btn') : null;
      if (button) button.dataset.copy = code.textContent;
    });
  }

  /* -------------------------------------------------------------- downloads */

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

    if (!data || typeof data !== 'object' || !data.assets || typeof data.assets !== 'object') {
      return;
    }

    const cdnBase = normalizeBase(data.cdnBase);
    const githubBase = normalizeBase(data.githubBase);
    const assets = data.assets;
    const release = typeof data.release === 'string' ? data.release.trim() : '';

    if (release) {
      setReleaseVersion(release);
      setReleaseUrl(releaseTagUrl(release));
    }

    const assetKeys = Object.keys(assets);
    setReleaseCount(assetKeys.length);

    const totalBytes = assetKeys.reduce((sum, key) => {
      const asset = assets[key];
      const size = asset ? Number(asset.size) : NaN;
      return Number.isFinite(size) && size > 0 ? sum + size : sum;
    }, 0);
    setReleaseTotal(totalBytes);

    doc.querySelectorAll('[data-file-for]').forEach((node) => {
      const asset = assets[node.dataset.fileFor];
      if (asset && typeof asset.file === 'string' && asset.file) {
        node.textContent = asset.file;
      }
    });

    doc.querySelectorAll('[data-size-for]').forEach((node) => {
      const asset = assets[node.dataset.sizeFor];
      const size = asset ? Number(asset.size) : NaN;
      if (!Number.isFinite(size) || size <= 0) return;
      const strong = doc.createElement('strong');
      strong.textContent = size.toLocaleString('en-US');
      const label = doc.createElement('span');
      label.textContent = `bytes · ≈ ${formatMiB(size)} MiB`;
      node.replaceChildren(strong, doc.createTextNode(' '), label);
    });

    doc.querySelectorAll('[data-sha-for]').forEach((node) => {
      const asset = assets[node.dataset.shaFor];
      const digest = asset && typeof asset.sha256 === 'string' ? asset.sha256.trim() : '';
      if (!/^[0-9a-f]{64}$/i.test(digest)) return;
      node.textContent = digest;
      const block = node.closest('.dl-hash');
      const button = block ? block.querySelector('.copy-btn') : null;
      if (button) button.dataset.copy = digest;
    });

    if (githubBase) {
      doc.querySelectorAll('[data-github]').forEach((link) => {
        const asset = assets[link.dataset.github];
        if (!asset || typeof asset.file !== 'string' || !asset.file) return;
        link.href = assetUrl(githubBase, `${asset.file}${link.dataset.githubSuffix || ''}`);
      });

      doc.querySelectorAll('[data-github-file]').forEach((link) => {
        if (link.dataset.githubFile) link.href = assetUrl(githubBase, link.dataset.githubFile);
      });
    }

    const primaryBase = cdnBase || githubBase;
    if (primaryBase) {
      doc.querySelectorAll('[data-download]').forEach((link) => {
        const asset = assets[link.dataset.download];
        if (!asset || typeof asset.file !== 'string' || !asset.file) return;
        link.href = assetUrl(primaryBase, asset.file);

        const badge = link.querySelector('.cdn-badge');
        if (badge) badge.hidden = !cdnBase;
      });
    }

    if (githubBase) {
      doc.querySelectorAll('[data-sha]').forEach((link) => {
        const asset = assets[link.dataset.sha];
        if (!asset || typeof asset.file !== 'string' || !asset.file) return;
        link.href = assetUrl(githubBase, `${asset.file}.sha256`);
      });
    }

    updateCommandTemplates(assets);
  }

  hydrateDownloads();

  /* -------------------------------------------------------------- changelog */

  function bilingual(tag, zh, en) {
    const node = doc.createElement(tag);
    const zhNode = doc.createElement('span');
    zhNode.className = 'lang--zh';
    zhNode.textContent = zh;
    const enNode = doc.createElement('span');
    enNode.className = 'lang--en';
    enNode.lang = 'en';
    enNode.textContent = en;
    node.append(zhNode, enNode);
    return node;
  }

  function showChangelogFallback() {
    const mount = doc.querySelector('[data-changelog]');
    const loading = doc.querySelector('[data-changelog-loading]');
    const list = doc.querySelector('[data-changelog-list]');
    const fallback = doc.querySelector('[data-changelog-fallback]');
    if (loading) loading.hidden = true;
    if (list) {
      list.hidden = true;
      list.replaceChildren();
    }
    if (fallback) fallback.hidden = false;
    if (mount) mount.setAttribute('aria-busy', 'false');
  }

  function buildRelease(release, index) {
    const latest = index === 0;
    const version = release.version.trim();
    const panelId = `cl-panel-${index}`;

    const item = doc.createElement('li');
    item.className = latest ? 'cl-release cl-release--latest is-open' : 'cl-release';

    const head = doc.createElement('div');
    head.className = 'cl-release-head';

    const main = doc.createElement('div');
    main.className = 'cl-head-main';

    const heading = doc.createElement('h3');
    heading.className = 'cl-version';
    heading.textContent = version;

    if (latest) {
      const badge = bilingual('span', '最新', 'LATEST');
      badge.className = 'cl-latest';
      heading.append(' ', badge);
    }

    const meta = doc.createElement('p');
    meta.className = 'cl-meta';

    if (typeof release.date === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(release.date.trim())) {
      const date = release.date.trim();
      const time = doc.createElement('time');
      time.dateTime = date;
      time.textContent = date;
      meta.append(time);
    }

    const title = typeof release.title === 'string' ? release.title.trim() : '';
    const genericTitle = `yacr ${version}`;
    if (title && title.toLowerCase() !== genericTitle.toLowerCase()) {
      const titleNode = doc.createElement('span');
      titleNode.className = 'cl-title';
      titleNode.textContent = title;
      if (meta.childNodes.length) meta.append(doc.createTextNode(' · '));
      meta.append(titleNode);
    }

    const groups = release.sections.filter(
      (section) => section && typeof section === 'object'
    );
    if (meta.childNodes.length) meta.append(doc.createTextNode(' · '));
    meta.append(bilingual('span', `${groups.length} 个分组`, `${groups.length} sections`));

    main.append(heading, meta);

    const actions = doc.createElement('div');
    actions.className = 'cl-head-actions';

    const url = httpsUrl(release.url);
    if (url) {
      const link = doc.createElement('a');
      link.className = 'cl-full-link';
      link.href = url;
      link.target = '_blank';
      link.rel = 'noopener';
      link.append(bilingual('span', '查看完整发布说明', 'Full release notes'));
      const ext = doc.createElement('span');
      ext.className = 'ext';
      ext.setAttribute('aria-hidden', 'true');
      ext.textContent = '↗';
      link.append(ext);
      actions.append(link);
    }

    const toggle = doc.createElement('button');
    toggle.className = 'cl-toggle';
    toggle.type = 'button';
    toggle.setAttribute('aria-expanded', String(latest));
    toggle.setAttribute('aria-controls', panelId);

    const togglePrefix = doc.createElement('span');
    togglePrefix.className = 'sr-only';
    togglePrefix.textContent = `${version} `;
    const closedLabel = bilingual('span', '展开', 'Expand');
    closedLabel.className = 'cl-when-closed';
    const openLabel = bilingual('span', '收起', 'Collapse');
    openLabel.className = 'cl-when-open';
    const chevron = doc.createElement('span');
    chevron.className = 'cl-chevron';
    chevron.setAttribute('aria-hidden', 'true');
    toggle.append(togglePrefix, closedLabel, openLabel, chevron);
    actions.append(toggle);

    const panel = doc.createElement('div');
    panel.className = 'cl-panel';
    panel.id = panelId;
    panel.hidden = !latest;

    let renderedGroups = 0;
    groups.forEach((group) => {
      const headingText = typeof group.heading === 'string' ? group.heading.trim() : '';
      const entries = Array.isArray(group.items)
        ? group.items.filter((entry) => typeof entry === 'string' && entry.trim())
        : [];
      if (!headingText && !entries.length) return;

      const block = doc.createElement('section');
      block.className = 'cl-group';
      if (headingText) {
        const groupHeading = doc.createElement('h4');
        groupHeading.textContent = headingText;
        block.append(groupHeading);
      }
      if (entries.length) {
        const list = doc.createElement('ul');
        entries.forEach((entry) => {
          const entryNode = doc.createElement('li');
          entryNode.textContent = entry.trim();
          list.append(entryNode);
        });
        block.append(list);
      }
      panel.append(block);
      renderedGroups += 1;
    });

    if (!renderedGroups) {
      const empty = doc.createElement('p');
      empty.className = 'cl-empty';
      empty.append(
        bilingual(
          'span',
          '这个 tag 没有分组条目；完整发布说明在 GitHub 上。',
          'No grouped entries for this tag; the full release notes are on GitHub.'
        )
      );
      panel.append(empty);
    }

    toggle.addEventListener('click', () => {
      const isOpen = toggle.getAttribute('aria-expanded') === 'true';
      toggle.setAttribute('aria-expanded', String(!isOpen));
      panel.hidden = isOpen;
      item.classList.toggle('is-open', !isOpen);
    });

    head.append(main, actions);
    item.append(head, panel);
    return item;
  }

  function renderChangelog(releases) {
    const mount = doc.querySelector('[data-changelog]');
    const loading = doc.querySelector('[data-changelog-loading]');
    const list = doc.querySelector('[data-changelog-list]');
    const fallback = doc.querySelector('[data-changelog-fallback]');
    if (!mount || !list) return;

    const valid = releases
      .filter(
        (release) =>
          release &&
          typeof release === 'object' &&
          typeof release.version === 'string' &&
          release.version.trim() &&
          Array.isArray(release.sections)
      )
      .slice(0, 30);

    if (!valid.length) {
      showChangelogFallback();
      return;
    }

    valid.forEach((release, index) => {
      list.append(buildRelease(release, index));
    });

    if (loading) loading.hidden = true;
    if (fallback) fallback.hidden = true;
    list.hidden = false;
    mount.setAttribute('aria-busy', 'false');

    const latest = valid[0];
    setReleaseVersion(latest.version);
    setReleaseDate(latest.date);
    setReleaseUrl(latest.url);
  }

  async function hydrateChangelog() {
    let data;
    try {
      const response = await fetch('./changelog.json', { cache: 'no-store' });
      if (!response.ok) {
        showChangelogFallback();
        return;
      }
      data = await response.json();
    } catch (_error) {
      showChangelogFallback();
      return;
    }

    renderChangelog(data && Array.isArray(data.releases) ? data.releases : []);
  }

  hydrateChangelog();

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
