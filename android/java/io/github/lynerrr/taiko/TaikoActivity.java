package io.github.lynerrr.taiko;

import android.app.NativeActivity;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.OpenableColumns;
import android.text.InputType;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.WindowManager;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;

import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;

/**
 * The game is native code (libtaiko_rs.so, see src/android.rs); this adds two things to it:
 *
 * <ul>
 * <li>Importing beatmap archives opened with or shared to the game. Each one is copied into
 * {@code <files>/taiko-rs/import/<name>.osz.part} and renamed to {@code <name>.osz} once complete;
 * the game picks it up from there and unpacks it into its Songs folder.
 * <li>The on-screen keyboard. The game shows and hides it ({@link #keyboard}); what is typed on it
 * goes to the game through {@link #typed}, and whether it is up through {@link #keyboardShown}.
 * </ul>
 */
public class TaikoActivity extends NativeActivity {
    /** Talks to the input method; the game draws everything, so it has nothing to show. */
    private KeyboardView keyboardView;

    /** Text typed on the on-screen keyboard, or (with {@code text} null) a key pressed on it. */
    private native void typed(String text, int keyCode);

    /** The on-screen keyboard came up or went away (Android 11 and later tell). */
    private native void keyboardShown(boolean shown);

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        // NativeActivity asks for resizing; the keyboard goes over the game instead (the game
        // shows what is being typed above it), and only when the game asks for it.
        getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_HIDDEN
                | WindowManager.LayoutParams.SOFT_INPUT_ADJUST_NOTHING);
        keyboardView = new KeyboardView();
        addContentView(keyboardView, new ViewGroup.LayoutParams(1, 1));
        // A recreated activity (saved != null) already imported its intent.
        if (saved == null) {
            importFrom(getIntent());
        }
    }

    /** Shows or hides the on-screen keyboard. Called by the game, from its own thread. */
    public void keyboard(final boolean show) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                final InputMethodManager imm = getSystemService(InputMethodManager.class);
                if (show) {
                    keyboardView.setFocusableInTouchMode(true);
                    keyboardView.requestFocus();
                    // Once the input method has taken the focus change in: a fresh start (nothing
                    // half-typed left from last time), then the keyboard.
                    keyboardView.post(new Runnable() {
                        @Override
                        public void run() {
                            imm.restartInput(keyboardView);
                            imm.showSoftInput(keyboardView, 0);
                        }
                    });
                } else {
                    imm.hideSoftInputFromWindow(keyboardView.getWindowToken(), 0);
                    // Not a text field any more: no keyboard coming back by itself.
                    keyboardView.setFocusableInTouchMode(false);
                    keyboardView.clearFocus();
                }
            }
        });
    }

    /**
     * A view that is only there to be the input method's text field. While the keyboard is up it
     * has the focus, and its connection turns what is typed into the game's typing: text, and
     * Backspace for each character to take back.
     */
    private final class KeyboardView extends View {
        private boolean shown;

        KeyboardView() {
            super(TaikoActivity.this);
            setFocusable(true);
            if (Build.VERSION.SDK_INT >= 30) {
                setOnApplyWindowInsetsListener(new View.OnApplyWindowInsetsListener() {
                    @Override
                    public WindowInsets onApplyWindowInsets(View v, WindowInsets insets) {
                        boolean up = insets.isVisible(WindowInsets.Type.ime());
                        if (up != shown) {
                            shown = up;
                            keyboardShown(up);
                        }
                        return insets;
                    }
                });
            }
        }

        @Override
        public boolean onCheckIsTextEditor() {
            return true;
        }

        @Override
        public InputConnection onCreateInputConnection(EditorInfo info) {
            info.inputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
            // Landscape keyboards otherwise take the whole screen with a text box of their own.
            info.imeOptions = EditorInfo.IME_ACTION_DONE | EditorInfo.IME_FLAG_NO_FULLSCREEN | EditorInfo.IME_FLAG_NO_EXTRACT_UI;
            return new Connection(this);
        }
    }

    /**
     * The game's text fields only add at the end and take the last character back, so this keeps
     * nothing but what the keyboard is still composing (a word being typed or converted), which
     * is already in the game: when it changes, what differs is taken back and typed anew.
     */
    private final class Connection extends BaseInputConnection {
        private String composing = "";

        Connection(View view) {
            super(view, false);
        }

        @Override
        public boolean setComposingText(CharSequence text, int newCursorPosition) {
            replaceComposing(text.toString());
            return true;
        }

        @Override
        public boolean commitText(CharSequence text, int newCursorPosition) {
            replaceComposing(text.toString());
            composing = "";
            return true;
        }

        @Override
        public boolean finishComposingText() {
            composing = "";
            return true;
        }

        @Override
        public boolean deleteSurroundingText(int beforeLength, int afterLength) {
            // Lengths in UTF-16 units; the game takes back whole characters.
            if (!composing.isEmpty()) {
                int keep = Math.max(0, composing.length() - beforeLength);
                if (keep > 0 && keep < composing.length() && Character.isLowSurrogate(composing.charAt(keep))) {
                    keep--;
                }
                replaceComposing(composing.substring(0, keep));
            } else {
                for (int i = 0; i < beforeLength; i++) {
                    typed(null, KeyEvent.KEYCODE_DEL);
                }
            }
            return true;
        }

        @Override
        public boolean deleteSurroundingTextInCodePoints(int beforeLength, int afterLength) {
            return deleteSurroundingText(beforeLength, afterLength);
        }

        @Override
        public boolean sendKeyEvent(KeyEvent event) {
            if (event.getAction() == KeyEvent.ACTION_MULTIPLE && event.getKeyCode() == KeyEvent.KEYCODE_UNKNOWN) {
                typed(event.getCharacters(), 0);
                return true;
            }
            if (event.getAction() != KeyEvent.ACTION_DOWN) {
                return true;
            }
            int ch = event.getUnicodeChar();
            if (ch > 0 && !Character.isISOControl(ch)) {
                composing = "";
                typed(new String(Character.toChars(ch)), 0);
                return true;
            }
            int code = event.getKeyCode();
            if (code == KeyEvent.KEYCODE_DEL && !composing.isEmpty()) {
                composing = composing.substring(0, composing.offsetByCodePoints(composing.length(), -1));
            } else {
                composing = "";
            }
            typed(null, code);
            return true;
        }

        @Override
        public boolean performEditorAction(int action) {
            composing = "";
            typed(null, KeyEvent.KEYCODE_ENTER);
            return true;
        }

        /** Makes the composing text in the game {@code text}. */
        private void replaceComposing(String text) {
            int same = 0;
            while (same < composing.length() && same < text.length() && composing.codePointAt(same) == text.codePointAt(same)) {
                same += Character.charCount(composing.codePointAt(same));
            }
            for (int i = composing.codePointCount(same, composing.length()); i > 0; i--) {
                typed(null, KeyEvent.KEYCODE_DEL);
            }
            if (same < text.length()) {
                typed(text.substring(same), 0);
            }
            composing = text;
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
