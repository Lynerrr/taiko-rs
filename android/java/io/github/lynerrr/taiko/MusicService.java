package io.github.lynerrr.taiko;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.drawable.Icon;
import android.media.MediaMetadata;
import android.media.session.MediaSession;
import android.media.session.PlaybackState;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.PowerManager;

/**
 * The game's music outside the app. Without a foreground service Android freezes the process once
 * the screen is off, so this holds it, and shows the song as the system's media player: the
 * notification, the quick panel and the lock screen, with the map's background as its artwork. The
 * music itself stays in the game (src/android.rs): this only shows it and passes presses back.
 */
public class MusicService extends Service {
    /** What the game says is playing. */
    static final class Song {
        String title = "taiko.rs";
        String artist = "";
        String detail = "";
        String art = "";
        long lengthMs;
        long atMs;
        float speed = 1f;
        boolean paused;
    }

    /** What presses of the controls are, as the game takes them. */
    static final int TOGGLE = 0;
    static final int NEXT = 1;
    static final int PREV = 2;
    static final int SEEK = 3;

    private static final String CHANNEL = "music";
    private static final int NOTIFICATION = 1;
    /** The drum's red: the notification's colour where the artwork doesn't set one. */
    private static final int COLOUR = 0xFFEB452C;
    private static final int ART_SIZE = 1024;

    private static volatile Song song = new Song();
    private static volatile MusicService running;

    private final Handler main = new Handler(Looper.getMainLooper());
    private MediaSession session;
    private PowerManager.WakeLock wakeLock;
    private String artPath;
    private Bitmap art;

    /** The game has a new song, or its place in it changed. Any thread. */
    static void show(Song now) {
        song = now;
        final MusicService service = running;
        if (service != null) {
            service.main.post(new Runnable() {
                @Override
                public void run() {
                    service.update();
                }
            });
        }
    }

    @Override
    public void onCreate() {
        super.onCreate();
        running = this;
        getSystemService(NotificationManager.class).createNotificationChannel(
                new NotificationChannel(CHANNEL, "Music", NotificationManager.IMPORTANCE_LOW));
        session = new MediaSession(this, "taiko.rs");
        session.setCallback(new MediaSession.Callback() {
            @Override
            public void onPlay() {
                press(TOGGLE, 0);
                shown(song.atMs, false);
            }

            @Override
            public void onPause() {
                press(TOGGLE, 0);
                shown(song.atMs, true);
            }

            @Override
            public void onSkipToNext() {
                press(NEXT, 0);
            }

            @Override
            public void onSkipToPrevious() {
                press(PREV, 0);
            }

            @Override
            public void onSeekTo(long ms) {
                press(SEEK, ms);
                shown(ms, song.paused);
            }
        });
        session.setActive(true);
    }

    /**
     * Shows the press done at once: the game takes it on its next frame and says so after, and
     * meanwhile the bar would jump back to where it was.
     */
    private void shown(long atMs, boolean paused) {
        Song was = song;
        Song now = new Song();
        now.title = was.title;
        now.artist = was.artist;
        now.detail = was.detail;
        now.art = was.art;
        now.lengthMs = was.lengthMs;
        now.speed = was.speed;
        now.atMs = atMs;
        now.paused = paused;
        song = now;
        update();
    }

    private static void press(int what, long ms) {
        TaikoActivity game = TaikoActivity.running;
        if (game != null) {
            game.mediaPress(what, ms);
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        Notification notification = build();
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(NOTIFICATION, notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK);
        } else {
            startForeground(NOTIFICATION, notification);
        }
        String action = intent == null ? null : intent.getAction();
        if ("toggle".equals(action)) {
            press(TOGGLE, 0);
        } else if ("next".equals(action)) {
            press(NEXT, 0);
        } else if ("prev".equals(action)) {
            press(PREV, 0);
        }
        update();
        return START_NOT_STICKY;
    }

