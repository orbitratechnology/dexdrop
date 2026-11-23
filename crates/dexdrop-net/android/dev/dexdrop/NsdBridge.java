package dev.dexdrop;

import android.content.Context;
import android.net.nsd.NsdManager;
import android.net.nsd.NsdServiceInfo;
import android.util.Log;
import java.net.InetAddress;
import java.nio.charset.StandardCharsets;
import java.util.ArrayDeque;
import java.util.Map;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;

/**
 * DexDrop's bridge to Android's DNS-SD service ({@link NsdManager}).
 *
 * <p>Loaded from memory by the Rust discovery adapter
 * (crates/dexdrop-net/src/discovery/nsd.rs), which calls these methods from
 * one thread and reads what was found with {@link #poll}. The system does
 * the multicast work, so the app needs no multicast lock or extra
 * permission.
 *
 * <p>Events, one per {@link #poll}, fields separated by U+0001:
 * <ul>
 *   <li>{@code F name host port txt}: a service resolved; {@code txt} is
 *       {@code key=value} entries separated by U+0002;
 *   <li>{@code L name}: a service went away;
 *   <li>{@code E message}: discovery failed.
 * </ul>
 *
 * <p>Build with scripts/build-nsd-dex.sh.
 */
public final class NsdBridge {
    private static final char SEP = '\u0001';
    private static final char TXT_SEP = '\u0002';
    private static final String TAG = "DexDropNsd";

    private final NsdManager nsd;
    private final LinkedBlockingQueue<String> events = new LinkedBlockingQueue<>();
    private final ArrayDeque<NsdServiceInfo> toResolve = new ArrayDeque<>();
    private boolean resolving;
    private NsdManager.DiscoveryListener browser;
    private NsdManager.RegistrationListener registration;

    public NsdBridge() throws Exception {
        Context context = (Context) Class.forName("android.app.ActivityThread")
                .getMethod("currentApplication")
                .invoke(null);
        if (context == null) {
            throw new IllegalStateException("no application context");
        }
        nsd = (NsdManager) context.getSystemService(Context.NSD_SERVICE);
        if (nsd == null) {
            throw new IllegalStateException("no network service discovery");
        }
    }

    /** Starts browsing for {@code type}, e.g. {@code _dexdrop._tcp}. */
    public synchronized void browse(String type) {
        if (browser != null) {
            return;
        }
        browser = new NsdManager.DiscoveryListener() {
            @Override public void onDiscoveryStarted(String serviceType) {
                Log.i(TAG, "browsing for " + serviceType);
            }
            @Override public void onDiscoveryStopped(String serviceType) {}

            @Override public void onStartDiscoveryFailed(String serviceType, int errorCode) {
                Log.w(TAG, "discovery failed: " + errorCode);
                events.add("E" + SEP + "Discovery could not start (" + errorCode + ").");
            }

            @Override public void onStopDiscoveryFailed(String serviceType, int errorCode) {}

            @Override public void onServiceFound(NsdServiceInfo info) {
                resolve(info);
            }

            @Override public void onServiceLost(NsdServiceInfo info) {
                events.add("L" + SEP + info.getServiceName());
            }
        };
        nsd.discoverServices(type, NsdManager.PROTOCOL_DNS_SD, browser);
    }

    /** Advertises this device, replacing an earlier advertisement. */
    public synchronized void advertise(String name, String type, int port, String txt) {
        withdraw();
        NsdServiceInfo info = new NsdServiceInfo();
        info.setServiceName(name);
        info.setServiceType(type);
        info.setPort(port);
        if (!txt.isEmpty()) {
            for (String entry : txt.split(String.valueOf(TXT_SEP))) {
                int eq = entry.indexOf('=');
                if (eq > 0) {
                    info.setAttribute(entry.substring(0, eq), entry.substring(eq + 1));
                }
            }
        }
        registration = new NsdManager.RegistrationListener() {
            @Override public void onServiceRegistered(NsdServiceInfo registered) {
                Log.i(TAG, "advertised as " + registered.getServiceName());
            }
            @Override public void onServiceUnregistered(NsdServiceInfo unregistered) {}

            @Override public void onRegistrationFailed(NsdServiceInfo failed, int errorCode) {
                Log.w(TAG, "registration failed: " + errorCode);
                events.add("E" + SEP + "This device could not be advertised (" + errorCode + ").");
            }

            @Override public void onUnregistrationFailed(NsdServiceInfo failed, int errorCode) {}
        };
        nsd.registerService(info, NsdManager.PROTOCOL_DNS_SD, registration);
    }

    /** Stops advertising; browsing continues. */
    public synchronized void withdraw() {
        if (registration != null) {
            try {
                nsd.unregisterService(registration);
            } catch (IllegalArgumentException ignored) {
                // Never registered (registration failed).
            }
            registration = null;
        }
    }

    /** Stops browsing and advertising. */
    public synchronized void stop() {
        withdraw();
        if (browser != null) {
            try {
                nsd.stopServiceDiscovery(browser);
            } catch (IllegalArgumentException ignored) {
                // Discovery never started.
            }
            browser = null;
        }
    }

    /** The next event, or null after {@code timeoutMs} without one. */
    public String poll(int timeoutMs) throws InterruptedException {
        return events.poll(timeoutMs, TimeUnit.MILLISECONDS);
    }

    // Before Android 14 the system resolves one service at a time: queue.
    private synchronized void resolve(NsdServiceInfo info) {
        toResolve.add(info);
        resolveNext();
    }

    private synchronized void resolveNext() {
        if (resolving || toResolve.isEmpty()) {
            return;
        }
        resolving = true;
        final NsdServiceInfo info = toResolve.poll();
        resolveWithRetries(info, 3);
    }

    @SuppressWarnings("deprecation")
    private void resolveWithRetries(final NsdServiceInfo info, final int attempts) {
        nsd.resolveService(info, new NsdManager.ResolveListener() {
            @Override public void onResolveFailed(NsdServiceInfo failed, int errorCode) {
                if (attempts > 1) {
                    resolveWithRetries(info, attempts - 1);
                } else {
                    done();
                }
            }

            @Override public void onServiceResolved(NsdServiceInfo resolved) {
                InetAddress host = resolved.getHost();
                if (host != null) {
                    StringBuilder txt = new StringBuilder();
                    for (Map.Entry<String, byte[]> e : resolved.getAttributes().entrySet()) {
                        if (txt.length() > 0) {
                            txt.append(TXT_SEP);
                        }
                        txt.append(e.getKey()).append('=');
                        if (e.getValue() != null) {
                            txt.append(new String(e.getValue(), StandardCharsets.UTF_8));
                        }
                    }
                    String address = host.getHostAddress();
                    Log.i(TAG, "resolved " + resolved.getServiceName() + " at " + address);
                    int scope = address.indexOf('%');
                    events.add("F" + SEP + resolved.getServiceName() + SEP
                            + (scope >= 0 ? address.substring(0, scope) : address) + SEP
                            + resolved.getPort() + SEP + txt);
                }
                done();
            }
        });
    }

    private synchronized void done() {
        resolving = false;
        resolveNext();
    }
}
