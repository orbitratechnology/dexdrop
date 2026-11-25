package dev.dexdrop;

import android.content.Context;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.ResolveInfo;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.drawable.Drawable;
import android.os.Build;
import java.io.File;
import java.io.FileOutputStream;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

/**
 * DexDrop's bridge to Android's {@link PackageManager}: the apps installed
 * (to send their APKs) and the details of an APK file.
 *
 * <p>Made by the Rust adapter (crates/dexdrop-net/src/apps.rs),
 * which calls these methods from a worker thread. Each app is one line,
 * fields separated by U+0001:
 * {@code package label versionName versionCode system size apks icon
 * installed updated minSdk targetSdk}, where {@code apks} are the APK
 * paths (base first, then splits) separated by U+0002 and {@code icon} is
 * a PNG of the app's icon written under the given folder.
 *
 * <p>Seeing other apps needs the manifest's {@code <queries>} entry for
 * launcher activities (Android 11+); dexdrop-net's Cargo.toml declares
 * it to the packaged APK.
 *
 * <p>Build with scripts/build-nsd-dex.sh.
 */
public final class AppsBridge {
    private static final char SEP = '\u0001';
    private static final char LIST_SEP = '\u0002';
    private static final int ICON_PX = 144;

    private final Context context;
    private final PackageManager pm;

    /** {@code context}: the application's, which NSH gives the Rust side. */
    public AppsBridge(Context context) {
        this.context = context;
        pm = context.getPackageManager();
    }

    /** The apps with a launcher entry, other than this one, one per line. */
    public String installed(String iconDir) {
        Intent main = new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER);
        List<ResolveInfo> found = pm.queryIntentActivities(main, 0);
        Set<String> seen = new HashSet<>();
        seen.add(context.getPackageName());
        StringBuilder out = new StringBuilder();
        for (ResolveInfo info : found) {
            String name = info.activityInfo.packageName;
            if (!seen.add(name)) {
                continue;
            }
            try {
                PackageInfo pkg = pm.getPackageInfo(name, 0);
                line(out, pkg, iconDir);
            } catch (Exception e) {
                // Uninstalled meanwhile, or not readable: left out.
            }
        }
        return out.toString();
    }

    /** The details of the APK file at {@code path}; empty when it is not one. */
    public String archive(String path, String iconDir) {
        PackageInfo pkg = pm.getPackageArchiveInfo(path, 0);
        if (pkg == null || pkg.applicationInfo == null) {
            return "";
        }
        // Its label and icon are read from the file itself.
        pkg.applicationInfo.sourceDir = path;
        pkg.applicationInfo.publicSourceDir = path;
        StringBuilder out = new StringBuilder();
        try {
            line(out, pkg, iconDir);
        } catch (Exception e) {
            return "";
        }
        return out.toString();
    }

    private void line(StringBuilder out, PackageInfo pkg, String iconDir) {
        ApplicationInfo app = pkg.applicationInfo;
        boolean system = (app.flags & ApplicationInfo.FLAG_SYSTEM) != 0
                && (app.flags & ApplicationInfo.FLAG_UPDATED_SYSTEM_APP) == 0;
        long code = Build.VERSION.SDK_INT >= 28 ? pkg.getLongVersionCode() : pkg.versionCode;
        StringBuilder apks = new StringBuilder();
        long size = 0;
        if (app.sourceDir != null) {
            apks.append(app.sourceDir);
            size += new File(app.sourceDir).length();
        }
        if (app.splitSourceDirs != null) {
            for (String split : app.splitSourceDirs) {
                apks.append(LIST_SEP).append(split);
                size += new File(split).length();
            }
        }
        String icon = icon(app, pkg.packageName + "-" + code, iconDir);
        out.append(clean(pkg.packageName)).append(SEP)
                .append(clean(String.valueOf(pm.getApplicationLabel(app)))).append(SEP)
                .append(clean(pkg.versionName == null ? "" : pkg.versionName)).append(SEP)
                .append(code).append(SEP)
                .append(system ? '1' : '0').append(SEP)
                .append(size).append(SEP)
                .append(apks).append(SEP)
                .append(icon).append(SEP)
                .append(pkg.firstInstallTime).append(SEP)
                .append(pkg.lastUpdateTime).append(SEP)
                .append(Build.VERSION.SDK_INT >= 24 ? app.minSdkVersion : 0).append(SEP)
                .append(app.targetSdkVersion)
                .append('\n');
    }

    /** The app's icon as a PNG under {@code dir} (made once per version). */
    private String icon(ApplicationInfo app, String key, String dir) {
        File file = new File(dir, key.replaceAll("[^A-Za-z0-9._-]", "_") + ".png");
        if (file.isFile() && file.length() > 0) {
            return file.getPath();
        }
        try {
            Drawable drawable = app.loadIcon(pm);
            Bitmap bitmap = Bitmap.createBitmap(ICON_PX, ICON_PX, Bitmap.Config.ARGB_8888);
            Canvas canvas = new Canvas(bitmap);
            drawable.setBounds(0, 0, ICON_PX, ICON_PX);
            drawable.draw(canvas);
            new File(dir).mkdirs();
            File temp = new File(dir, file.getName() + ".part");
            try (FileOutputStream stream = new FileOutputStream(temp)) {
                bitmap.compress(Bitmap.CompressFormat.PNG, 100, stream);
            }
            bitmap.recycle();
            return temp.renameTo(file) ? file.getPath() : "";
        } catch (Exception e) {
            return "";
        }
    }

    /** Text without the separators or line breaks. */
    private static String clean(String s) {
        StringBuilder out = new StringBuilder(s.length());
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            out.append(c < 0x20 ? ' ' : c);
        }
        return out.toString().trim();
    }
}
