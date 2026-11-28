package dev.dexdrop;

import android.Manifest;
import android.content.Context;
import android.content.pm.PackageManager;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.NetworkRequest;
import android.net.wifi.WifiManager;
import android.net.wifi.WifiNetworkSpecifier;
import android.net.wifi.p2p.WifiP2pConfig;
import android.net.wifi.p2p.WifiP2pGroup;
import android.net.wifi.p2p.WifiP2pInfo;
import android.net.wifi.p2p.WifiP2pManager;
import android.os.Build;
import android.os.Looper;
import android.util.Log;
import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

/**
 * DexDrop's direct link on Android (docs/CONNECTIVITY.md 4.3): hosting a
 * Wi-Fi Direct group with credentials made for one transfer, which other
 * devices join as an ordinary WPA2 network, and joining such a network with
 * {@link WifiNetworkSpecifier}.
 *
 * <p>Made by the Rust side (crates/dexdrop-net/src/direct/android.rs), which
 * calls these methods from its own threads; they block until Android
 * answers. Every method returns text: {@code ok} and its fields, or
 * {@code error} and why, separated by newlines. A missing permission is
 * {@code error\npermission}: the app asks for it before hosting
 * ({@code NEARBY_WIFI_DEVICES} from Android 13, fine location before).
 *
 * <p>A joined network has no internet and is not the process's default:
 * the Rust side binds its sockets to it with the network's handle
 * ({@code android_setsocknetwork}).
 *
 * <p>Build with scripts/build-nsd-dex.sh.
 */
public final class DirectLinkBridge {
    private static final String TAG = "DexDropDirect";

    private final Context context;
    private final WifiP2pManager p2p;
    private final Object lock = new Object();
    private WifiP2pManager.Channel channel;
    /** Joined networks by their handle, with the request that holds each. */
    private final Map<String, ConnectivityManager.NetworkCallback> joined = new HashMap<>();

    /** {@code context}: the application's, which NSH gives the Rust side. */
    public DirectLinkBridge(Context context) {
        this.context = context.getApplicationContext();
        this.p2p = (WifiP2pManager) this.context.getSystemService(Context.WIFI_P2P_SERVICE);
    }

    /** {@code host,join}, each present when this device can do it. */
    public String capabilities() {
        boolean host = p2p != null
                && context.getPackageManager().hasSystemFeature(PackageManager.FEATURE_WIFI_DIRECT);
        boolean join = Build.VERSION.SDK_INT >= 29;
        return (host ? "host" : "") + "," + (join ? "join" : "");
    }

    /**
     * Hosts a Wi-Fi Direct group named {@code ssid} with {@code passphrase}
     * on {@code band} ({@code 5} or {@code 2.4}); before Android 10 the
     * system chooses all three. Returns {@code ok}, the name, the
     * passphrase, the band and this device's address in the group.
     */
    public String host(String ssid, String passphrase, String band) {
        try {
            if (p2p == null) {
                return "error\nThis device has no Wi-Fi Direct.";
            }
            WifiManager wifi = (WifiManager) context.getSystemService(Context.WIFI_SERVICE);
            if (wifi != null && !wifi.isWifiEnabled()) {
                return "error\nTurn on Wi-Fi first.";
            }
            if (!permitted()) {
                return "error\npermission";
            }
            final WifiP2pManager.Channel ch = channel();
            // A group left from before (this app's) goes first.
            await(l -> p2p.removeGroup(ch, l), 5);
            String failure;
            String used = band;
            if (Build.VERSION.SDK_INT >= 29) {
                WifiP2pConfig.Builder builder = new WifiP2pConfig.Builder()
                        .setNetworkName(ssid)
                        .setPassphrase(passphrase)
                        .enablePersistentMode(false);
                if ("5".equals(band)) {
                    builder.setGroupOperatingBand(WifiP2pConfig.GROUP_OWNER_BAND_5GHZ);
                } else if ("2.4".equals(band)) {
                    builder.setGroupOperatingBand(WifiP2pConfig.GROUP_OWNER_BAND_2GHZ);
                }
                final WifiP2pConfig config = builder.build();
                failure = await(l -> p2p.createGroup(ch, config, l), 15);
            } else {
                failure = await(l -> p2p.createGroup(ch, l), 15);
                used = "auto";
            }
            if (failure != null) {
                Log.w(TAG, "createGroup on " + band + ": " + failure);
                return "error\n" + failure;
            }
            // The group forms moments after Android accepted it.
            for (int i = 0; i < 40; i++) {
                WifiP2pGroup group = groupInfo(ch);
                WifiP2pInfo info = connectionInfo(ch);
                if (group != null && group.isGroupOwner() && info != null && info.groupFormed
                        && info.groupOwnerAddress != null) {
                    Log.i(TAG, "hosting " + group.getNetworkName() + " on " + used);
                    return "ok\n" + group.getNetworkName() + "\n" + group.getPassphrase() + "\n"
                            + used + "\n" + info.groupOwnerAddress.getHostAddress();
                }
                Thread.sleep(250);
            }
            stop();
            return "error\nThe direct link did not come up.";
        } catch (SecurityException e) {
            return "error\npermission";
        } catch (Exception e) {
            Log.w(TAG, "host", e);
            return "error\n" + e;
        }
    }