    /** Shows the song everywhere it's shown. */
    private void update() {
        if (session == null) {
            return;
        }
        Song now = song;
        loadArt(now.art);
        MediaMetadata.Builder meta = new MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, now.title)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, now.artist)
                .putString(MediaMetadata.METADATA_KEY_ALBUM, now.detail)
                .putLong(MediaMetadata.METADATA_KEY_DURATION, now.lengthMs);
        if (art != null) {
            meta.putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, art);
        }
        session.setMetadata(meta.build());
        session.setPlaybackState(new PlaybackState.Builder()
                .setActions(PlaybackState.ACTION_PLAY | PlaybackState.ACTION_PAUSE
                        | PlaybackState.ACTION_PLAY_PAUSE | PlaybackState.ACTION_SKIP_TO_NEXT
                        | PlaybackState.ACTION_SKIP_TO_PREVIOUS | PlaybackState.ACTION_SEEK_TO)
                .setState(now.paused ? PlaybackState.STATE_PAUSED : PlaybackState.STATE_PLAYING,
                        now.atMs, now.paused ? 0f : now.speed)
                .build());
        getSystemService(NotificationManager.class).notify(NOTIFICATION, build());
        hold(!now.paused);
    }

    /** The CPU stays up only while there's music to keep going. */
    private void hold(boolean on) {
        if (on && wakeLock == null) {
            PowerManager power = getSystemService(PowerManager.class);
            wakeLock = power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "taiko:music");
            wakeLock.acquire();
        } else if (!on && wakeLock != null) {
            wakeLock.release();
            wakeLock = null;
        }
    }

    /** The map's background, scaled down: a whole image would be too big to hand to the system. */
    private void loadArt(String path) {
        if (path.equals(artPath)) {
            return;
        }
        artPath = path;
        art = null;
        if (path.isEmpty()) {
            return;
        }
        BitmapFactory.Options size = new BitmapFactory.Options();
        size.inJustDecodeBounds = true;
        BitmapFactory.decodeFile(path, size);
        BitmapFactory.Options load = new BitmapFactory.Options();
        load.inSampleSize = 1;
        while (Math.max(size.outWidth, size.outHeight) / (load.inSampleSize * 2) >= ART_SIZE) {
            load.inSampleSize *= 2;
        }
        art = BitmapFactory.decodeFile(path, load);
    }

    private Notification build() {
        Song now = song;
        Intent open = new Intent(this, TaikoActivity.class);
        PendingIntent back = PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE);
        Notification.Builder notification = new Notification.Builder(this, CHANNEL)
                .setSmallIcon(drawable("ic_taiko"))
                .setContentTitle(now.title)
                .setContentText(now.artist)
                .setSubText(now.detail)
                .setContentIntent(back)
                .setColor(COLOUR)
                .setColorized(true)
                .setVisibility(Notification.VISIBILITY_PUBLIC)
                .setShowWhen(false)
                .setOngoing(!now.paused)
                .addAction(action("ic_prev", "Previous", "prev", 1))
                .addAction(now.paused ? action("ic_play", "Play", "toggle", 2)
                        : action("ic_pause", "Pause", "toggle", 2))
                .addAction(action("ic_next", "Next", "next", 3))
                .setStyle(new Notification.MediaStyle()
                        .setMediaSession(session.getSessionToken())
                        .setShowActionsInCompactView(0, 1, 2));
        if (art != null) {
            notification.setLargeIcon(art);
        }
        return notification.build();
    }

    private Notification.Action action(String icon, String title, String what, int code) {
        Intent intent = new Intent(this, MusicService.class).setAction(what);
        PendingIntent press = PendingIntent.getService(this, code, intent,
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        return new Notification.Action.Builder(
                Icon.createWithResource(this, drawable(icon)), title, press).build();
    }

    /** A drawable by name: the APK is built without an R class. */
    private int drawable(String name) {
        // Resources keep the source package, whatever the app id (a dev build renames it).
        return getResources().getIdentifier(name, "drawable", MusicService.class.getPackage().getName());
    }

    /** Swiped out of the recent apps: the game is going, and its music with it. */
    @Override
    public void onTaskRemoved(Intent root) {
        stopSelf();
    }

    @Override
    public void onDestroy() {
        running = null;
        hold(false);
        if (session != null) {
            session.setActive(false);
            session.release();
            session = null;
        }
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
