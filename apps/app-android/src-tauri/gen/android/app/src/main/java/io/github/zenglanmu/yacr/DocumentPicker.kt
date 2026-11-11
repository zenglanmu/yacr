package io.github.zenglanmu.yacr

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.util.Log
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import java.io.ByteArrayOutputStream
import java.io.IOException

/**
 * Storage Access Framework (SAF) open/save mediation for the native-below Android
 * host.
 *
 * This class owns **no** CAD/document state (`AGENTS.md` iron rule 8): it owns one
 * *interaction slot* — the state of the one picker/read the native host asked for
 * and, at the very most, the bytes of one finished read. The Rust CAD session is the
 * only document authority; it drives this class through `MainActivity`'s five
 * `document*` methods (instance JNI) and **consumes** every terminal result with
 * [documentSnapshot], so a value here can never be mistaken for a second copy of the
 * document.
 *
 * The transport states are mirrored by the Rust side. Cancellation is a distinct
 * state, never a failure: a user who backs out of the picker produced no error, and
 * the host must be able to tell "cancelled" from "failed". A failure is always an
 * explicit [STATE_FAILED] with a message — there is no empty success and no
 * fabricated byte array.
 *
 * ## Why the open picker uses the wildcard MIME type
 *
 * `.dwg` providers routinely report `application/octet-stream` (or a vendor type)
 * instead of a drawing MIME type, so a narrow filter would grey out the very files
 * this host exists to open. `ACTION_OPEN_DOCUMENT` (not `ACTION_GET_CONTENT`) is the
 * SAF action: it returns a `content://` URI with a per-URI grant, which is exactly
 * what [documentReadUri] reads and [documentWriteUri] writes.
 */
class DocumentPicker(private val activity: MainActivity) {
  /** One immutable observation of the interaction slot. */
  private class Slot(
    val state: Int,
    val uri: String?,
    val name: String?,
    val error: String?,
    val bytes: ByteArray?,
  )

  companion object {
    /** No interaction pending. */
    const val STATE_NONE = 0
    /** A read is in flight; `uri`/`name` are already known. */
    const val STATE_LOADING = 1
    /** Terminal: a read produced `bytes`, or a save destination was chosen. */
    const val STATE_OK = 2
    /** Terminal: the user backed out of the picker (not an error). */
    const val STATE_CANCELLED = 3
    /** Terminal: the interaction failed; `error` says why. */
    const val STATE_FAILED = 4

    private const val TAG = "yacr"
    /** Read in 64 KiB chunks: one buffer for the whole document, no growth per read. */
    private const val READ_BUFFER_BYTES = 64 * 1024
    private const val READ_WORKER_NAME = "yacr-document-read"

    /** Nothing pending; shared because slots are immutable. */
    private val EMPTY = Slot(STATE_NONE, null, null, null, null)
  }

  /** Guards [generation] and every field of [slot] (JNI reads/writes cross threads). */
  private val lock = Any()

  /**
   * Monotonic interaction counter. A background read captures its value and only
   * publishes if it still matches, so a stale read can never overwrite a newer
   * picker/intent result.
   */
  private var generation = 0L

  private var slot = EMPTY

  /**
   * Whether the pending picker result is a save destination. Owned by the launch
   * calls and read once when the result arrives; the *purpose* of the interaction
   * otherwise stays on the Rust side (the two sides cannot disagree about it).
   */
  private var pendingSave = false

  private var launcher: ActivityResultLauncher<Intent>? = null

  /**
   * Registers the picker launcher. Must run in `MainActivity.onCreate` after
   * `super.onCreate` and before the activity is STARTED; `onActivityResult` is
   * deliberately not overridden.
   */
  fun register() {
    if (launcher != null) {
      Log.w(TAG, "document picker: register() called twice; keeping the first launcher")
      return
    }
    launcher = activity.registerForActivityResult(
      ActivityResultContracts.StartActivityForResult(),
    ) { result -> onPickerResult(result.resultCode, result.data) }
  }

  // ---------------------------------------------------------------------------
  // Rust -> Kotlin launch calls (one interaction at a time; Rust gates open/save)
  // ---------------------------------------------------------------------------

