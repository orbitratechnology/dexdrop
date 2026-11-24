package dev.dexdrop;

import android.app.Activity;
import android.content.ContentResolver;
import android.content.ContentUris;
import android.content.Context;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.graphics.Bitmap;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.MediaStore;
import android.util.Size;
import java.io.File;
import java.io.FileOutputStream;
import java.lang.reflect.Field;
import java.util.Map;

/**
 * DexDrop's bridge to Android's {@link MediaStore}: the photos and videos
 * most recently added to the device, shown under Send's choices to pick
 * from in one tap, and the permission to read them.
 *
 * <p>Loaded from memory by the Rust adapter (crates/dexdrop-net/src/media.rs),
 * which calls these methods from a worker thread. Each item is one line,
 * fields separated by U+0001:
 * {@code path kind size added duration thumbnail}, where {@code kind} is
 * {@code photo} or {@code video}, {@code added} is in seconds since 1970,
 * {@code duration} in milliseconds (0 for photos) and {@code thumbnail} is
 * a small JPEG written under the given folder (empty when none could be
 * made). Items are sent from their own files: nothing is copied.
 *
 * <p>Reading them needs {@code READ_MEDIA_IMAGES} and
 * {@code READ_MEDIA_VIDEO} (Android 13+; with
 * {@code READ_MEDIA_VISUAL_USER_SELECTED} on Android 14+ the user may allow
 * only some), or {@code READ_EXTERNAL_STORAGE} before;
 * scripts/patch-android-manifest.sh declares them in the packaged APK.
 *
 * <p>Build with scripts/build-nsd-dex.sh.
 */
public final class MediaBridge {
    private static final char SEP = '\u0001';
    private static final int THUMB_PX = 256;

    private final Context context;

    public MediaBridge() throws Exception {
        context = (Context) Class.forName("android.app.ActivityThread")
                .getMethod("currentApplication")
                .invoke(null);
        if (context == null) {
            throw new IllegalStateException("no application context");
        }
    }

    private static String[] permissions() {
        if (Build.VERSION.SDK_INT >= 34) {
            return new String[] {
                "android.permission.READ_MEDIA_IMAGES",
                "android.permission.READ_MEDIA_VIDEO",
                "android.permission.READ_MEDIA_VISUAL_USER_SELECTED",
            };
        }
        if (Build.VERSION.SDK_INT >= 33) {
            return new String[] {
                "android.permission.READ_MEDIA_IMAGES", "android.permission.READ_MEDIA_VIDEO",
            };
        }
        return new String[] {"android.permission.READ_EXTERNAL_STORAGE"};
    }

    private boolean granted(String permission) {
        return context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED;
    }

    /**
     * How much the app may read: {@code all}, {@code some} (Android 14's
     * "allow limited access") or {@code none}.
     */
    public String access() {
        if (Build.VERSION.SDK_INT >= 33) {
            if (granted("android.permission.READ_MEDIA_IMAGES")
                    || granted("android.permission.READ_MEDIA_VIDEO")) {
                return "all";
            }
            if (Build.VERSION.SDK_INT >= 34
                    && granted("android.permission.READ_MEDIA_VISUAL_USER_SELECTED")) {
                return "some";
            }
            return "none";
        }
        return granted("android.permission.READ_EXTERNAL_STORAGE") ? "all" : "none";
    }

