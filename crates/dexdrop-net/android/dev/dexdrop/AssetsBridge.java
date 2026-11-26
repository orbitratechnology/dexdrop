package dev.dexdrop;

import android.content.ContentResolver;
import android.content.ContentUris;
import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.graphics.Bitmap;
import android.graphics.Point;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.provider.MediaStore;
import android.provider.OpenableColumns;
import android.util.Size;
import java.io.File;
import java.io.FileOutputStream;
import java.util.List;

/**
 * DexDrop's bridge to the device's media and documents, for the Rust asset
 * providers (crates/dexdrop-net/src/assets_android.rs): items are listed and
 * described from {@link MediaStore} rows and document metadata, never by
 * reading them, and opened as file descriptors the Rust side reads in place
 * when a transfer starts. Nothing is copied.
 *
 * <p>Lists are lines, fields separated by U+0001:
 * {@code token kind name mime size added modified duration width height album}
 * ({@code kind}: image, video, audio or doc; times in Unix seconds; empty
 * fields unknown). A media token is {@code kind:id}; a document's is its
 * {@code content://} URI.
 *
 * <p>Media needs {@code READ_MEDIA_IMAGES}/{@code VIDEO}/{@code AUDIO}
 * (Android 13+, with {@code READ_MEDIA_VISUAL_USER_SELECTED} on 14+ for a
 * partial grant) or {@code READ_EXTERNAL_STORAGE} before; documents need no
 * permission (the user picks them; the grant is kept with
 * {@code takePersistableUriPermission}). Build with scripts/build-nsd-dex.sh.
 */
public final class AssetsBridge {
    private static final char SEP = '\u0001';

    private final Context context;

    public AssetsBridge(Context context) {
        this.context = context;
    }

    private static Uri collection(String kind) {
        switch (kind) {
            case "video":
                return MediaStore.Video.Media.EXTERNAL_CONTENT_URI;
            case "audio":
                return MediaStore.Audio.Media.EXTERNAL_CONTENT_URI;
            default:
                return MediaStore.Images.Media.EXTERNAL_CONTENT_URI;
        }
    }

    private static Uri item(String token) {
        int colon = token.indexOf(':');
        String kind = token.substring(0, colon);
        long id = Long.parseLong(token.substring(colon + 1));
        return ContentUris.withAppendedId(collection(kind), id);
    }

    private static final String[] COLUMNS = {
        MediaStore.MediaColumns._ID,
        MediaStore.MediaColumns.DISPLAY_NAME,
        MediaStore.MediaColumns.MIME_TYPE,
        MediaStore.MediaColumns.SIZE,
        MediaStore.MediaColumns.DATE_ADDED,
        MediaStore.MediaColumns.DATE_MODIFIED,
        "duration",
        MediaStore.MediaColumns.WIDTH,
        MediaStore.MediaColumns.HEIGHT,
        "bucket_display_name",
    };

    /** {@link #COLUMNS}, then the row's media type (lists span kinds). */
    private static final String[] LIST_COLUMNS = {
        MediaStore.MediaColumns._ID,
        MediaStore.MediaColumns.DISPLAY_NAME,
        MediaStore.MediaColumns.MIME_TYPE,
        MediaStore.MediaColumns.SIZE,
        MediaStore.MediaColumns.DATE_ADDED,
        MediaStore.MediaColumns.DATE_MODIFIED,
        "duration",
        MediaStore.MediaColumns.WIDTH,
        MediaStore.MediaColumns.HEIGHT,
        "bucket_display_name",
        MediaStore.Files.FileColumns.MEDIA_TYPE,
    };

    private static final Uri FILES = MediaStore.Files.getContentUri("external");

    /** {@code media_type IN (…)} for kinds {@code image,video,audio}. */
    private static String typesIn(String kinds) {
        StringBuilder in = new StringBuilder(MediaStore.Files.FileColumns.MEDIA_TYPE + " IN (");
        boolean first = true;
        for (String kind : kinds.split(",")) {
            int type;
            switch (kind) {
                case "video":
                    type = MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO;
                    break;
                case "audio":
                    type = MediaStore.Files.FileColumns.MEDIA_TYPE_AUDIO;
                    break;
                case "image":
                    type = MediaStore.Files.FileColumns.MEDIA_TYPE_IMAGE;
                    break;
                default:
                    continue;
            }
            in.append(first ? "" : ",").append(type);
            first = false;
        }
        return in.append(first ? "-1)" : ")").toString();
    }

    private static String kindOf(int mediaType) {
        switch (mediaType) {
            case MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO:
                return "video";
            case MediaStore.Files.FileColumns.MEDIA_TYPE_AUDIO:
                return "audio";
            default:
                return "image";
        }
    }

