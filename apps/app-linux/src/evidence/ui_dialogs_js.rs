//! JS blobs for [`super::ui_dialogs`] (`YACR_PROBE_UI_DIALOGS`).
//!
//! Split out of the driver for the same reason as
//! [`super::draw_baseline_continue_js`]: the driver stays well under the
//! 1000-line code-file gate (`scripts/check-file-size.py`, no whitelist) and the
//! scripts can be asserted on their own.
//!
//! ## What these scripts are (and are not)
//!
//! Every script runs **inside the real running webview** (`Webview::eval` /
//! `Webview::eval_with_callback`), i.e. against the same React chrome the user
//! sees, and every one of them goes through a **handler the production UI
//! already installs**:
//!
//! * [`CTRL_1_JS`] dispatches a real `KeyboardEvent('keydown')` for `Ctrl+1` on
//!   `window`; the app's own global handler (`apps/desktop-ui/src/App.jsx`,
//!   `if (event.ctrlKey && event.key === '1') … setPropertiesOpen`) toggles the
//!   palette — exactly the path a physical chord takes once the webview has the
//!   key;
//! * [`type_command_js`] sets the command field's value through the
//!   `HTMLInputElement.prototype` value setter (so React's value tracker sees a
//!   real change), dispatches the `input` event `onChange` listens to, and then
//!   dispatches the same `KeyboardEvent('keydown')` with `key: 'Enter'` that the
//!   field's own `onKeyDown` (`handleCommandKey`) turns into `submitCommand` —
//!   the app's command funnel, not a call to `cad_command` from outside.
//!
//! They are still **synthetic** events: no kernel/evdev/XInput event is
//! injected, so a run must never be described as driven by a real mouse or a
//! real keyboard. The pointer picks are dispatched by
//! [`super::tool::tool_click_world_js`], which is the repo's existing
//! `PointerEvent` template for `.native-input`.

/// `Ctrl+1`: one synthetic `keydown` for the app's global properties toggle.
///
/// The event carries `ctrlKey` and `key: '1'` (the only two fields the handler
/// reads) plus `code`/`keyCode`/`which` so it looks like the chord a browser
/// reports. The returned string says whether the handler called
/// `preventDefault` — the app does, so a run that reports `accepted` really
/// reached the branch (a `prevented` report would mean some earlier listener
/// swallowed the chord first, which the driver must not paper over).
pub(super) const CTRL_1_JS: &str = r#"
(function () {
  try {
    var event = new KeyboardEvent('keydown', {
      key: '1', code: 'Digit1', keyCode: 49, which: 49,
      ctrlKey: true, metaKey: false, altKey: false, shiftKey: false,
      bubbles: true, cancelable: true
    });
    window.dispatchEvent(event);
    return event.defaultPrevented ? 'ctrl+1-dispatched-prevented' : 'ctrl+1-dispatched-unhandled';
  } catch (e) { return 'error:' + e; }
})()
"#;

/// Read back what the toggle produced: the docked palette's presence and the
/// live CAD partition rect (the palette shrinks it, so the pair is a real read
/// of the layout, not a copy of what the driver intended).
///
/// `.properties` is the palette's own root
/// (`apps/desktop-ui/src/components/PropertiesPalette.jsx`,
/// `className="palette properties"`); `.native-input` is the CAD partition
/// (`apps/desktop-ui/src/components/Workspace.jsx`).
pub(super) const PALETTE_READ_JS: &str = r#"
(function () {
  try {
    var palette = document.querySelector('.properties');
    var canvas = document.querySelector('.native-input');
    var rect = canvas ? canvas.getBoundingClientRect() : null;
    var canvasText = rect ? (Math.round(rect.width) + 'x' + Math.round(rect.height)) : '-';
    return 'palette=' + (palette ? 'present' : 'absent') + ' canvas=' + canvasText;
  } catch (e) { return 'error:' + e; }
})()
"#;

/// Whether the React chrome has mounted the two elements every later step needs
/// (the command field and the CAD partition).
///
/// The driver waits on this before dispatching `Ctrl+1`: a `keydown` sent to a
/// page whose `App.jsx` listener is not attached yet would be lost, and the
/// palette read two seconds later would then report an absence the run never
/// really tested.
pub(super) const MOUNTED_JS: &str = r#"
(function () {
  try {
    var command = document.getElementById('cad-command');
    var canvas = document.querySelector('.native-input');
    return (command && canvas) ? 'mounted' : 'not-mounted';
  } catch (e) { return 'error:' + e; }
})()
"#;

/// Read back what [`super::tool::tool_click_world_js`] recorded on
/// `window.__yacr_tool` for the selection click (its own dispatch counters and
/// the pixel/world point it resolved), so the selection line can show whether
/// the click was really dispatched before the `cad_status.selection` was read.
///
/// The template writes those fields from an async `.then`, so this read is a
/// real recount at read time, not a copy of the driver's intent.
pub(super) const CLICK_READBACK_JS: &str = r#"
(function () {
  var t = window.__yacr_tool || {};
  function v(x) {
    if (x === null || typeof x === 'undefined') { return '-'; }
    try { return String(x).replace(/\s+/g, '_'); } catch (e) { return '?'; }
  }
  try {
    return 'clicks=' + (t.clicks || 0) +
      ' last_click=' + v(t.last_click) +
      ' last_world=' + v(t.last_world) +
      ' clicks_failed=' + v(t.clicks_failed);
  } catch (e) { return 'error:' + e; }
})()
"#;

