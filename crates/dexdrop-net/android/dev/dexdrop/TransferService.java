package dev.dexdrop;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.net.wifi.WifiManager;
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
 * <p>While files move it also holds Wi-Fi locks, so the radio does not doze
 * between packets (Wi-Fi power save is a common cause of slow, uneven phone
 * transfers): {@code WIFI_MODE_FULL_LOW_LATENCY} (Android 10+, effective
 * while the app is in front with the screen on) and, before Android 14,
 * {@code WIFI_MODE_FULL_HIGH_PERF}. They need {@code WAKE_LOCK}, which
 * dexdrop-net's Cargo.toml declares, and are released when no transfer
 * moves ({@link #hide}), so they cost battery only during transfers.
 *
 * <p>Build with scripts/build-nsd-dex.sh.
 */
public final class TransferService extends Service {
    private static final String TAG = "DexDropTransfers";
    private static final String CHANNEL = "dexdrop-transfers";
    private static final int ID = 0xDD01;

    private static final int IDLE = 0;
    /** Asked to start; Android has not called onStartCommand yet. */
    private static final int STARTING = 1;
    private static final int RUNNING = 2;

    /**
     * Guards the fields below: show and hide come from Rust threads, the
     * system's calls on the main thread.
     */
    private static final Object LOCK = new Object();
    private static int state = IDLE;
    /**
     * hide came while starting. Android kills an app that stops a service
     * started with startForegroundService before it calls startForeground
     * (ForegroundServiceDidNotStartInTimeException), as a transfer that ends
     * within moments would: onStartCommand goes into the foreground, then
     * stops.
     */
    private static boolean stopWhenStarted;
    private static String lastTitle = "DexDrop";
    private static String lastText = "";
    private static int lastPercent = -1;
    private static boolean lastUpload;
    /** The Wi-Fi locks held while files move (none when not allowed). */
    private static WifiManager.WifiLock lowLatency;
    private static WifiManager.WifiLock highPerf;

    /**
     * Shows {@code title} and {@code text} with {@code percent} done (-1:
     * not known yet; {@code upload}: something is being sent), starting the
     * service when it does not run. Returns {@code ok}, or why it could not
     * start (the app is in the background).
     */
    public static String show(
            Context context, String title, String text, int percent, boolean upload) {
        synchronized (LOCK) {
            lastTitle = title;
            lastText = text;
            lastPercent = percent;
            lastUpload = upload;
            stopWhenStarted = false;
            holdWifi(context);
            try {
                if (state == RUNNING) {
                    manager(context).notify(ID, build(context, title, text, percent, upload));
                } else if (state == IDLE) {
                    // onStartCommand shows the newest status.
                    context.startForegroundService(new Intent(context, TransferService.class));
                    state = STARTING;
                }
                return "ok";
            } catch (Exception e) {
                // ForegroundServiceStartNotAllowedException (Android 12+)
                // from the background, or a SecurityException.
                Log.w(TAG, "could not start: " + e);
                return e.toString();
            }
        }
    }

    /** Stops the service and takes its notification away. */
    public static void hide(Context context) {
        synchronized (LOCK) {
            releaseWifi();
            if (state == STARTING) {
                stopWhenStarted = true;
                return;
            }
            if (state != RUNNING) {
                return;
            }
            state = IDLE;
            try {
                context.stopService(new Intent(context, TransferService.class));
            } catch (Exception e) {
                Log.w(TAG, "could not stop: " + e);
            }
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        synchronized (LOCK) {
            // Every startForegroundService must be answered with
            // startForeground, even one that is to stop at once.
            Notification notification = build(this, lastTitle, lastText, lastPercent, lastUpload);
            try {
                if (Build.VERSION.SDK_INT >= 29) {
                    startForeground(ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC);
                } else {
                    startForeground(ID, notification);
                }
            } catch (Exception e) {
                Log.w(TAG, "could not run in the foreground: " + e);
                state = IDLE;
                stopWhenStarted = false;
                stopSelf();
                return START_NOT_STICKY;
            }
            if (stopWhenStarted) {
                stopWhenStarted = false;
                state = IDLE;
                stopForeground(STOP_FOREGROUND_REMOVE);
                stopSelf();
            } else {
                state = RUNNING;
            }
        }
        // Not restarted by the system: the app restarts transfers itself.
        return START_NOT_STICKY;
    }

    /** Android 15 caps data-sync services at six hours a day. */
    @Override
    public void onTimeout(int startId, int fgsType) {
        synchronized (LOCK) {
            state = IDLE;
        }
        stopSelf();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    /** Takes the Wi-Fi locks, once; a failure leaves Wi-Fi as it was. */
    private static void holdWifi(Context context) {
        if (lowLatency != null || highPerf != null) {
            return;
        }
        try {
            WifiManager wifi = (WifiManager)
                    context.getApplicationContext().getSystemService(Context.WIFI_SERVICE);
            if (wifi == null) {
                return;
            }
            if (Build.VERSION.SDK_INT >= 29) {
                lowLatency = wifi.createWifiLock(
                        WifiManager.WIFI_MODE_FULL_LOW_LATENCY, "DexDrop:transfer-low-latency");
                lowLatency.setReferenceCounted(false);
                lowLatency.acquire();
            }
            if (Build.VERSION.SDK_INT < 34) {
                highPerf = wifi.createWifiLock(
                        WifiManager.WIFI_MODE_FULL_HIGH_PERF, "DexDrop:transfer-high-perf");
                highPerf.setReferenceCounted(false);
                highPerf.acquire();
            }
        } catch (Exception e) {
            // No WAKE_LOCK or no Wi-Fi: transfers go on as before.
            Log.w(TAG, "no Wi-Fi lock: " + e);
        }
    }

    /** Lets Wi-Fi save power again. */
    private static void releaseWifi() {
        for (WifiManager.WifiLock lock : new WifiManager.WifiLock[] {lowLatency, highPerf}) {
            try {
                if (lock != null && lock.isHeld()) {
                    lock.release();
                }
            } catch (Exception e) {
                Log.w(TAG, "could not release a Wi-Fi lock: " + e);
            }
        }
        lowLatency = null;
        highPerf = null;
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