    /** The albums holding {@code kinds}, as lines {@code name count}. */
    public String albums(String kinds) {
        java.util.HashMap<String, Integer> counts = new java.util.HashMap<>();
        try (Cursor c = context.getContentResolver().query(FILES,
                new String[] {"bucket_display_name"}, typesIn(kinds), null, null)) {
            if (c != null) {
                while (c.moveToNext()) {
                    String name = c.isNull(0) ? "" : c.getString(0);
                    Integer n = counts.get(name);
                    counts.put(name, n == null ? 1 : n + 1);
                }
            }
        } catch (Exception e) {
            return "";
        }
        StringBuilder out = new StringBuilder();
        for (java.util.Map.Entry<String, Integer> e : counts.entrySet()) {
            if (!e.getKey().isEmpty()) {
                out.append(e.getKey()).append(SEP).append(e.getValue()).append('\n');
            }
        }
        return out.toString();
    }

    /**
     * One page of {@code kinds} (comma-separated: image, video, audio),
     * sorted ({@code newest}, {@code oldest}, {@code largest}, {@code name}),
     * names containing {@code search} (empty: all), at least {@code minSize}
     * bytes, in {@code album} (empty: all).
     */
    public String list(String kinds, String offset, String limit, String search, String sort,
            String minSize, String album) {
        int from = Integer.parseInt(offset);
        int max = Integer.parseInt(limit);
        StringBuilder where = new StringBuilder(typesIn(kinds))
                .append(" AND ").append(MediaStore.MediaColumns.SIZE).append(" >= ?");
        java.util.ArrayList<String> args = new java.util.ArrayList<>();
        args.add(minSize.isEmpty() ? "0" : minSize);
        if (!search.isEmpty()) {
            where.append(" AND ").append(MediaStore.MediaColumns.DISPLAY_NAME).append(" LIKE ?");
            args.add("%" + search + "%");
        }
        if (!album.isEmpty()) {
            where.append(" AND bucket_display_name = ?");
            args.add(album);
        }
        String order;
        switch (sort) {
            case "oldest":
                order = MediaStore.MediaColumns.DATE_ADDED + " ASC";
                break;
            case "largest":
                order = MediaStore.MediaColumns.SIZE + " DESC";
                break;
            case "name":
                order = MediaStore.MediaColumns.DISPLAY_NAME + " COLLATE NOCASE ASC";
                break;
            default:
                order = MediaStore.MediaColumns.DATE_ADDED + " DESC";
        }
        ContentResolver resolver = context.getContentResolver();
        String[] selection = args.toArray(new String[0]);
        Cursor c;
        if (Build.VERSION.SDK_INT >= 30) {
            Bundle q = new Bundle();
            q.putString(ContentResolver.QUERY_ARG_SQL_SELECTION, where.toString());
            q.putStringArray(ContentResolver.QUERY_ARG_SQL_SELECTION_ARGS, selection);
            q.putString(ContentResolver.QUERY_ARG_SQL_SORT_ORDER, order);
            q.putInt(ContentResolver.QUERY_ARG_LIMIT, max);
            q.putInt(ContentResolver.QUERY_ARG_OFFSET, from);
            c = resolver.query(FILES, LIST_COLUMNS, q, null);
        } else {
            c = resolver.query(FILES, LIST_COLUMNS, where.toString(), selection,
                    order + " LIMIT " + max + " OFFSET " + from);
        }
        StringBuilder out = new StringBuilder();
        if (c == null) {
            return "";
        }
        try {
            while (c.moveToNext()) {
                row(out, kindOf(c.getInt(10)), c);
            }
        } finally {
            c.close();
        }
        return out.toString();
    }

    private static String text(Cursor c, int i) {
        return c.isNull(i) ? "" : c.getString(i);
    }

    private static void row(StringBuilder out, String kind, Cursor c) {
        out.append(kind).append(':').append(c.getLong(0)).append(SEP)
                .append(kind).append(SEP)
                .append(text(c, 1)).append(SEP)
                .append(text(c, 2)).append(SEP)
                .append(text(c, 3)).append(SEP)
                .append(text(c, 4)).append(SEP)
                .append(text(c, 5)).append(SEP)
                .append(text(c, 6)).append(SEP)
                .append(text(c, 7)).append(SEP)
                .append(text(c, 8)).append(SEP)
                .append(text(c, 9)).append('\n');
    }

