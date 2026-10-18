//! Bounded DOM diagnostics: the injected error hook and the mount-state probe.

use super::WEBVIEW;

/// Initialization script: capture uncaught JS errors into `window.__yacr_diag`
/// so the diagnostic eval can report them (bounded diagnostics only).
pub(super) const ERROR_HOOK_JS: &str = r#"
(function () {
  try {
    window.__yacr_diag = window.__yacr_diag || { errors: [] };
    window.addEventListener('error', function (e) {
      try { window.__yacr_diag.errors.push('error: ' + (e.message || e.error || String(e))); } catch (_) {}
    });
    window.addEventListener('unhandledrejection', function (e) {
      try { window.__yacr_diag.errors.push('rejection: ' + String(e.reason)); } catch (_) {}
    });
  } catch (_) {}
})();
"#;

/// Diagnostic eval: report DOM mount state, bounding rects, computed styles and
/// counts. Returns an object; the callback receives it serialized as JSON.
const DOM_DIAG_JS: &str = r#"
(function () {
  try {
    var d = window.__yacr_diag || { errors: [] };
    function info(el) {
      if (!el) return null;
      var r = el.getBoundingClientRect();
      var cs = getComputedStyle(el);
      return {
        rect: [r.x, r.y, r.width, r.height],
        display: cs.display, visibility: cs.visibility, opacity: cs.opacity,
        background: cs.backgroundColor, color: cs.color,
        children: el.children.length, htmlLen: el.innerHTML.length
      };
    }
    return {
      readyState: document.readyState,
      href: location.href,
      title: document.title,
      bodyTextLen: (document.body && document.body.innerText || '').length,
      buttons: document.querySelectorAll('button').length,
      inputs: document.querySelectorAll('input').length,
      selects: document.querySelectorAll('select').length,
      root: info(document.getElementById('root')),
      main: info(document.querySelector('main.shell')),
      ribbon: info(document.querySelector('.ribbon')),
      commandBar: info(document.querySelector('.commandline')),
      errors: d.errors
    };
  } catch (e) {
    return { evalError: String(e) };
  }
})()
"#;

pub(super) fn run_dom_diag(run: u32) {
    WEBVIEW.with(|cell| {
        if let Some(webview) = cell.borrow().as_ref() {
            let result = webview.eval_with_callback(DOM_DIAG_JS, move |json| {
                eprintln!("[probe] DOM diag #{run}: {json}");
            });
            if let Err(error) = result {
                eprintln!("[probe] DOM diag #{run} dispatch failed: {error}");
            }
        } else {
            eprintln!("[probe] DOM diag #{run}: no webview handle");
        }
    });
}
