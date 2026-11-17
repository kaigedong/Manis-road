package com.manisroad.app;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.pm.ServiceInfo;
import android.location.Location;
import android.location.LocationListener;
import android.location.LocationManager;
import android.os.Build;
import android.os.IBinder;
import android.os.Looper;

import com.manisroad.app.core.CoreResult;
import com.manisroad.app.core.MotuCore;
import com.manisroad.app.core.OfflineData;

import org.json.JSONObject;

/**
 * Foreground GNSS cruise. It keeps {@link LocationManager#GPS_PROVIDER} running
 * while the screen is off and scans the cached cameras on every fix. No network
 * is involved. Results are broadcast to {@link MapActivity} so the UI can follow
 * the position; the notification keeps the user aware while the activity is
 * paused.
 */
public final class CruiseService extends Service implements LocationListener {
    public static final String ACTION_START = "com.manisroad.app.CRUISE_START";
    public static final String ACTION_STOP = "com.manisroad.app.CRUISE_STOP";
    public static final String ACTION_UPDATE = "com.manisroad.app.CRUISE_UPDATE";
    public static final String ACTION_STATE = "com.manisroad.app.CRUISE_STATE";
    public static final String EXTRA_LATITUDE = "latitude";
    public static final String EXTRA_LONGITUDE = "longitude";
    public static final String EXTRA_BEARING = "bearing";
    public static final String EXTRA_SCAN = "scan";
    public static final String EXTRA_RUNNING = "running";

    private static final String CHANNEL_ID = "cruise";
    private static final int NOTIFICATION_ID = 41;
    private static volatile boolean running;

    private LocationManager locationManager;
    private NotificationManager notificationManager;
    private String lastText;

    public static boolean isRunning() {
        return running;
    }

    @Override
    public void onCreate() {
        super.onCreate();
        notificationManager = (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && notificationManager != null) {
            NotificationChannel channel = new NotificationChannel(
                    CHANNEL_ID,
                    getString(R.string.cruise_channel_name),
                    NotificationManager.IMPORTANCE_LOW);
            channel.setDescription(getString(R.string.cruise_channel_description));
            notificationManager.createNotificationChannel(channel);
        }
        MotuCore.ensureInit(getApplicationContext());
        String cameras = OfflineData.readText(OfflineData.camerasFile(getApplicationContext()));
        org.json.JSONArray array = OfflineData.parseCameras(cameras);
        if (array != null) {
            MotuCore.updateCameras(array);
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        String action = intent == null ? null : intent.getAction();
        if (ACTION_STOP.equals(action)) {
            stopSelf();
            return START_NOT_STICKY;
        }
        if (checkSelfPermission(android.Manifest.permission.ACCESS_FINE_LOCATION)
                != PackageManager.PERMISSION_GRANTED) {
            stopSelf();
            return START_NOT_STICKY;
        }
        startForegroundCompat();
        startLocationUpdates();
        running = true;
        broadcastState(true);
        return START_STICKY;
    }

    private void startForegroundCompat() {
        Notification notification = buildNotification(getString(R.string.cruise_running));
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION);
        } else {
            startForeground(NOTIFICATION_ID, notification);
        }
    }

    private Notification buildNotification(String text) {
        lastText = text;
        Intent open = new Intent(this, MapActivity.class);
        open.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_NEW_TASK);
        PendingIntent pending = PendingIntent.getActivity(
                this, 0, open,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        Notification.Builder builder = Build.VERSION.SDK_INT >= Build.VERSION_CODES.O
                ? new Notification.Builder(this, CHANNEL_ID)
                : new Notification.Builder(this);
        return builder
                .setContentTitle(getString(R.string.cruise_notification_title))
                .setContentText(text)
                .setSmallIcon(R.drawable.ic_launcher)
                .setContentIntent(pending)
                .setOngoing(true)
                .build();
    }

    private void startLocationUpdates() {
        locationManager = (LocationManager) getSystemService(LOCATION_SERVICE);
        if (locationManager == null) {
            stopSelf();
            return;
        }
        try {
            if (!locationManager.isProviderEnabled(LocationManager.GPS_PROVIDER)) {
                stopSelf();
                return;
            }
            locationManager.requestLocationUpdates(
                    LocationManager.GPS_PROVIDER, 1000L, 1f, this, Looper.getMainLooper());
        } catch (SecurityException error) {
            stopSelf();
        }
    }

    @Override
    public void onLocationChanged(Location location) {
        double latitude = location.getLatitude();
        double longitude = location.getLongitude();
        float bearing = location.hasBearing() ? location.getBearing() : 0f;

        String scanJson = "";
        // Keep the previous alert when the scan engine is throttled or there is
        // nothing ahead; only the very first fix shows "waiting for position".
        String text = lastText == null ? getString(R.string.cruise_running) : lastText;
        CoreResult scan = MotuCore.scan(latitude, longitude, bearing, true);
        JSONObject data = scan.objectData();
        if (scan.ok && data != null) {
            scanJson = data.toString();
            text = formatScan(data);
        }
        if (!text.equals(lastText)) {
            notificationManager.notify(NOTIFICATION_ID, buildNotification(text));
        }

        Intent update = new Intent(ACTION_UPDATE).setPackage(getPackageName());
        update.putExtra(EXTRA_LATITUDE, latitude);
        update.putExtra(EXTRA_LONGITUDE, longitude);
        update.putExtra(EXTRA_BEARING, bearing);
        update.putExtra(EXTRA_SCAN, scanJson);
        sendBroadcast(update);
    }

    private String formatScan(JSONObject scan) {
        org.json.JSONArray types = scan.optJSONArray("cameraTypes");
        String type = types != null && types.length() > 0
                ? typeName(types.optInt(0)) : "电子眼";
        double distance = scan.optDouble("primaryDistanceM", 0.0);
        return getString(R.string.cruise_alert, type, Math.max(1, Math.round(distance)));
    }

    private static String typeName(int type) {
        switch (type) {
            case 1: return "闯限行";
            case 2: return "应急车道";
            case 3: return "非机动车道";
            case 4: return "闯红灯";
            case 5: return "超速";
            case 6: return "公交车道";
            case 7: return "未按车道";
            case 8: return "逆行";
            case 9: return "尾号限行";
            case 10: return "不带头盔";
            case 11: return "噪音";
            case 12: return "现场罚单";
            case 13: return "压线";
            case 14: return "违规用灯";
            default: return "电子眼";
        }
    }

    private void broadcastState(boolean isRunning) {
        Intent state = new Intent(ACTION_STATE).setPackage(getPackageName());
        state.putExtra(EXTRA_RUNNING, isRunning);
        sendBroadcast(state);
    }

    @Override
    public void onDestroy() {
        if (locationManager != null) {
            locationManager.removeUpdates(this);
        }
        running = false;
        broadcastState(false);
        if (notificationManager != null) {
            notificationManager.cancel(NOTIFICATION_ID);
        }
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