    /** The row of one media item, or empty when it is gone. */
    public String meta(String token) {
        String kind = token.substring(0, token.indexOf(':'));
        try (Cursor c = context.getContentResolver().query(item(token), COLUMNS, null, null,
                null)) {
            if (c == null || !c.moveToFirst()) {
                return "";
            }
            StringBuilder out = new StringBuilder();
            row(out, kind, c);
            return out.toString();
        } catch (Exception e) {
            return "";
        }
    }

    /**
     * A file descriptor of the item's original, owned by the caller, or -1:
     * the media file or document read where it is.
     */
    public String open(String token) {
        Uri uri = token.startsWith("content://") ? Uri.parse(token) : item(token);
        try {
            ParcelFileDescriptor pfd = context.getContentResolver().openFileDescriptor(uri, "r");
            return pfd == null ? "-1" : String.valueOf(pfd.detachFd());
        } catch (Exception e) {
            return "-1";
        }
    }

    /**
     * A JPEG thumbnail of about {@code px} pixels written to {@code to}
     * (made once), or empty: the system's thumbnail, never the full image
     * decoded.
     */
    public String thumbnail(String token, String px, String to) {
        File file = new File(to);
        if (file.length() > 0) {
            return to;
        }
        file.getParentFile().mkdirs();
        int side = Integer.parseInt(px);
        ContentResolver resolver = context.getContentResolver();
        try {
            Bitmap bitmap;
            if (token.startsWith("content://")) {
                bitmap = DocumentsContract.getDocumentThumbnail(resolver, Uri.parse(token),
                        new Point(side, side), null);
            } else if (Build.VERSION.SDK_INT >= 29) {
                bitmap = resolver.loadThumbnail(item(token), new Size(side, side), null);
            } else {
                long id = Long.parseLong(token.substring(token.indexOf(':') + 1));
                bitmap = token.startsWith("video:")
                        ? MediaStore.Video.Thumbnails.getThumbnail(resolver, id,
                                MediaStore.Video.Thumbnails.MINI_KIND, null)
                        : MediaStore.Images.Thumbnails.getThumbnail(resolver, id,
                                MediaStore.Images.Thumbnails.MINI_KIND, null);
            }
            if (bitmap == null) {
                return "";
            }
            try (FileOutputStream stream = new FileOutputStream(file)) {
                bitmap.compress(Bitmap.CompressFormat.JPEG, 82, stream);
            }
            bitmap.recycle();
            return to;
        } catch (Exception e) {
            file.delete();
            return "";
        }
    }

    // ------------------------------------------------------------ documents

    /**
     * Shows the system's document picker (several, any type) and waits for
     * the user: the documents picked, as lines, their read grant kept; empty
     * when cancelled. Blocks: call from a worker thread.
     */
    public String pickDocuments(String mimeTypes) {
        List<Uri> picked = PickerActivity.pick(context, mimeTypes.split(","));
        StringBuilder out = new StringBuilder();
        for (Uri uri : picked) {
            try {
                context.getContentResolver().takePersistableUriPermission(uri,
                        Intent.FLAG_GRANT_READ_URI_PERMISSION);
            } catch (Exception e) {
                // A provider without persistable grants: readable while the
                // app runs.
            }
            out.append(docMeta(uri.toString()));
        }
        return out.toString();
    }

    /** A document's line (its size from the provider, else its descriptor). */
    public String docMeta(String token) {
        Uri uri = Uri.parse(token);
        ContentResolver resolver = context.getContentResolver();
        String name = "";
        String size = "";
        String modified = "";
        try (Cursor c = resolver.query(uri, null, null, null, null)) {
            if (c != null && c.moveToFirst()) {
                int n = c.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                int s = c.getColumnIndex(OpenableColumns.SIZE);
                int m = c.getColumnIndex(DocumentsContract.Document.COLUMN_LAST_MODIFIED);
                if (n >= 0 && !c.isNull(n)) {
                    name = c.getString(n);
                }
                if (s >= 0 && !c.isNull(s)) {
                    size = c.getString(s);
                }
                if (m >= 0 && !c.isNull(m)) {
                    modified = String.valueOf(c.getLong(m) / 1000);
                }
            }
        } catch (Exception e) {
            return "";
        }
        String mime = resolver.getType(uri);
        return token + SEP + "doc" + SEP + name + SEP + (mime == null ? "" : mime) + SEP + size
                + SEP + "" + SEP + modified + SEP + SEP + SEP + SEP + '\n';
    }

    /** Lets go of a document's kept grant. */
    public String releaseDoc(String token) {
        try {
            context.getContentResolver().releasePersistableUriPermission(Uri.parse(token),
                    Intent.FLAG_GRANT_READ_URI_PERMISSION);
        } catch (Exception e) {
            // Not kept, or already released.
        }
        return "";
    }
}