/// Template for "type this text into the command line and press Enter".
///
/// `__TEXT__` is substituted by [`type_command_js`] with a constant the driver
/// itself owns (`DIST` / `OPTIONS`) — nothing here is user input.
///
/// The Enter press is dispatched on a short timeout so it lands **after** React
/// has flushed the `input` event's state update: the field's `onKeyDown` reads
/// `shell.command` through `effectiveText()`, and pressing Enter inside the same
/// task as the value change could submit the previous text.
pub(super) const COMMAND_TYPE_TEMPLATE: &str = r#"
(function () {
  var text = '__TEXT__';
  try {
    var el = document.getElementById('cad-command');
    if (!el) { el = document.querySelector('.commandline__input'); }
    if (!el) { return 'no-command-input'; }
    if (el.disabled) { return 'command-input-disabled'; }
    var setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    if (!setter) { return 'no-value-setter'; }
    setter.call(el, text);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    window.setTimeout(function () {
      el.dispatchEvent(new KeyboardEvent('keydown', {
        key: 'Enter', code: 'Enter', keyCode: 13, which: 13,
        shiftKey: false, metaKey: false, altKey: false, ctrlKey: false,
        bubbles: true, cancelable: true
      }));
    }, 60);
    return 'typed+enter-scheduled text=' + text;
  } catch (e) { return 'error:' + e; }
})()
"#;

/// The command-field text + Enter script for one constant command name.
///
/// Synthetic keyboard input, **not** a real keyboard: the events are dispatched
/// on the field the chrome already listens to.
pub(super) fn type_command_js(text: &str) -> String {
    COMMAND_TYPE_TEMPLATE.replace("__TEXT__", text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_1_dispatches_a_real_keydown_for_the_apps_global_handler() {
        for expected in [
            "new KeyboardEvent('keydown'",
            "key: '1'",
            "code: 'Digit1'",
            "ctrlKey: true",
            "window.dispatchEvent(event)",
            "event.defaultPrevented",
        ] {
            assert!(
                CTRL_1_JS.contains(expected),
                "CTRL_1_JS is missing {expected}"
            );
        }
        // The app's handler reads exactly these two fields.
        assert!(CTRL_1_JS.contains("ctrlKey: true"));
        assert!(CTRL_1_JS.contains("key: '1'"));
    }

    #[test]
    fn palette_read_selects_the_palette_root_and_the_live_cad_partition() {
        for expected in [
            "document.querySelector('.properties')",
            "document.querySelector('.native-input')",
            "getBoundingClientRect()",
            "'palette=' + (palette ? 'present' : 'absent')",
        ] {
            assert!(
                PALETTE_READ_JS.contains(expected),
                "PALETTE_READ_JS is missing {expected}"
            );
        }
    }

    #[test]
    fn the_type_script_uses_the_fields_own_input_and_enter_handlers() {
        for expected in [
            "document.getElementById('cad-command')",
            "document.querySelector('.commandline__input')",
            "Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')",
            "new Event('input'",
            "new KeyboardEvent('keydown'",
            "key: 'Enter'",
            "keyCode: 13",
            "bubbles: true",
        ] {
            assert!(
                COMMAND_TYPE_TEMPLATE.contains(expected),
                "COMMAND_TYPE_TEMPLATE is missing {expected}"
            );
        }
        // The Enter press must land after React flushed the input event.
        assert!(COMMAND_TYPE_TEMPLATE.contains("window.setTimeout("));
    }

    #[test]
    fn type_command_js_substitutes_the_whole_text_once() {
        let script = type_command_js("DIST");
        assert!(script.contains("var text = 'DIST';"), "{script}");
        assert!(!script.contains("__TEXT__"), "placeholder left: {script}");
        assert_eq!(script.matches("DIST").count(), 1, "{script}");

        let options = type_command_js("OPTIONS");
        assert!(options.contains("var text = 'OPTIONS';"));
        assert!(!options.contains("__TEXT__"));
    }

    #[test]
    fn mounted_read_needs_both_the_command_field_and_the_cad_partition() {
        for expected in [
            "document.getElementById('cad-command')",
            "document.querySelector('.native-input')",
            "(command && canvas) ? 'mounted' : 'not-mounted'",
        ] {
            assert!(
                MOUNTED_JS.contains(expected),
                "MOUNTED_JS is missing {expected}"
            );
        }
        // The driver compares against this exact literal.
        assert!(MOUNTED_JS.contains("'mounted'"));
    }

    #[test]
    fn click_readback_reports_the_tool_templates_own_counters() {
        for expected in [
            "window.__yacr_tool || {}",
            "'clicks=' + (t.clicks || 0)",
            "last_click=",
            "last_world=",
            "clicks_failed=",
            "replace(/\\s+/g, '_')",
        ] {
            assert!(
                CLICK_READBACK_JS.contains(expected),
                "CLICK_READBACK_JS is missing {expected}"
            );
        }
    }

    #[test]
    fn every_blob_is_a_self_invoking_script_a_webview_can_eval() {
        for script in [
            CTRL_1_JS,
            PALETTE_READ_JS,
            MOUNTED_JS,
            CLICK_READBACK_JS,
            &type_command_js("DIST"),
        ] {
            let trimmed = script.trim();
            assert!(trimmed.starts_with("(function () {"), "{trimmed}");
            assert!(trimmed.ends_with(")()"), "{trimmed}");
            // Every blob catches its own failures and returns a string, so an
            // eval callback always receives something instead of a rejection.
            assert!(trimmed.contains("catch (e) { return 'error:' + e; }"));
        }
    }
}
