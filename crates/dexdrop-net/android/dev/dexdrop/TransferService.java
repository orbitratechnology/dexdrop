package dev.dexdrop;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.util.Log;

/**
 * Keeps DexDrop running while files move (PRD 8.5): a foreground service of
 * type {@code dataSync} with an ongoing notification that shows the
 * transfers' progress and opens the app when tapped. Android stops a
 * backgrounded app's network within seconds otherwise.
 *
 * <p>The Rust side (crates/dexdrop-net/src/android_service.rs) calls
 * {@link #show} as transfers move and {@link #hide} once none does, from any
 * thread. Android only lets an app start a foreground service while it is
 * in front (Android 12+): a transfer that starts in the background goes on
 * without it, as before. dexdrop-net's Cargo.toml declares the service and
 * its permissions ({@code FOREGROUND_SERVICE},
 * {@code FOREGROUND_SERVICE_DATA_SYNC}).
 *
 * <p>Build with scripts/build-nsd-dex.sh.
 */
public final class TransferService extends Service {
    private static final String TAG = "DexDropTransfers";
    private static final String CHANNEL = "dexdrop-transfers";
    private static final int ID = 0xDD01;
    private static final String TITLE = "title";
    private static final String TEXT = "text";
    private static final String PERCENT = "percent";
    private static final String UPLOAD = "upload";

    /** Whether the service runs (set by the system's calls on the main thread). */
    private static volatile boolean running;

    /**
     * Shows {@code title} and {@code text} with {@code percent} done (-1:
     * not known yet; {@code upload}: something is being sent), starting the
     * service when it does not run. Returns {@code ok}, or why it could not
     * start (the app is in the background).
     */
    public static String show(
            Context context, String title, String text, int percent, boolean upload) {
        try {
            if (running) {
                manager(context).notify(ID, build(context, title, text, percent, upload));
                return "ok";
            }
            Intent start = new Intent(context, TransferService.class)
                    .putExtra(TITLE, title)
                    .putExtra(TEXT, text)
                    .putExtra(PERCENT, percent)
                    .putExtra(UPLOAD, upload);
            context.startForegroundService(start);
            return "ok";
        } catch (Exception e) {
            // ForegroundServiceStartNotAllowedException (Android 12+) from
            // the background, or a SecurityException.
            Log.w(TAG, "could not start: " + e);
            return e.toString();
        }
    }

    /** Stops the service and takes its notification away. */
    public static void hide(Context context) {
        try {
            context.stopService(new Intent(context, TransferService.class));
        } catch (Exception e) {
            Log.w(TAG, "could not stop: " + e);
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        String title = intent != null ? intent.getStringExtra(TITLE) : null;
        String text = intent != null ? intent.getStringExtra(TEXT) : null;
        int percent = intent != null ? intent.getIntExtra(PERCENT, -1) : -1;
        boolean upload = intent != null && intent.getBooleanExtra(UPLOAD, false);
        Notification notification = build(
                this, title != null ? title : "DexDrop", text != null ? text : "", percent, upload);
        try {
            if (Build.VERSION.SDK_INT >= 29) {
                startForeground(ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC);
            } else {
                startForeground(ID, notification);
            }
            running = true;
        } catch (Exception e) {
            Log.w(TAG, "could not run in the foreground: " + e);
            stopSelf();
        }
        // Not restarted by the system: the app restarts transfers itself.
        return START_NOT_STICKY;
    }

    /** Android 15 caps data-sync services at six hours a day. */
    @Override
    public void onTimeout(int startId, int fgsType) {
        stopSelf();
    }

    @Override
    public void onDestroy() {
        running = false;
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    private static NotificationManager manager(Context context) {
        return (NotificationManager) context.getSystemService(Context.NOTIFICATION_SERVICE);
    }

    private static Notification build(
            Context context, String title, String text, int percent, boolean upload) {
        NotificationManager manager = manager(context);
        if (manager.getNotificationChannel(CHANNEL) == null) {
            NotificationChannel channel = new NotificationChannel(
                    CHANNEL, "Transfers", NotificationManager.IMPORTANCE_LOW);
            channel.setDescription("Files moving to and from your devices");
            channel.setShowBadge(false);
            manager.createNotificationChannel(channel);
        }
        Notification.Builder builder = new Notification.Builder(context, CHANNEL)
                // The system's arrows: a launcher icon (adaptive) as the
                // small icon breaks Android 8.0's status bar.
                .setSmallIcon(upload
                        ? android.R.drawable.stat_sys_upload
                        : android.R.drawable.stat_sys_download)
                .setContentTitle(title)
                .setContentText(text)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setCategory(Notification.CATEGORY_PROGRESS)
                .setProgress(100, Math.max(percent, 0), percent < 0);
        Intent open = context.getPackageManager().getLaunchIntentForPackage(context.getPackageName());
        if (open != null) {
            open.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_SINGLE_TOP);
            builder.setContentIntent(PendingIntent.getActivity(
                    context, 0, open, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT));
        }
        if (Build.VERSION.SDK_INT >= 31) {
            builder.setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE);
        }
        return builder.build();
    }
}