    /** Stops hosting. */
    public String stop() {
        try {
            if (p2p != null) {
                final WifiP2pManager.Channel ch = channel();
                await(l -> p2p.removeGroup(ch, l), 5);
            }
            return "ok";
        } catch (Exception e) {
            return "error\n" + e;
        }
    }

    /**
     * Joins the WPA2 network {@code ssid} (Android 10+): the system asks the
     * user once. Returns {@code ok} and the network's handle, which binds
     * sockets to it and leaves it ({@link #leave}).
     */
    public String join(String ssid, String passphrase) {
        if (Build.VERSION.SDK_INT < 29) {
            return "error\nJoining a direct link needs Android 10 or later.";
        }
        try {
            ConnectivityManager cm =
                    (ConnectivityManager) context.getSystemService(Context.CONNECTIVITY_SERVICE);
            WifiNetworkSpecifier spec = new WifiNetworkSpecifier.Builder()
                    .setSsid(ssid)
                    .setWpa2Passphrase(passphrase)
                    .build();
            NetworkRequest request = new NetworkRequest.Builder()
                    .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
                    .removeCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                    .setNetworkSpecifier(spec)
                    .build();
            final Network[] got = {null};
            final CountDownLatch done = new CountDownLatch(1);
            ConnectivityManager.NetworkCallback callback = new ConnectivityManager.NetworkCallback() {
                @Override public void onAvailable(Network network) {
                    got[0] = network;
                    done.countDown();
                }

                @Override public void onUnavailable() {
                    done.countDown();
                }
            };
            // The user picks the network in the system's dialog: a minute.
            cm.requestNetwork(request, callback, 60_000);
            if (!done.await(65, TimeUnit.SECONDS) || got[0] == null) {
                try {
                    cm.unregisterNetworkCallback(callback);
                } catch (Exception ignored) {
                    // Already released by the timeout.
                }
                return "error\nThe direct link was not joined (declined, or out of range).";
            }
            String handle = Long.toString(got[0].getNetworkHandle());
            synchronized (lock) {
                joined.put(handle, callback);
            }
            Log.i(TAG, "joined " + ssid);
            return "ok\n" + handle;
        } catch (SecurityException e) {
            return "error\npermission";
        } catch (Exception e) {
            Log.w(TAG, "join", e);
            return "error\n" + e;
        }
    }

    /** Leaves a network {@link #join} joined. */
    public String leave(String handle) {
        ConnectivityManager.NetworkCallback callback;
        synchronized (lock) {
            callback = joined.remove(handle);
        }
        if (callback != null) {
            ConnectivityManager cm =
                    (ConnectivityManager) context.getSystemService(Context.CONNECTIVITY_SERVICE);
            try {
                cm.unregisterNetworkCallback(callback);
            } catch (Exception e) {
                Log.w(TAG, "leave", e);
            }
        }
        return "ok";
    }

    // ------------------------------------------------------------ helpers

    /** Hosting needs NEARBY_WIFI_DEVICES (13+) or fine location (before). */
    private boolean permitted() {
        String permission = Build.VERSION.SDK_INT >= 33
                ? "android.permission.NEARBY_WIFI_DEVICES"
                : Manifest.permission.ACCESS_FINE_LOCATION;
        return context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED;
    }

    private WifiP2pManager.Channel channel() {
        synchronized (lock) {
            if (channel == null) {
                channel = p2p.initialize(context, Looper.getMainLooper(), () -> {
                    synchronized (lock) {
                        channel = null;
                    }
                });
            }
            return channel;
        }
    }

    private interface Action {
        void run(WifiP2pManager.ActionListener listener);
    }

    /** Runs one Wi-Fi Direct action; null when it succeeded, else why not. */
    private static String await(Action action, int seconds) throws InterruptedException {
        final String[] failure = {null};
        final CountDownLatch done = new CountDownLatch(1);
        action.run(new WifiP2pManager.ActionListener() {
            @Override public void onSuccess() {
                done.countDown();
            }

            @Override public void onFailure(int reason) {
                failure[0] = reasonText(reason);
                done.countDown();
            }
        });
        if (!done.await(seconds, TimeUnit.SECONDS)) {
            return "Android did not answer.";
        }
        return failure[0];
    }

    private static String reasonText(int reason) {
        switch (reason) {
            case WifiP2pManager.P2P_UNSUPPORTED:
                return "This device has no Wi-Fi Direct.";
            case WifiP2pManager.BUSY:
                return "Wi-Fi Direct is busy. Try again in a moment.";
            default:
                return "Wi-Fi Direct refused (" + reason + ").";
        }
    }

    private WifiP2pGroup groupInfo(WifiP2pManager.Channel ch) throws InterruptedException {
        final WifiP2pGroup[] out = {null};
        final CountDownLatch done = new CountDownLatch(1);
        p2p.requestGroupInfo(ch, group -> {
            out[0] = group;
            done.countDown();
        });
        done.await(3, TimeUnit.SECONDS);
        return out[0];
    }

    private WifiP2pInfo connectionInfo(WifiP2pManager.Channel ch) throws InterruptedException {
        final WifiP2pInfo[] out = {null};
        final CountDownLatch done = new CountDownLatch(1);
        p2p.requestConnectionInfo(ch, info -> {
            out[0] = info;
            done.countDown();
        });
        done.await(3, TimeUnit.SECONDS);
        return out[0];
    }
}