    /**
     * Asks the user, with the system's dialog, from the activity in front;
     * {@code asked} when the dialog was requested, else why not. The answer
     * is read with {@link #access()} once the app is back.
     */
    public String ask() {
        final Activity activity = foreground();
        if (activity == null) {
            return "no activity in front";
        }
        final String[] wanted = permissions();
        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                activity.requestPermissions(wanted, 0xDD);
            }
        });
        return "asked";
    }

    /** The resumed activity, found through ActivityThread's records. */
    private static Activity foreground() {
        try {
            Class<?> threadClass = Class.forName("android.app.ActivityThread");
            Object thread = threadClass.getMethod("currentActivityThread").invoke(null);
            Field activities = threadClass.getDeclaredField("mActivities");
            activities.setAccessible(true);
            Object records = activities.get(thread);
            if (!(records instanceof Map)) {
                return null;
            }
            Activity any = null;
            for (Object record : ((Map<?, ?>) records).values()) {
                Class<?> recordClass = record.getClass();
                Field activityField = recordClass.getDeclaredField("activity");
                activityField.setAccessible(true);
                Activity activity = (Activity) activityField.get(record);
                if (activity == null) {
                    continue;
                }
                any = activity;
                Field paused = recordClass.getDeclaredField("paused");
                paused.setAccessible(true);
                if (!paused.getBoolean(record)) {
                    return activity;
                }
            }
            return any;
        } catch (Exception e) {
            return null;
        }
    }

    /**
     * The {@code limit} photos and videos added last, newest first, with
     * thumbnails written into {@code thumbDir}; empty without access.
     */
    public String recent(String limit, String thumbDir) {
        int max = Integer.parseInt(limit);
        Uri uri = MediaStore.Files.getContentUri("external");
        String[] projection = {
            MediaStore.Files.FileColumns._ID,
            MediaStore.Files.FileColumns.DATA,
            MediaStore.Files.FileColumns.MEDIA_TYPE,
            MediaStore.Files.FileColumns.SIZE,
            MediaStore.Files.FileColumns.DATE_ADDED,
            "duration",
        };
        String where = MediaStore.Files.FileColumns.MEDIA_TYPE + " IN ("
                + MediaStore.Files.FileColumns.MEDIA_TYPE_IMAGE + ","
                + MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO + ")";
        Bundle args = new Bundle();
        args.putString(ContentResolver.QUERY_ARG_SQL_SELECTION, where);
        args.putString(ContentResolver.QUERY_ARG_SQL_SORT_ORDER,
                MediaStore.Files.FileColumns.DATE_ADDED + " DESC");
        args.putInt(ContentResolver.QUERY_ARG_LIMIT, max);
        StringBuilder out = new StringBuilder();
        File dir = new File(thumbDir);
        dir.mkdirs();
        ContentResolver resolver = context.getContentResolver();
        try (Cursor c = resolver.query(uri, projection, args, null)) {
            if (c == null) {
                return "";
            }
            int seen = 0;
            while (c.moveToNext() && seen < max) {
                long id = c.getLong(0);
                String path = c.getString(1);
                if (path == null || !new File(path).canRead()) {
                    continue;
                }
                boolean video = c.getInt(2) == MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO;
                long size = c.getLong(3);
                long added = c.getLong(4);
                long duration = video && !c.isNull(5) ? c.getLong(5) : 0;
                Uri item = ContentUris.withAppendedId(video
                        ? MediaStore.Video.Media.EXTERNAL_CONTENT_URI
                        : MediaStore.Images.Media.EXTERNAL_CONTENT_URI, id);
                String thumb = thumbnail(resolver, item, new File(dir, id + ".jpg"));
                out.append(path).append(SEP)
                        .append(video ? "video" : "photo").append(SEP)
                        .append(size).append(SEP)
                        .append(added).append(SEP)
                        .append(duration).append(SEP)
                        .append(thumb).append('\n');
                seen++;
            }
        } catch (Exception e) {
            // No access, or the provider failed: nothing to show.
        }
        return out.toString();
    }

    /** A small JPEG of the item, made once; its path, or empty. */
    private static String thumbnail(ContentResolver resolver, Uri item, File to) {
        if (to.length() > 0) {
            return to.getPath();
        }
        if (Build.VERSION.SDK_INT < 29) {
            return "";
        }
        try {
            Bitmap bitmap = resolver.loadThumbnail(item, new Size(THUMB_PX, THUMB_PX), null);
            try (FileOutputStream stream = new FileOutputStream(to)) {
                bitmap.compress(Bitmap.CompressFormat.JPEG, 80, stream);
            }
            bitmap.recycle();
            return to.getPath();
        } catch (Exception e) {
            to.delete();
            return "";
        }
    }
}
