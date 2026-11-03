package dev.yacr.app

import android.content.res.AssetManager
import android.content.res.Configuration
import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import android.util.Log
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import java.io.File
import kotlin.math.roundToInt

/**
 * The Android host activity: a Tauri 2 WebView carrier for the shared React chrome.
 * On this **native-below** carrier the CAD pixels come from the native wgpu renderer,
 * not from the WebView: the Activity owns the `SurfaceView` that carries the native
 * window and nothing else. It holds **no** document state; the only thing it
 * contributes to the page is a platform fact the page cannot read for itself.
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
 *
 * ## The native CAD surface (`SurfaceView` + JNI)
 *
 * On this carrier the CAD pixels come from the native wgpu renderer, not from the
 * WebView. Tauri's window `transparent` flag is **not supported on Android**, so
 * transparency is arranged per-view instead: a `SurfaceView` is inserted at index
 * `0` of the WebView's parent (the content frame), i.e. **behind** the WebView, and
 * the WebView's own background is cleared. The platform's normal `SurfaceView`
 * hole-punch then composites the native surface through the transparent page.
 * `setZOrderOnTop(false)` is the whole point: an on-top surface would cover the
 * React chrome.
 *
 * The four `native*` externals are the **frozen** JNI contract implemented by the
 * Rust lane in the same `libapp_android.so` Tauri loads (`Rust` object). The holder
 * callbacks run on the main thread and forward synchronously; the asset handshake
 * is the only call posted from a worker thread. Nothing here holds CAD state
 * (`AGENTS.md` iron rule 8): the `Long` is an opaque native handle.
 */
class MainActivity : TauriActivity() {
  /** The live WebView, once Tauri has created it (`onWebViewCreate`). */
  private var webView: WebView? = null
  /** The native wgpu CAD surface, layered below the WebView in the window. */
  private var cadSurface: SurfaceView? = null
  /** Opaque handle from `nativeSurfaceCreated`; `0` means no live native surface. */
  private var cadSurfaceHandle = 0L
  /** One-shot guard: Wry calls `onWebViewCreate` before the WebView is attached. */
  private var cadSurfaceInserted = false
  /** Last published inset, in CSS px, so repeated dispatches do not spam logcat. */
  private var safeBottomCssPx = NO_INSET
  /** The value a write has been **confirmed** against the live document with. */
  private var appliedCssPx = NO_INSET
  /**
   * SAF open/save mediation. It owns only one pending picker/read result, never a
   * document (`AGENTS.md` iron rule 8). Lazy so an early JNI call cannot hit an
   * uninitialised property; the only setup step is `register()` in `onCreate`.
   */
  private val documentPicker: DocumentPicker by lazy { DocumentPicker(this) }

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

