package dev.yacr.app

import android.content.res.Configuration
import android.os.Bundle
import android.util.Log
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import kotlin.math.roundToInt

/**
 * The Android host activity: a Tauri 2 WebView carrier for the shared React chrome
 * and the wasm `dom-canvas` CAD session (ADR 0024). It holds **no** document state
 * (iron rule 8); the only thing it contributes to the page is a platform fact the
 * page cannot read for itself.
 *
 * ## Why this class publishes `--yacr-safe-bottom`
 *
 * `enableEdgeToEdge()` (Tauri's template, kept) makes the WebView own the whole
 * window, so the system gesture pill is drawn **over** the page instead of pushing
 * it up. The page reserves room with `padding-bottom: calc(8px + max(
 * env(safe-area-inset-bottom, 0px), var(--yacr-safe-bottom, 0px)))`
 * (`apps/desktop-ui/src/mobile.css`), but `env()` inside the WebView reports the
 * *display cutout* safe area, not the gesture-navigation inset, so on a phone the
 * command field ran under the pill: measured 2026-10-28 on the emulator, the pill
 * occupied y 2364..2373 while the field spanned y 2273..2383 — the pill's top edge
 * was 19 px **inside** the field.
 *
 * The reserved height is a **host** fact (`WindowInsets`), so the host publishes it:
 * the navigation-bar inset, in CSS px, as the custom property
 * `--yacr-safe-bottom` on `document.documentElement`. It is re-published whenever
 * the insets change (rotation, gesture/3-button switch, window resize) and once
 * more when the page has its first visual state, because a value written before the
 * document exists dies with the previous document.
 *
 * Deliberately **not** done here: hiding, recolouring or otherwise restyling the
 * system bars, padding a native view (the page is the owner of its own layout), or
 * any device-specific constant — the number always comes from `WindowInsets`.
 */
class MainActivity : TauriActivity() {
  /** The live WebView, once Tauri has created it (`onWebViewCreate`). */
  private var webView: WebView? = null
  /** Last published inset, in CSS px, so repeated dispatches do not spam logcat. */
  private var safeBottomCssPx = NO_INSET
  /** The value a write has been **confirmed** against the live document with. */
  private var appliedCssPx = NO_INSET

  companion object {
    private const val TAG = "yacr"
    /** One outstanding `postVisualStateCallback` request is all this host needs. */
    private const val VISUAL_STATE_REQUEST_ID = 1L
    /** Sentinel: no inset has been measured / written yet. */
    private const val NO_INSET = Int.MIN_VALUE
    /**
     * How long to keep confirming the write. The Activity cannot observe the page
     * load: `WryActivity` owns the `WebViewClient` (it is not gettable or
     * replaceable), so `onPageFinished` is not callable from here, and a write
     * issued before the document is replaced silently dies with the old one. So the
     * host verifies its own write and retries only while the page still reports a
     * pre-navigation document — bounded, self-stopping, and it logs when it gives up.
     */
    private const val RETRY_DELAY_MS = 100L
    private const val MAX_ATTEMPTS = 40
    /** Bounded wait for React to mount its shell before a layout snapshot is logged. */
    private const val MAX_LOG_ATTEMPTS = 25
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // The content frame is the activity's own root: it sees the window insets even
    // before (and independently of) the WebView being attached, and it returns them
    // unchanged, so nothing downstream loses an inset it used to get.
    ViewCompat.setOnApplyWindowInsetsListener(findViewById(android.R.id.content)) { _, insets ->
      publishSafeBottom(navigationBarBottomPx(insets))
      insets
    }
  }

