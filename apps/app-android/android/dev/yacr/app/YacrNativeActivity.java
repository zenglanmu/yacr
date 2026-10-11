package dev.yacr.app;

import android.app.NativeActivity;
import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.provider.OpenableColumns;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * The declared launcher activity: a thin {@link NativeActivity} subclass that
 * additionally mediates Storage Access Framework (SAF) open/save pickers.
 *
 * <p>The pure-Rust {@code android-activity} native-activity stack cannot receive
 * {@code onActivityResult} (it uses the framework {@code android.app.NativeActivity}
 * directly, with no Java subclass to override), so this class exists for exactly
 * one reason: to run the SAF picker on the activity and hand the result back to
 * native code.
 *
 * <p>Unlike a separate proxy activity this does not need a second manifest
 * {@code <activity>} entry — {@code cargo-apk} declares only one activity, and
 * rewriting the compiled binary {@code AndroidManifest.xml} is fragile. The
 * activity's {@code android:name} is set (via {@code [package.metadata.android.application.activity]})
 * to this class, which subclasses {@code android.app.NativeActivity} and therefore
 * keeps the exact native lifecycle the Rust host depends on.
 *
 * <p>The Rust host drives it through the static helpers below (via JNI) and reads
 * the outcome through {@link #snapshot()} (a single synchronized read). Only one
 * interaction is in flight at a time (the host gates open/save), so a single
 * shared result slot is enough. The purpose of an interaction (open vs save) is
 * owned by the Rust side and is <em>not</em> mirrored here, so the two sides
 * cannot disagree about it.
 */
public final class YacrNativeActivity extends NativeActivity {
    // NOTE: these must NOT be named RESULT_OK/RESULT_CANCELED — those are
    // inherited from `android.app.Activity` (RESULT_OK == -1) and a same-named
    // constant here would shadow them, making `resultCode == RESULT_OK` compare
    // against the wrong value. The values are mirrored by `saf.rs` (STATE_*).
    public static final int STATE_NONE = 0;
    public static final int STATE_OK = 1;
    public static final int STATE_CANCELLED = 2;
    public static final int STATE_FAILED = 3;

    private static final int REQUEST_OPEN = 1001;
    private static final int REQUEST_SAVE = 1002;

    /** Guards every access to the result slots. */
    private static final Object LOCK = new Object();

    // Result state read from native via JNI. Written only under LOCK and read
    // only through `snapshot()` (also under LOCK), so the reader always observes
    // a consistent tuple even on a weakly ordered CPU.
    private static int sResult = STATE_NONE;
    private static String sUri = null;
    private static String sName = null;
    private static String sError = null;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
    }

    /** Reset the result slots for a new interaction. */
    public static void begin() {
        synchronized (LOCK) {
            sResult = STATE_NONE;
            sUri = null;
            sName = null;
            sError = null;
        }
    }

    /**
     * Atomically snapshot the result slots for the native bridge.
     *
     * <p>Native code must read the outcome through this single synchronized call
     * rather than reading the individual fields: a picker result written by
     * {@link #onActivityResult} would otherwise be observable half-applied on a
     * weakly ordered CPU (plain JNI field reads carry no happens-before edge).
     *
     * @return {@code {sResult, sUri, sName, sError}}
     */
    public static Object[] snapshot() {
        synchronized (LOCK) {
            return new Object[] { sResult, sUri, sName, sError };
        }
    }

    /**
     * Launch the SAF picker for a pending interaction. Called from the host's
     * command handler, which runs on the {@code android_main} (Slint event-loop)
     * thread — the same thread that owns the activity's JNI environment.
     */
    public static void launch(Context context, boolean save, String defaultName) {
        Intent intent;
        if (save) {
            intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType("application/octet-stream");
            if (defaultName != null && !defaultName.isEmpty()) {
                intent.putExtra(Intent.EXTRA_TITLE, defaultName);
            }
        } else {
            intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            // A bare `*/*` keeps every document selectable; a narrower
            // `EXTRA_MIME_TYPES` list would *disable* (grey out) any drawing whose
            // provider-reported MIME type is not in the list, which is the common
            // case for `.dwg`/`.dxf` (usually `application/octet-stream`).
            intent.setType("*/*");
        }
        if (context instanceof YacrNativeActivity) {
            YacrNativeActivity activity = (YacrNativeActivity) context;
            activity.startActivityForResult(intent, save ? REQUEST_SAVE : REQUEST_OPEN);
        } else {
            // Defensive: the host always passes the activity, but a plain context
            // still gets a usable (result-less) picker rather than a crash.
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            context.startActivity(intent);
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        if (requestCode != REQUEST_OPEN && requestCode != REQUEST_SAVE) {
            // Not one of ours (e.g. a system request); leave the slots alone.
            super.onActivityResult(requestCode, resultCode, data);
            return;
        }
        synchronized (LOCK) {
            if (resultCode != RESULT_OK) {
                sResult = STATE_CANCELLED;
            } else if (data == null || data.getData() == null) {
                // RESULT_OK with no document is a failure, not a cancel.
                sError = "the picker returned RESULT_OK without a document URI";
                sResult = STATE_FAILED;
            } else {
                Uri uri = data.getData();
                if (requestCode == REQUEST_OPEN) {
                    // Best effort: not every provider offers persistable grants,
                    // and the immediate session grant is still usable. A failure
                    // here must NOT discard the successful pick.
                    try {
                        getContentResolver().takePersistableUriPermission(
                            uri, data.getFlags() & Intent.FLAG_GRANT_READ_URI_PERMISSION);
                    } catch (Exception ignored) {
                        // Non-persistable provider; the session grant still applies.
                    }
                }
                sUri = uri.toString();
                sName = queryDisplayName(uri);
                sResult = STATE_OK;
            }
        }
        super.onActivityResult(requestCode, resultCode, data);
    }

    /** Read the whole document at {@code uriString}; throws on any failure. */
    public static byte[] readUri(Context context, String uriString) throws IOException {
        Uri uri = Uri.parse(uriString);
        try (InputStream in = context.getContentResolver().openInputStream(uri);
             ByteArrayOutputStream out = new ByteArrayOutputStream()) {
            if (in == null) {
                throw new IOException("openInputStream returned null");
            }
            byte[] buffer = new byte[64 * 1024];
            int read;
            while ((read = in.read(buffer)) != -1) {
                out.write(buffer, 0, read);
            }
            return out.toByteArray();
        }
    }

    /** Truncate-and-write {@code bytes} to the document at {@code uriString}. */
    public static void writeUri(Context context, String uriString, byte[] bytes) throws IOException {
        Uri uri = Uri.parse(uriString);
        try (OutputStream out = context.getContentResolver().openOutputStream(uri, "wt")) {
            if (out == null) {
                throw new IOException("openOutputStream returned null");
            }
            out.write(bytes);
            out.flush();
        }
    }

    private String queryDisplayName(Uri uri) {
        try (Cursor cursor = getContentResolver().query(
                uri, new String[] { OpenableColumns.DISPLAY_NAME }, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                int index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (index >= 0) {
                    String name = cursor.getString(index);
                    if (name != null && !name.isEmpty()) {
                        return name;
                    }
                }
            }
        } catch (Exception ignored) {
            // Fall back to the URI's last path segment below.
        }
        String last = uri.getLastPathSegment();
        return last != null ? last : uri.toString();
    }
}