    /** Asset directory packaged from the repository's committed `fonts/` package. */
    private const val FONT_ASSET_DIR = "fonts"
    /** Optional asset directory for CAD raster resources (absent in this repo today). */
    private const val IMAGE_ASSET_DIR = "images"
    /** The catalogue the native font loader requires next to the faces. */
    private const val FONT_CATALOG = "fonts.json"
    /**
     * Written after a complete copy; presence + catalog makes extraction a no-op
     * for this asset version. Bump the version whenever the bundled package gains
     * a file, so existing installs re-extract instead of serving the old set (v2
     * adds the Noto Sans CJK SC fallback).
     */
    private const val ASSET_EXTRACT_MARKER = ".yacr-assets-v2-complete"
    /** Font-package files when Gradle exposes the repository's `fonts/` as an asset root. */
    private val FONT_SUFFIXES = listOf(".ttf", ".otf", ".ttc", ".woff", ".woff2", ".shx", ".cxf")
  }

  // ---------------------------------------------------------------------------
  // Frozen Kotlin -> Rust JNI contract (implemented in `libapp_android.so` by the
  // native renderer lane; names and signatures must not change here).
  //
  // JNI symbols are `Java_dev_yacr_app_MainActivity_<name>` with an instance
  // receiver (`this`). Surface callbacks run on the main thread, so the calls are
  // synchronous; the asset handshake is posted to the main thread after extraction.
  // ---------------------------------------------------------------------------

  private external fun nativeSurfaceCreated(surface: Surface, width: Int, height: Int): Long
  private external fun nativeSurfaceResized(handle: Long, width: Int, height: Int)
  private external fun nativeSurfaceDestroyed(handle: Long)
  private external fun nativeAssetsReady(fontsDir: String, imagesDir: String)

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // `super.onCreate(...)` has already initialised the `Rust` object, which loads
    // `libapp_android.so`, so the JNI calls below can resolve.
    //
    // The SAF launcher must be registered while the activity is CREATED — after
    // `super.onCreate` and before STARTED — and never through `onActivityResult`.
    documentPicker.register()
    // An external "Open with" (VIEW) intent handled by this same launch: the async
    // read runs independently, so it works even before the Rust CAD session exists;
    // Rust simply polls the slot when it starts.
    documentPicker.handleIncomingIntent(intent)
    extractBundledAssetsOnce()
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
    // The native CAD surface is a separate window layer *behind* this window. Clear
    // only the WebView's own background so the page composites through to it; the
    // page keeps its CSS backgrounds and `--yacr-safe-bottom` (published below).
    webView.setBackgroundColor(Color.TRANSPARENT)
    insertCadSurfaceBehind(webView)
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
   * An "Open with" (VIEW) intent that arrives while this activity is already
   * running: the manifest is `singleTask`, so the document is delivered here rather
   * than through a new launch. Same slot as `onCreate`: the read starts immediately
   * and Rust consumes the result whenever it polls.
   */
  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    setIntent(intent)
    documentPicker.handleIncomingIntent(intent)
  }

  /**
   * No native pause symbol is part of the frozen contract, and none is invented
   * here. `super.onPause()` still reaches `Rust.onPause` (the Tauri loop) and
   * `RustWebView.onPause` through `WryActivity`. When the window is no longer
   * visible the platform destroys the `SurfaceView`'s surface, which arrives as
   * `surfaceDestroyed` -> `nativeSurfaceDestroyed(handle)`: the renderer loses its
   * target and cannot draw off-screen. Resuming produces a fresh
   * `surfaceCreated`/handle.
   */
  override fun onPause() {
    super.onPause()
  }

  // ---------------------------------------------------------------------------
  // Native surface lifecycle
  // ---------------------------------------------------------------------------

  private fun buildCadSurfaceView(): SurfaceView =
    SurfaceView(this).apply {
      // The native CAD layer must sit *below* the activity window (and below the
      // transparent WebView), never on top of it or above another SurfaceView.
      setZOrderOnTop(false)
      setZOrderMediaOverlay(false)
      holder.addCallback(
        object : SurfaceHolder.Callback {
          override fun surfaceCreated(holder: SurfaceHolder) {
            val frame = holder.surfaceFrame
            // A re-created surface without an intervening destroy must not leak the
            // previous wgpu surface.
            val previous = cadSurfaceHandle
            cadSurfaceHandle = 0L
            if (previous != 0L) nativeSurfaceDestroyed(previous)
            cadSurfaceHandle = nativeSurfaceCreated(holder.surface, frame.width(), frame.height())
            if (cadSurfaceHandle == 0L) {
              Log.w(TAG, "native CAD surface creation returned no handle; surface stays unbound")
            } else {
              Log.i(
                TAG,
                "native CAD surface created ${frame.width()}x${frame.height()} handle=$cadSurfaceHandle",
              )
            }
          }

          override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
            val handle = cadSurfaceHandle
            if (handle == 0L) return
            nativeSurfaceResized(handle, width, height)
            Log.i(TAG, "native CAD surface resized ${width}x${height} handle=$handle")
          }

          override fun surfaceDestroyed(holder: SurfaceHolder) {
            val handle = cadSurfaceHandle
            cadSurfaceHandle = 0L
            if (handle == 0L) return
            nativeSurfaceDestroyed(handle)
            Log.i(TAG, "native CAD surface destroyed handle=$handle")
          }
        },
      )
    }

  /**
   * Insert the native surface **behind** the WebView.
   *
   * Wry calls `onWebViewCreate` *before* `activity.setContentView(webview)`, so a
   * direct add to `android.R.id.content` there is wiped by that call. The attach
   * listener runs after the carrier attaches the WebView; `post` also covers a
   * carrier that changes that order. Index `0` of the content frame is below the
   * WebView child, and `setZOrderOnTop(false)` keeps the platform surface itself
   * behind the window.
   */
  private fun insertCadSurfaceBehind(webView: WebView) {
    // Created lazily: this hook can run from inside `super.onCreate` (Wry attaches
    // the WebView there), before the Activity body would otherwise build the view.
    val surface = cadSurface ?: buildCadSurfaceView().also { cadSurface = it }
    val insert = { parent: ViewGroup ->
      if (!cadSurfaceInserted) {
        parent.addView(
          surface,
          0,
          ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.MATCH_PARENT,
          ),
        )
        cadSurfaceInserted = true
        Log.i(TAG, "native CAD SurfaceView inserted at index 0 (below the WebView)")
      }
    }
    val insertIfAttached = {
      if (!cadSurfaceInserted) {
        val parent = webView.parent
        if (webView.isAttachedToWindow && parent is ViewGroup) insert(parent)
      }
    }
    webView.addOnAttachStateChangeListener(
      object : View.OnAttachStateChangeListener {
        override fun onViewAttachedToWindow(view: View) {
          view.removeOnAttachStateChangeListener(this)
          insertIfAttached()
        }

        override fun onViewDetachedFromWindow(view: View) = Unit
      },
    )
    webView.post { insertIfAttached() }
  }

  // ---------------------------------------------------------------------------
  // Bundled assets: APK assets -> filesDir, then the main-thread JNI handshake
  // ---------------------------------------------------------------------------

  /**
   * Copy the bundled CAD assets into the app's private files directory once.
   *
   * `nativeAssetsReady` takes **filesystem paths**, so APK assets must be extracted
   * to real files. This runs on a worker thread (no main-thread I/O); the callback
   * is posted to the main thread because every other call in the frozen contract is
   * main-thread.
   *
   * Idempotent: a marker written after a complete copy (plus `fonts.json`) makes
   * later Activity starts skip the copy, but they still announce the paths because
   * native state does not survive the process. When the APK has no `fonts` assets
   * this logs an explicit diagnostic and **does not call** `nativeAssetsReady` with
   * paths that would look like a valid but empty package.
   */
  private fun extractBundledAssetsOnce() {
    val assetManager = assets
    val filesDir = filesDir
    val fontsDir = File(filesDir, FONT_ASSET_DIR)
    val imagesDir = File(filesDir, IMAGE_ASSET_DIR)
    Thread(
      {
        val fontsReady = try {
          ensureFontsExtracted(assetManager, fontsDir)
        } catch (error: Exception) {
          Log.e(TAG, "bundled CAD assets: font extraction failed: ${error.message}")
          false
        }
        if (!fontsReady) {
          // No fake paths: native must be told nothing rather than pointed at an
          // empty directory that looks like a valid (fontless) package.
          Log.e(TAG, "bundled CAD assets: no '$FONT_ASSET_DIR' assets in the APK; nativeAssetsReady NOT called")
        } else {
          try {
            ensureImagesExtracted(assetManager, imagesDir)
          } catch (error: Exception) {
            Log.w(TAG, "bundled CAD assets: image extraction failed: ${error.message}")
          }
          // The contract always receives both paths; the directory is created so
          // the optional-images argument is a real path even when the repo ships
          // none.
          imagesDir.mkdirs()
          runOnUiThread {
            if (isFinishing || isDestroyed) return@runOnUiThread
            nativeAssetsReady(fontsDir.absolutePath, imagesDir.absolutePath)
            Log.i(
              TAG,
              "nativeAssetsReady(fonts=${fontsDir.absolutePath}, images=${imagesDir.absolutePath})",
            )
          }
        }
      },
      "yacr-asset-extract",
    ).start()
  }

  private fun ensureFontsExtracted(assets: AssetManager, fontsDir: File): Boolean {
    val marker = File(fontsDir, ASSET_EXTRACT_MARKER)
    val catalog = File(fontsDir, FONT_CATALOG)
    if (marker.isFile && catalog.isFile) {
      Log.i(TAG, "bundled CAD fonts already extracted: ${fontsDir.absolutePath}")
      return true
    }
    val nested = assets.list(FONT_ASSET_DIR)
    val copied = if (!nested.isNullOrEmpty()) {
      // Conventional layout: `assets/fonts/...`.
      copyAssetTree(assets, FONT_ASSET_DIR, fontsDir)
      true
    } else {
      // `assets.srcDir(<repo>/fonts)` makes the repository's package an asset
      // *root*, so its files appear flat under `assets/`. Copy only the package
      // files, never unrelated APK assets such as `tauri.conf.json`.
      copyFlatFontAssets(assets, fontsDir)
    }
    if (!copied || !catalog.isFile) return false
    marker.writeText("yacr android assets\n")
    return true
  }

  private fun copyFlatFontAssets(assets: AssetManager, fontsDir: File): Boolean {
    val root = assets.list("") ?: return false
    val fontFiles = root.filter { name ->
      name == FONT_CATALOG ||
        FONT_SUFFIXES.any { suffix -> name.endsWith(suffix, ignoreCase = true) } ||
        name.startsWith("COPYING", ignoreCase = true) ||
        name.startsWith("SOURCE", ignoreCase = true)
    }
    if (fontFiles.isEmpty()) return false
    fontsDir.mkdirs()
    for (name in fontFiles) copyAssetFile(assets, name, File(fontsDir, name))
    return true
  }

  private fun ensureImagesExtracted(assets: AssetManager, imagesDir: File) {
    if (assets.list(IMAGE_ASSET_DIR).isNullOrEmpty()) {
      Log.i(TAG, "bundled CAD assets: no '$IMAGE_ASSET_DIR' assets (optional)")
      return
    }
    copyAssetTree(assets, IMAGE_ASSET_DIR, imagesDir)
  }

  private fun copyAssetTree(assets: AssetManager, assetDir: String, targetDir: File) {
    val children = assets.list(assetDir) ?: return
    targetDir.mkdirs()
    for (child in children) {
      val assetPath = if (assetDir.isEmpty()) child else "$assetDir/$child"
      val grandchildren = assets.list(assetPath)
      if (!grandchildren.isNullOrEmpty()) {
        copyAssetTree(assets, assetPath, File(targetDir, child))
      } else {
        copyAssetFile(assets, assetPath, File(targetDir, child))
      }
    }
  }

  private fun copyAssetFile(assets: AssetManager, assetPath: String, target: File) {
    target.parentFile?.mkdirs()
    assets.open(assetPath).use { input ->
      target.outputStream().use { output -> input.copyTo(output) }
    }
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

  // ---------------------------------------------------------------------------
  // Rust -> Kotlin document contract (SAF open/save)
  //
  // The native host calls these on **this** activity by exact name (instance JNI:
  // `Java_dev_yacr_app_MainActivity_documentLaunchOpen`, …), mirroring the frozen
  // Kotlin -> Rust `native*` symbols above in the other direction. They carry no
  // document state: `DocumentPicker` holds only one in-flight picker result and the
  // bytes of one read, and every terminal state is consumed by
  // `documentSnapshot()`. Nothing here invents a document, a path or a success.
  // ---------------------------------------------------------------------------

  /** Launches `ACTION_OPEN_DOCUMENT` and arms the slot for the picked document. */
  fun documentLaunchOpen() {
    documentPicker.documentLaunchOpen()
  }

  /** Launches `ACTION_CREATE_DOCUMENT` with `defaultName` as the suggested title. */
  fun documentLaunchSave(defaultName: String) {
    documentPicker.documentLaunchSave(defaultName)
  }

  /** Starts an async read of `uri`; poll [documentSnapshot] for LOADING -> OK/FAILED. */
  fun documentReadUri(uri: String) {
    documentPicker.documentReadUri(uri)
  }

  /**
   * Synchronous truncating write of `bytes` to `uri`. Throws `IOException` on any
   * failure (the JNI caller reports it); it never reports an empty success.
   */
  fun documentWriteUri(uri: String, bytes: ByteArray) {
    documentPicker.documentWriteUri(uri, bytes)
  }

  /**
   * `[Int state, String? uri, String? name, String? error, ByteArray? bytes]`.
   * Terminal states (OK / CANCELLED / FAILED) are consumed by this call.
   */
  fun documentSnapshot(): Array<Any?> = documentPicker.documentSnapshot()
}