  /** Tauri's documented hook: runs once, when the carrier's WebView exists. */
  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    this.webView = webView
    // Whatever was measured before the WebView existed would otherwise be lost.
    pushSafeBottom()
    // And re-apply once the page has a visual state to commit (the first push may
    // have landed in the pre-navigation document; `applySafeBottom` confirms and
    // retries on its own, this is just the earliest well-defined trigger).
    webView.postVisualStateCallback(
      VISUAL_STATE_REQUEST_ID,
      object : WebView.VisualStateCallback() {
        override fun onComplete(requestId: Long) = pushSafeBottom()
      },
    )
  }

  /**
   * Rotation does not recreate this activity (`android:configChanges` covers
   * orientation/screenSize), so ask for a fresh inset dispatch instead of waiting
   * for some later layout pass to produce one — and log the resolved geometry
   * again, because the inset itself may be unchanged (the gesture bar keeps its
   * height in landscape) while the viewport is not.
   */
  override fun onConfigurationChanged(newConfig: Configuration) {
    super.onConfigurationChanged(newConfig)
    window?.decorView?.requestApplyInsets()
    webView?.let { view -> view.postDelayed({ logLayout(view) }, 600) }
  }

  /**
   * The height the platform reserves at the bottom of the window, in physical px.
   *
   * `WindowInsetsCompat.Type.navigationBars()` is the right subset on every
   * supported API level: on API 30+ it is the framework's own navigation-bar
   * inset, and on older releases androidx derives it from `systemWindowInsets`
   * (whose `bottom` is the same navigation-bar height — the top value in that
   * legacy mapping also contains the status bar, which is why only `bottom` is
   * read here). No `Build.VERSION` branch is needed, and no new dependency.
   */
  private fun navigationBarBottomPx(insets: WindowInsetsCompat): Int =
    insets.getInsets(WindowInsetsCompat.Type.navigationBars()).bottom.coerceAtLeast(0)

  private fun publishSafeBottom(bottomPx: Int) {
    val density = resources.displayMetrics.density
    // CSS px are density-independent px: the page is `width=device-width` at
    // `initial-scale=1`, so physical px / density is the unit `mobile.css` pads in.
    val cssPx = if (density > 0f) (bottomPx / density).roundToInt() else 0
    if (cssPx == safeBottomCssPx) return
    safeBottomCssPx = cssPx
    // A changed inset invalidates the confirmation: the document has to be written
    // again, whatever the previous value was.
    appliedCssPx = NO_INSET
    Log.i(TAG, "safe-bottom inset: ${bottomPx}px physical / density=$density -> ${cssPx}px CSS")
    pushSafeBottom()
  }

  private fun pushSafeBottom() {
    val view = webView ?: return
    val cssPx = safeBottomCssPx
    if (cssPx == NO_INSET) return
    if (appliedCssPx == cssPx) return
    view.post { applySafeBottom(view, cssPx, 1) }
  }

  /**
   * Write `--yacr-safe-bottom` and **confirm** the write reached a real document.
   *
   * `about:` is the pre-navigation document the WebView starts on: it does have a
   * `documentElement`, so a blind write would look successful and then vanish. The
   * script therefore reports `wait` there, and the page's own `documentElement`
   * reports `ok:<value>` once the property is readable back.
   */
  private fun applySafeBottom(view: WebView, cssPx: Int, attempt: Int) {
    if (view !== webView || cssPx != safeBottomCssPx) return
    val value = "${cssPx}px"
    view.evaluateJavascript(
      "(function(){var r=document.documentElement;" +
        "if(!r||location.protocol==='about:')return 'wait';" +
        "r.style.setProperty('--yacr-safe-bottom','$value');" +
        "return 'ok:'+r.style.getPropertyValue('--yacr-safe-bottom');})();",
    ) { raw ->
      if (view !== webView || cssPx != safeBottomCssPx) return@evaluateJavascript
      // `evaluateJavascript` hands the JS string back JSON-encoded.
      if (raw != null && raw.trim() == "\"ok:$value\"") {
        val first = appliedCssPx != cssPx
        appliedCssPx = cssPx
        if (first) {
          Log.i(TAG, "safe-bottom CSS var confirmed on the document: $value (attempt $attempt)")
          logLayout(view)
        }
      } else if (attempt < MAX_ATTEMPTS) {
        view.postDelayed({ applySafeBottom(view, cssPx, attempt + 1) }, RETRY_DELAY_MS)
      } else {
        Log.w(TAG, "safe-bottom CSS var NOT applied after $attempt attempts (last read: $raw)")
      }
    }
  }

  /**
   * Log the geometry the page actually resolved, once per inset confirmation and
   * once per rotation: the CSS viewport, the layout attribute, and the boxes of the
   * chrome the platform inset has to clear (the top bar, its menu button, the
   * command field, the CAD canvas). `logcat` therefore states the numbers this
   * host's one platform contribution was judged on, instead of leaving them to be
   * inferred from a screenshot, and it is the same numbers in either orientation.
   *
   * React mounts after the document exists, so an early call reports nothing; it
   * waits for the shell's `data-layout` attribute (bounded) instead of logging an
   * empty snapshot that reads like "the layout failed".
   */
  private fun logLayout(view: WebView, attempt: Int = 1) {
    view.evaluateJavascript(
      "(function(){" +
        "var shell=document.querySelector('main.shell');" +
        "if(!shell||!shell.hasAttribute('data-layout')||!document.getElementById('cad-command'))" +
        "return 'wait';" +
        "var box=function(sel){var e=document.querySelector(sel);if(!e)return null;" +
        "var r=e.getBoundingClientRect();return [Math.round(r.left*10)/10,Math.round(r.top*10)/10," +
        "Math.round(r.width*10)/10,Math.round(r.height*10)/10];};" +
        "return JSON.stringify({" +
        "layout:document.documentElement.getAttribute('data-layout')," +
        "vw:window.innerWidth,vh:window.innerHeight,dpr:window.devicePixelRatio," +
        "safeBottom:document.documentElement.style.getPropertyValue('--yacr-safe-bottom')," +
        "topbar:box('.mobile-topbar'),menu:box('.mobile-topbar__menu')," +
        "field:box('#cad-command'),canvas:box('.cad-canvas'),ribbon:box('.ribbon')});})();",
    ) { raw ->
      if (view !== webView) return@evaluateJavascript
      if (raw != null && raw.trim() == "\"wait\"" && attempt < MAX_LOG_ATTEMPTS) {
        view.postDelayed({ logLayout(view, attempt + 1) }, RETRY_DELAY_MS)
      } else {
        Log.i(TAG, "page layout: $raw")
      }
    }
  }
}
