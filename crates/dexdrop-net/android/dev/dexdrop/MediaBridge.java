package dev.dexdrop;

import android.content.ContentResolver;
import android.content.ContentUris;
import android.content.Context;
import android.database.Cursor;
import android.graphics.Bitmap;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.MediaStore;
import android.util.Size;
import java.io.File;
import java.io.FileOutputStream;

/**
 * DexDrop's bridge to Android's {@link MediaStore}: the photos and videos
 * most recently added to the device, shown under Send's choices to pick
 * from in one tap.
 *
 * <p>Made by the Rust adapter (crates/dexdrop-net/src/media.rs),
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
 * dexdrop-net's Cargo.toml ([package.metadata.nsh.android]) declares them
 * and the app asks for them with NSH's {@code nsh::permissions}.
 *
 * <p>Build with scripts/build-nsd-dex.sh.
 */
public final class MediaBridge {
    private static final char SEP = '\u0001';
    private static final int THUMB_PX = 256;

    private final Context context;

    /** {@code context}: the application's, which NSH gives the Rust side. */
    public MediaBridge(Context context) {
        this.context = context;
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
