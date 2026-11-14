package io.github.lynerrr.taiko;

import android.app.NativeActivity;
import android.content.Intent;
import android.content.pm.ActivityInfo;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.OpenableColumns;
import android.text.InputType;
import android.view.DisplayCutout;
import android.view.KeyEvent;
import android.view.RoundedCorner;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import android.webkit.MimeTypeMap;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.util.ArrayList;

/**
 * The game is native code (libtaiko_rs.so, see src/android.rs). This activity provides:
 *
 * <ul>
 * <li>Importing beatmap archives opened with or shared to the game. Each one is copied into
 * {@code <files>/taiko-rs/import/<name>.osz.part} and renamed to {@code <name>.osz} once complete;
 * the game picks it up from there and unpacks it into its Songs folder.
 * <li>The on-screen keyboard. The game shows and hides it ({@link #keyboard}); what is typed on it
 * goes to the game through {@link #typed}, and whether it is up through {@link #keyboardShown}.
 * <li>The system file picker for the map editor.
 * </ul>
 */
public class TaikoActivity extends NativeActivity {
    private static final int PICK_FILE = 0x544B;
    private static final String PICK_CACHE_PREFIX = "taiko-pick-";
    private static boolean pickerCacheSwept;
    /** Talks to the input method; the game draws everything, so it has nothing to show. */
    private KeyboardView keyboardView;

    /** Text typed on the on-screen keyboard, or (with {@code text} null) a key pressed on it. */
    private native void typed(String text, int keyCode);

    /** The on-screen keyboard came up or went away (Android 11 and later tell). */
    private native void keyboardShown(boolean shown);

    /** A copied file, a failure message, or two nulls when the picker was cancelled. */
    private native void filePicked(String path, String error);

