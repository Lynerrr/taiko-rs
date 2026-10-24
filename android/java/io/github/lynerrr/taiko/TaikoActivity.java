package io.github.lynerrr.taiko;

import android.app.NativeActivity;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.provider.OpenableColumns;

import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;

/**
 * The game is native code (libtaiko_rs.so, see src/android.rs); this only adds importing beatmap
 * archives opened with or shared to the game. Each one is copied into
 * {@code <files>/taiko-rs/import/<name>.osz.part} and renamed to {@code <name>.osz} once complete;
 * the game picks it up from there and unpacks it into its Songs folder.
 */
public class TaikoActivity extends NativeActivity {
    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        // A recreated activity (saved != null) already imported its intent.
        if (saved == null) {
            importFrom(getIntent());
        }
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        importFrom(intent);
    }

    @SuppressWarnings("deprecation") // getParcelable*Extra(String): the typed versions need API 33.
    private void importFrom(Intent intent) {
        if (intent == null || intent.getAction() == null) {
            return;
        }
        ArrayList<Uri> uris = new ArrayList<>();
        switch (intent.getAction()) {
            case Intent.ACTION_VIEW:
                if (intent.getData() != null) {
                    uris.add(intent.getData());
                }
                break;
            case Intent.ACTION_SEND:
                Uri one = intent.getParcelableExtra(Intent.EXTRA_STREAM);
                if (one != null) {
                    uris.add(one);
                }
                break;
            case Intent.ACTION_SEND_MULTIPLE:
                ArrayList<Uri> many = intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM);
                if (many != null) {
                    uris.addAll(many);
                }
                break;
            default:
                return;
        }
        File base = getExternalFilesDir(null);
        if (base == null) {
            base = getFilesDir();
        }
        final File dir = new File(base, "taiko-rs/import");
        dir.mkdirs();
        for (final Uri uri : uris) {
            final String name = nameOf(uri);
            new Thread(new Runnable() {
                @Override
                public void run() {
                    copy(uri, dir, name);
                }
            }, "osz import").start();
        }
    }

    /** The archive's file name without its extension, safe as a folder name. */
    private String nameOf(Uri uri) {
        String name = null;
        try (Cursor c = getContentResolver().query(uri, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst() && !c.isNull(0)) {
                name = c.getString(0);
            }
        } catch (Exception ignored) {
            // Some providers don't answer queries; fall back to the URI.
        }
        if (name == null) {
            name = uri.getLastPathSegment();
        }
        if (name == null || name.isEmpty()) {
            name = "import " + System.currentTimeMillis();
        }
        name = name.replaceAll("[\\\\/:*?\"<>|\\p{Cntrl}]", "_");
        String lower = name.toLowerCase();
        if (lower.endsWith(".osz") || lower.endsWith(".zip")) {
            name = name.substring(0, name.length() - 4);
        }
        return name.trim().isEmpty() ? "import " + System.currentTimeMillis() : name.trim();
    }

    private void copy(Uri uri, File dir, String name) {
        File part = new File(dir, name + ".osz.part");
        try (InputStream in = getContentResolver().openInputStream(uri);
             OutputStream out = new FileOutputStream(part)) {
            if (in == null) {
                throw new java.io.IOException("can't open " + uri);
            }
            byte[] buf = new byte[1 << 16];
            for (int n; (n = in.read(buf)) > 0; ) {
                out.write(buf, 0, n);
            }
        } catch (Exception e) {
            android.util.Log.w("taiko-rs", "import of " + uri + " failed", e);
            part.delete();
            return;
        }
        File done = new File(dir, name + ".osz");
        for (int i = 2; done.exists(); i++) {
            done = new File(dir, name + " (" + i + ").osz");
        }
        if (!part.renameTo(done)) {
            part.delete();
        }
    }
}