  /** Launches the SAF open picker (`ACTION_OPEN_DOCUMENT`), replacing the slot. */
  fun documentLaunchOpen(initialUri: String?) {
    resetSlot(save = false)
    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
      addCategory(Intent.CATEGORY_OPENABLE)
      // Bare `*/*` is intentional: `.dwg` is usually reported as
      // `application/octet-stream`, and a MIME filter would grey those files out.
      type = "*/*"
      addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
      // Start in the last opened drawing's folder when the host remembered one.
      // The extra is ignored by a provider without folder support, never an error.
      initialDocumentUri(initialUri)?.let { putExtra(DocumentsContract.EXTRA_INITIAL_URI, it) }
    }
    Log.i(TAG, "document picker: launch open (ACTION_OPEN_DOCUMENT, */*, initial=$initialUri)")
    launch(intent)
  }

  /**
   * The picker's starting SAF document URI derived from the last opened document.
   *
   * Android opens the containing folder of an initial *document* URI. When the
   * document id is a path (`dir/name`), the parent document URI is used so the
   * picker lands in the folder; otherwise the document URI itself is the best
   * available hint (the system still shows its folder). A non-`content:` URI is
   * not a SAF document and yields null: no invented location.
   */
  private fun initialDocumentUri(raw: String?): Uri? {
    if (raw.isNullOrBlank()) return null
    val uri = try {
      Uri.parse(raw)
    } catch (_: Exception) {
      return null
    }
    if (uri.scheme?.lowercase() != "content") return null
    return try {
      val documentId = DocumentsContract.getDocumentId(uri)
      val parentId = documentId.substringBeforeLast('/', missingDelimiterValue = "")
      val authority = uri.authority
      if (parentId.isEmpty() || authority == null) {
        uri
      } else {
        DocumentsContract.buildDocumentUri(authority, parentId)
      }
    } catch (_: Exception) {
      // A provider whose id is not a document id: the document URI is still a
      // valid initial hint, so keep it rather than dropping the whole gesture.
      uri
    }
  }

  /** Launches the SAF create-document picker (`ACTION_CREATE_DOCUMENT`). */
  fun documentLaunchSave(defaultName: String) {
    resetSlot(save = true)
    val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
      addCategory(Intent.CATEGORY_OPENABLE)
      type = "application/octet-stream"
      putExtra(Intent.EXTRA_TITLE, defaultName)
      addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
    }
    Log.i(TAG, "document picker: launch save (ACTION_CREATE_DOCUMENT, title='$defaultName')")
    launch(intent)
  }

  private fun launch(intent: Intent) {
    val active = launcher
    if (active == null) {
      fail("document picker: no launcher registered (register() must run in onCreate)")
      return
    }
    // The launch itself is an activity operation: it must happen on the UI thread
    // even when the JNI call arrives from the native render/tick thread.
    activity.runOnUiThread {
      if (activity.isFinishing || activity.isDestroyed) {
        fail("document picker: activity is finishing; the picker was not shown")
        return@runOnUiThread
      }
      try {
        active.launch(intent)
      } catch (error: Exception) {
        fail("document picker: launch failed: ${error.message}")
      }
    }
  }

  // ---------------------------------------------------------------------------
  // Reads and writes
  // ---------------------------------------------------------------------------

  /**
   * Arms a read of `uri`: the slot becomes LOADING (with URI and display name)
   * immediately, and the bytes are read on a worker thread — never on the UI or
   * native tick thread.
   */
  fun documentReadUri(uriString: String) {
    val target = Uri.parse(uriString)
    val displayName = queryDisplayName(target)
    beginRead(target, uriString, displayName, source = "rust")
  }

  /**
   * Truncating write of `bytes` to `uri`, synchronously (the caller's thread).
   * Throws [IOException] on failure so the native side reports a real error instead
   * of an empty success. No catch here is silent: every failure is logged first.
   */
  fun documentWriteUri(uriString: String, bytes: ByteArray) {
    val target = Uri.parse(uriString)
    Log.i(TAG, "document write: start uri=$uriString bytes=${bytes.size}")
    try {
      val output = activity.contentResolver.openOutputStream(target, "wt")
        ?: throw IOException("openOutputStream(..., \"wt\") returned null for $target")
      output.use { stream ->
        stream.write(bytes)
        stream.flush()
      }
      Log.i(TAG, "document write: done uri=$uriString bytes=${bytes.size}")
    } catch (error: IOException) {
      Log.e(TAG, "document write: failed uri=$uriString: ${error.message}")
      throw error
    } catch (error: Exception) {
      Log.e(TAG, "document write: failed uri=$uriString: ${error.message}")
      throw IOException("write failed for $uriString: ${error.message}", error)
    }
  }

  /**
   * One atomic read of the interaction slot for the native host.
   *
   * @return `[state, uri, name, error, bytes]`, with `state` one of the `STATE_*`
   *   constants. Terminal states (OK/CANCELLED/FAILED) are **consumed**: the slot is
   *   reset to NONE before this returns, so the same result is never delivered twice
   *   (and an old success cannot answer a new request). LOADING/NONE are returned
   *   without consuming.
   */
  fun documentSnapshot(): Array<Any?> {
    synchronized(lock) {
      val current = slot
      if (isTerminal(current.state)) {
        slot = EMPTY
      }
      val snapshot: Array<Any?> =
        arrayOf(current.state, current.uri, current.name, current.error, current.bytes)
      return snapshot
    }
  }

  // ---------------------------------------------------------------------------
  // External "Open with" (VIEW intent). Works before the Rust session exists.
  // ---------------------------------------------------------------------------

  /**
   * An external "Open with" / VIEW intent on a `content:` or `file:` URI. It starts
   * the same async read a picker would; the slot simply waits for Rust to poll it,
   * which is what makes "open with" work before the CAD session has started.
   */
  fun handleIncomingIntent(intent: Intent?) {
    val data = intent?.data ?: return
    val scheme = data.scheme?.lowercase() ?: return
    if (scheme != "content" && scheme != "file") return
    val uriString = data.toString()
    val displayName = queryDisplayName(data)
    Log.i(TAG, "document intent: external open uri=$uriString name=$displayName")
    beginRead(data, uriString, displayName, source = "intent")
  }

  // ---------------------------------------------------------------------------
  // Picker result
  // ---------------------------------------------------------------------------

  private fun onPickerResult(resultCode: Int, data: Intent?) {
    // The caller is the activity result callback (UI thread). Claim this result's
    // generation and the purpose of the picker that produced it.
    val (gen, save) = beginInteraction()

    if (resultCode != Activity.RESULT_OK) {
      // Back button / dismissed picker: a distinct state, not a failure.
      if (publish(gen, Slot(STATE_CANCELLED, null, null, null, null))) {
        Log.i(TAG, "document picker: result cancelled (resultCode=$resultCode, save=$save)")
      } else {
        Log.i(TAG, "document picker: stale cancel discarded (resultCode=$resultCode)")
      }
      return
    }

    val picked = data?.data
    if (picked == null) {
      // RESULT_OK without a document is a real failure, not a cancellation.
      val message = "the picker returned RESULT_OK without a document URI"
      if (publish(gen, Slot(STATE_FAILED, null, null, message, null))) {
        Log.e(TAG, "document picker: $message (save=$save)")
      }
      return
    }

    val pickedString = picked.toString()
    val displayName = queryDisplayName(picked)

    if (save) {
      // Save only needs the destination; the bytes are written later through
      // `documentWriteUri` (Rust owns the exported bytes).
      if (publish(gen, Slot(STATE_OK, pickedString, displayName, null, null))) {
        Log.i(TAG, "document picker: result save uri=$pickedString name=$displayName")
      } else {
        Log.i(TAG, "document picker: stale save result discarded (uri=$pickedString)")
      }
      return
    }

    // Best effort: not every provider offers persistable grants, and the immediate
    // session grant is usable anyway. A failure here must not discard the pick.
    try {
      activity.contentResolver.takePersistableUriPermission(
        picked,
        data.flags and Intent.FLAG_GRANT_READ_URI_PERMISSION,
      )
      Log.i(TAG, "document picker: persisted read permission for $pickedString")
    } catch (error: Exception) {
      Log.w(
        TAG,
        "document picker: persistable read grant unavailable for $pickedString: ${error.message}",
      )
    }

    if (!publish(gen, Slot(STATE_LOADING, pickedString, displayName, null, null))) {
      Log.i(TAG, "document picker: stale open result discarded (uri=$pickedString)")
      return
    }
    Log.i(TAG, "document picker: result open uri=$pickedString name=$displayName")
    readAsync(picked, pickedString, displayName, gen)
  }

  // ---------------------------------------------------------------------------
  // Slot plumbing (all field access under `lock`)
  // ---------------------------------------------------------------------------

  private fun resetSlot(save: Boolean) {
    synchronized(lock) {
      generation += 1
      pendingSave = save
      slot = EMPTY
    }
  }

  /** Claims a fresh generation plus the purpose of the picker that returned. */
  private fun beginInteraction(): Pair<Long, Boolean> = synchronized(lock) {
    generation += 1
    generation to pendingSave
  }

  /** Sets the slot to LOADING for a new read and returns the generation it owns. */
  private fun armLoading(uriString: String, displayName: String): Long = synchronized(lock) {
    generation += 1
    slot = Slot(STATE_LOADING, uriString, displayName, null, null)
    generation
  }

  /** Publishes `next` only if no newer interaction happened in the meantime. */
  private fun publish(gen: Long, next: Slot): Boolean {
    var applied = false
    synchronized(lock) {
      if (generation == gen) {
        slot = next
        applied = true
      }
    }
    return applied
  }

  private fun fail(message: String) {
    synchronized(lock) {
      generation += 1
      slot = Slot(STATE_FAILED, null, null, message, null)
    }
    Log.e(TAG, message)
  }

  private fun isTerminal(state: Int): Boolean =
    state == STATE_OK || state == STATE_CANCELLED || state == STATE_FAILED

  // ---------------------------------------------------------------------------
  // Worker read
  // ---------------------------------------------------------------------------

  private fun beginRead(target: Uri, uriString: String, displayName: String, source: String) {
    val gen = armLoading(uriString, displayName)
    Log.i(TAG, "document read: start uri=$uriString name=$displayName source=$source")
    readAsync(target, uriString, displayName, gen)
  }

  private fun readAsync(target: Uri, uriString: String, displayName: String, gen: Long) {
    val worker = Thread {
      var loaded: ByteArray? = null
      var failure: String? = null
      try {
        loaded = readBytes(target)
      } catch (error: Exception) {
        failure = error.message ?: error.javaClass.simpleName
      }
      val next = if (failure == null) {
        Slot(STATE_OK, uriString, displayName, null, loaded)
      } else {
        Slot(STATE_FAILED, uriString, displayName, "read failed for $uriString: $failure", null)
      }
      if (!publish(gen, next)) {
        // A newer picker/intent result won while this read was running: dropping the
        // bytes is the correct behaviour (never overwrite newer state).
        Log.i(TAG, "document read: stale result discarded (uri=$uriString)")
      } else if (failure != null) {
        Log.e(TAG, "document read: failed uri=$uriString: $failure")
      } else {
        Log.i(TAG, "document read: done uri=$uriString bytes=${loaded?.size ?: 0}")
      }
    }
    worker.name = READ_WORKER_NAME
    worker.start()
  }

  /** Reads the whole document; throws on any failure (never returns a short read). */
  private fun readBytes(target: Uri): ByteArray {
    val input = activity.contentResolver.openInputStream(target)
      ?: throw IOException("openInputStream returned null for $target")
    return input.use { stream ->
      val output = ByteArrayOutputStream()
      val buffer = ByteArray(READ_BUFFER_BYTES)
      var read = stream.read(buffer)
      while (read >= 0) {
        output.write(buffer, 0, read)
        read = stream.read(buffer)
      }
      output.toByteArray()
    }
  }

  /**
   * `OpenableColumns.DISPLAY_NAME`, else the URI's last path segment (URL-decoded),
   * else the URI itself. Providers may refuse the query; that is not a failure of the
   * interaction, so it is logged and the fallback is used.
   */
  private fun queryDisplayName(target: Uri): String {
    try {
      activity.contentResolver.query(
        target,
        arrayOf(OpenableColumns.DISPLAY_NAME),
        null,
        null,
        null,
      )?.use { cursor ->
        if (cursor.moveToFirst()) {
          val index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
          if (index >= 0) {
            val value: String? = cursor.getString(index)
            if (value != null && value.isNotEmpty()) return value
          }
        }
      }
    } catch (error: Exception) {
      Log.w(TAG, "document: display-name query failed for $target: ${error.message}")
    }
    val lastSegment = target.lastPathSegment
    if (!lastSegment.isNullOrEmpty()) {
      return try {
        Uri.decode(lastSegment)
      } catch (error: IllegalArgumentException) {
        Log.w(TAG, "document: undecodable path segment '$lastSegment': ${error.message}")
        lastSegment
      }
    }
    return target.toString()
  }
}