    /** Called by the game thread to open Android's document picker. */
    public void pickFile(boolean image) {
        runOnUiThread(() -> {
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType(image ? "image/*" : "audio/*");
            try {
                startActivityForResult(intent, PICK_FILE);
            } catch (RuntimeException e) {
                android.util.Log.w("taiko-rs", "opening the file picker failed", e);
                filePicked(null, "couldn't open the file picker");
            }
        });
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request != PICK_FILE) {
            return;
        }
        if (result != RESULT_OK) {
            filePicked(null, null);
            return;
        }
        Uri uri = data == null ? null : data.getData();
        if (uri == null) {
            filePicked(null, "the file picker didn't return a file");
            return;
        }
        new Thread(() -> copyPickedFile(uri), "editor file picker").start();
    }

    private void copyPickedFile(Uri uri) {
        File dir = null;
        File file = null;
        try {
            String name = documentName(uri);
            if (name.lastIndexOf('.') <= 0 || name.endsWith(".")) {
                String type = getContentResolver().getType(uri);
                String extension = type == null ? null
                    : MimeTypeMap.getSingleton().getExtensionFromMimeType(type);
                if (extension != null) {
                    name += (name.endsWith(".") ? "" : ".") + extension;
                }
            }
            dir = Files.createTempDirectory(getCacheDir().toPath(), PICK_CACHE_PREFIX).toFile();
            file = new File(dir, name);
            copyContents(uri, file);
        } catch (IOException | RuntimeException e) {
            android.util.Log.w("taiko-rs", "copying the chosen file failed", e);
            if (file != null) {
                file.delete();
            }
            if (dir != null) {
                dir.delete();
            }
            filePicked(null, "couldn't read the chosen file");
            return;
        }
        filePicked(file.getAbsolutePath(), null);
    }

    /** Remove picker copies left by a previous process, before starting the native game. */
    private void sweepPickerCache() {
        if (pickerCacheSwept) {
            return;
        }
        pickerCacheSwept = true;
        File cache = getCacheDir();
        if (cache == null) {
            return;
        }
        File[] dirs = cache.listFiles(file ->
            file.getName().startsWith(PICK_CACHE_PREFIX)
                && Files.isDirectory(file.toPath(), LinkOption.NOFOLLOW_LINKS));
        if (dirs == null) {
            return;
        }
        for (File dir : dirs) {
            File[] entries = dir.listFiles();
            if (entries != null) {
                for (File entry : entries) {
                    entry.delete();
                }
            }
            dir.delete();
        }
    }

    @Override
    protected void onCreate(Bundle saved) {
        sweepPickerCache();
        super.onCreate(saved);
        running = this;
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

    /**
     * How far in from the sides (high 16 bits) and from the top and bottom (low 16 bits) the game
     * keeps its interface, in pixels, to stay clear of the camera and the rounded corners; -1
     * until the window is up. The same on both sides, so turning the phone over moves nothing.
     */
    public int safeInsets() {
        WindowInsets insets = getWindow().getDecorView().getRootWindowInsets();
        if (insets == null) {
            return -1;
        }
        int side = 0;
        int edge = 0;
        if (Build.VERSION.SDK_INT >= 28 && insets.getDisplayCutout() != null) {
            DisplayCutout cutout = insets.getDisplayCutout();
            side = Math.max(cutout.getSafeInsetLeft(), cutout.getSafeInsetRight());
            edge = Math.max(cutout.getSafeInsetTop(), cutout.getSafeInsetBottom());
        }
        // Most of a corner's curve is cleared this far in from both of its edges. Before Android
        // 12 phones don't say how round they are: a little off every edge.
        int corner = Math.round(getResources().getDisplayMetrics().density * 10);
        if (Build.VERSION.SDK_INT >= 31) {
            int radius = 0;
            for (int at : new int[] {RoundedCorner.POSITION_TOP_LEFT, RoundedCorner.POSITION_TOP_RIGHT,
                    RoundedCorner.POSITION_BOTTOM_LEFT, RoundedCorner.POSITION_BOTTOM_RIGHT}) {
                RoundedCorner round = insets.getRoundedCorner(at);
                if (round != null) {
                    radius = Math.max(radius, round.getRadius());
                }
            }
            corner = Math.round(radius * 0.3f);
        }
        return Math.max(side, corner) << 16 | Math.max(edge, corner);
    }

    /** The activity, for the music's controls to reach the game with. */
    static volatile TaikoActivity running;

    /** A press of the media controls (`MusicService`), for the game. */
    private native void media(int what, long ms);

    void mediaPress(int what, long ms) {
        media(what, ms);
    }

    /** What's playing, for the media controls. Called by the game, from its own thread. */
    public void nowPlaying(String title, String artist, String detail, String art, long lengthMs,
            long atMs, float speed, boolean paused) {
        MusicService.Song song = new MusicService.Song();
        song.title = title;
        song.artist = artist;
        song.detail = detail;
        song.art = art;
        song.lengthMs = lengthMs;
        song.atMs = atMs;
        song.speed = speed;
        song.paused = paused;
        MusicService.show(song);
    }

    /** Lets the screen turn upright with the phone (the music player), or keeps it on its side. */
    public void allowPortrait(final boolean on) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                setRequestedOrientation(on ? ActivityInfo.SCREEN_ORIENTATION_FULL_USER
                        : ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE);
            }
        });
    }

    /** Whether leaving the app keeps the music alive. Set by the game, from its own thread. */
    private volatile boolean keepMusic;

    /** The game's music can carry on outside the app, or can't any more. */
    public void keepMusic(boolean on) {
        keepMusic = on;
        if (!on) {
            stopService(new Intent(this, MusicService.class));
        }
    }

    // While the app can still start one: Android refuses from the background.
    @Override
    protected void onPause() {
        super.onPause();
        if (keepMusic) {
            try {
                startForegroundService(new Intent(this, MusicService.class));
            } catch (RuntimeException e) {
                // The music just isn't kept alive with the screen off.
            }
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        stopService(new Intent(this, MusicService.class));
    }

    /**
     * The game closed (its Exit, or swiped out of the recent apps): the music stops with it, and
     * so does the process, before NativeActivity's own onDestroy, which waits for the game's loop
     * to end and never sees it (the system gave up after a timeout, leaving a stuck process the
     * next launch reused: the game never came up). Settings are saved as they change.
     */
    @Override
    protected void onDestroy() {
        stopService(new Intent(this, MusicService.class));
        running = null;
        if (isFinishing()) {
            android.os.Process.killProcess(android.os.Process.myPid());
        }
        super.onDestroy();
    }

    /** Full screen, over the camera's cutout too: the game keeps clear of it ({@link #safeInsets}). */
    private void goFullScreen() {
        if (Build.VERSION.SDK_INT >= 28) {
            WindowManager.LayoutParams params = getWindow().getAttributes();
            params.layoutInDisplayCutoutMode =
                    WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
            getWindow().setAttributes(params);
        }
        if (Build.VERSION.SDK_INT >= 30) {
            WindowInsetsController bars = getWindow().getInsetsController();
            if (bars != null) {
                bars.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
                bars.hide(WindowInsets.Type.systemBars());
            }
        }
    }

    @Override
    public void onWindowFocusChanged(boolean focused) {
        super.onWindowFocusChanged(focused);
        if (focused) {
            goFullScreen();
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

    private String documentName(Uri uri) {
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
        name = name.replaceAll("[\\\\/:*?\"<>|\\p{Cntrl}]", "_").trim();
        return name.isEmpty() || name.equals(".") || name.equals("..")
            ? "import " + System.currentTimeMillis() : name;
    }

    /** The archive's file name without its extension, safe as a folder name. */
    private String nameOf(Uri uri) {
        String name = documentName(uri);
        String lower = name.toLowerCase();
        // taiko.rs's own .rsz and osu!lazer's .olz are the same zip as an .osz.
        if (lower.endsWith(".osz") || lower.endsWith(".olz") || lower.endsWith(".rsz") || lower.endsWith(".zip")) {
            name = name.substring(0, name.length() - 4);
        }
        return name.trim().isEmpty() ? "import " + System.currentTimeMillis() : name.trim();
    }

    private void copyContents(Uri uri, File file) throws IOException {
        try (InputStream in = getContentResolver().openInputStream(uri);
             OutputStream out = new FileOutputStream(file)) {
            if (in == null) {
                throw new IOException("can't open " + uri);
            }
            byte[] buf = new byte[1 << 16];
            for (int n; (n = in.read(buf)) > 0; ) {
                out.write(buf, 0, n);
            }
        }
    }

    private void copy(Uri uri, File dir, String name) {
        File part = new File(dir, name + ".osz.part");
        try {
            copyContents(uri, part);
        } catch (IOException | RuntimeException e) {
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
