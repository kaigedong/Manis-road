package com.manisroad.app;

import android.Manifest;
import android.app.Activity;
import android.app.AlertDialog;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Bundle;
import android.os.SystemClock;
import android.view.View;
import android.widget.Button;
import android.widget.CheckBox;
import android.widget.TextView;

import com.manisroad.app.core.CoreResult;
import com.manisroad.app.core.MotuCore;
import com.manisroad.app.core.OfflineData;
import com.manisroad.app.core.SigningCertificate;
import com.manisroad.app.ui.MapCanvasView;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.File;
import java.text.SimpleDateFormat;
import java.util.ArrayList;
import java.util.Date;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * Camera-alert screen driven by simulated GPS ticks. The default points are
 * invented samples; the sync button can replace them with official points for
 * a signed-in account. The demo points are not a claim about real enforcement.
 */
public final class MapActivity extends Activity {
    private static final double ORIGIN_LATITUDE = 31.2304;
    private static final double ORIGIN_LONGITUDE = 121.4737;
    /** AMap city code for the Shanghai demo origin. */
    private static final String OFFICIAL_CITY_CODE = "021";
    private static final int MAX_OFFICIAL_MARKERS = 200;
    /** Demo destination for the offline route button (Lujiazui). */
    private static final double DEST_LATITUDE = 31.2397;
    private static final double DEST_LONGITUDE = 121.4998;
    private static final int REQUEST_LOCATION = 1001;

    private JSONArray cameras = new JSONArray();
    private final List<MapCanvasView.Marker> markers = new ArrayList<>();
    private final List<double[]> routePoints = new ArrayList<>();
    /** Details exported by the web debugger or opened while online. */
    private final Map<String, JSONObject> offlineDetails = new LinkedHashMap<>();

    private MapCanvasView map;
    private TextView alert;
    private Button syncButton;
    private double latitude = ORIGIN_LATITUDE;
    private double longitude = ORIGIN_LONGITUDE;
    private float bearing;
    private boolean alertsEnabled = true;
    private boolean detailLoading;
    private File graphFile;
    private CheckBox avoidCameras;
    private boolean gpsEnabled;
    private boolean routePlanned;
    private JSONArray routeSteps;
    private String lastScanText = "";
    private long lastRerouteAt;

    private final BroadcastReceiver cruiseReceiver = new BroadcastReceiver() {
        @Override
        public void onReceive(Context context, Intent intent) {
            String action = intent.getAction();
            if (CruiseService.ACTION_UPDATE.equals(action)) {
                onCruiseUpdate(intent);
            } else if (CruiseService.ACTION_STATE.equals(action)) {
                gpsEnabled = intent.getBooleanExtra(CruiseService.EXTRA_RUNNING, false);
                syncGpsButton();
            }
        }
    };

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_map);

        map = findViewById(R.id.map_canvas);
        alert = findViewById(R.id.text_alert);
        map.setOnMarkerClickListener(this::openCameraDetail);

        CoreResult init = MotuCore.ensureInit(this);
        if (!init.ok) {
            alert.setText(getString(R.string.core_init_failed, init.error));
        }

        graphFile = OfflineData.ensureGraph(this);
        avoidCameras = findViewById(R.id.checkbox_avoid_cameras);
        loadOfflineDetails();
        buildRoute();
        if (!loadOfflineCameras()) {
            loadSampleCameras();
        }
        render();

        findViewById(R.id.button_offline_route).setOnClickListener(view -> planOfflineRoute());
        findViewById(R.id.button_gps).setOnClickListener(view -> toggleGps());
        syncButton = findViewById(R.id.button_sync_official);
        syncButton.setOnClickListener(view -> syncOfficialCameras());
        findViewById(R.id.button_forward).setOnClickListener(view -> advance());
        findViewById(R.id.button_turn).setOnClickListener(view -> turn());
        findViewById(R.id.button_audit).setOnClickListener(view -> audit());
        findViewById(R.id.button_reset).setOnClickListener(view -> reset());
    }

    @Override
    protected void onStart() {
        super.onStart();
        IntentFilter filter = new IntentFilter();
        filter.addAction(CruiseService.ACTION_UPDATE);
        filter.addAction(CruiseService.ACTION_STATE);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(cruiseReceiver, filter, Context.RECEIVER_NOT_EXPORTED);
        } else {
            registerReceiver(cruiseReceiver, filter);
        }
        gpsEnabled = CruiseService.isRunning();
        syncGpsButton();
    }

    @Override
    protected void onStop() {
        unregisterReceiver(cruiseReceiver);
        super.onStop();
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions,
                                           int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        if (requestCode != REQUEST_LOCATION) {
            return;
        }
        if (grantResults.length > 0 && grantResults[0] == PackageManager.PERMISSION_GRANTED) {
            startCruiseService();
        } else {
            alert.setText(R.string.gps_denied);
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        CoreResult state = MotuCore.loadState();
        JSONObject data = state.objectData();
        if (state.ok && data != null) {
            JSONObject settings = data.optJSONObject("settings");
            if (settings != null) {
                alertsEnabled = settings.optBoolean("cameraAlertsEnabled", true);
            }
        }
        if (!alertsEnabled) {
            MotuCore.pause();
            alert.setText(R.string.alerts_disabled);
        }
    }

    /** Load cameras exported by the web debugger (`/api/export/cameras`). */
    private boolean loadOfflineCameras() {
        File file = OfflineData.camerasFile(this);
        String json = OfflineData.readText(file);
        JSONArray array = OfflineData.parseCameras(json);
        if (array == null || array.length() == 0) {
            return false;
        }
        cameras = array;
            markers.clear();
            for (int index = 0; index < array.length(); index += 1) {
                JSONObject camera = array.optJSONObject(index);
                if (camera == null) {
                    continue;
                }
                JSONObject position = camera.optJSONObject("position");
                if (position == null) {
                    continue;
                }
                JSONArray types = camera.optJSONArray("tabTypes");
                int type = types != null && types.length() > 0 ? types.optInt(0) : 0;
                markers.add(new MapCanvasView.Marker(
                        camera.optString("memberByString", camera.optString("id")),
                        position.optDouble("latitude"),
                        position.optDouble("longitude"),
                        typeName(type),
                        type == 1,
                        true));
            }
        reloadCameras();
        return true;
    }

    private void loadSampleCameras() {
        cameras = new JSONArray();
        markers.clear();
        buildSampleCameras();
        reloadCameras();
    }

    private void buildSampleCameras() {
        addCamera("demo-speed", 120.0, 5, false);
        addCamera("demo-light", 260.0, 4, false);
        addCamera("demo-restricted", 430.0, 1, true);
        addCamera("demo-behind", -80.0, 5, false);
    }

    private void addCamera(String id, double northMeters, int type, boolean restricted) {
        double lat = ORIGIN_LATITUDE + northMeters / 111_320.0;
        JSONObject camera = new JSONObject();
        try {
            JSONObject position = new JSONObject();
            position.put("latitude", lat);
            position.put("longitude", ORIGIN_LONGITUDE);
            camera.put("id", id);
            camera.put("position", position);
            camera.put("tabTypes", new JSONArray().put(type));
            camera.put("memberByString", id);
            camera.put("newFlag", false);
            camera.put("white", false);
            cameras.put(camera);
        } catch (JSONException ignored) {
            // Static, known-good data.
        }
        String label = restricted ? "示例·限行" : "示例·" + typeName(type);
        markers.add(new MapCanvasView.Marker(id, lat, ORIGIN_LONGITUDE, label, restricted, false));
    }

    private void buildRoute() {
        routePlanned = false;
        routeSteps = null;
        lastScanText = "";
        routePoints.add(new double[]{ORIGIN_LATITUDE, ORIGIN_LONGITUDE});
        routePoints.add(new double[]{ORIGIN_LATITUDE + 500.0 / 111_320.0, ORIGIN_LONGITUDE});
    }

    private void reloadCameras() {
        CoreResult result = MotuCore.updateCameras(cameras);
        if (!result.ok) {
            alert.setText(getString(R.string.native_error, result.error));
        }
    }

    private void advance() {
        latitude += 120.0 / 111_320.0;
        if (!alertsEnabled) {
            MotuCore.pause();
            alert.setText(R.string.alerts_disabled);
            render();
            return;
        }
        CoreResult result = MotuCore.scan(latitude, longitude, bearing, true);
        if (!result.ok) {
            alert.setText(getString(R.string.native_error, result.error));
            return;
        }
        JSONObject scan = result.objectData();
        if (scan == null) {
            alert.setText(R.string.no_alert);
        } else {
            alert.setText(formatScan(scan));
        }
        render();
    }

    private void turn() {
        bearing = (bearing + 90f) % 360f;
        // A release tick: scanning is disabled, but passed/turned cameras clear.
        MotuCore.scan(latitude, longitude, bearing, false);
        alert.setText(getString(R.string.turned, (int) bearing));
        render();
    }

    private void audit() {
        JSONArray points = new JSONArray();
        for (double[] point : routePoints) {
            try {
                points.put(new JSONObject()
                        .put("latitude", point[0])
                        .put("longitude", point[1]));
            } catch (JSONException ignored) {
                // Known-good values.
            }
        }
        CoreResult result = MotuCore.auditRoute(points, new JSONArray(), cameras, 50.0);
        JSONObject data = result.objectData();
        if (!result.ok || data == null) {
            alert.setText(getString(R.string.native_error, result.error));
            return;
        }
        JSONArray hits = data.optJSONArray("cameraHits");
        int count = hits == null ? 0 : hits.length();
        alert.setText(getString(R.string.audit_result, count));
    }

    private void reset() {
        MotuCore.resetScan();
        latitude = ORIGIN_LATITUDE;
        longitude = ORIGIN_LONGITUDE;
        bearing = 0f;
        routePlanned = false;
        routeSteps = null;
        lastScanText = "";
        loadSampleCameras();
        alert.setText(R.string.no_alert);
        render();
    }

    /** Plan with the local OSM graph; no network is involved. */
    private void planOfflineRoute() {
        if (graphFile == null || !graphFile.isFile()) {
            alert.setText(R.string.offline_route_missing);
            return;
        }
        boolean avoid = avoidCameras != null && avoidCameras.isChecked();
        CoreResult result = avoid
                ? MotuCore.routeAvoiding(
                        graphFile.getAbsolutePath(), latitude, longitude,
                        DEST_LATITUDE, DEST_LONGITUDE, cameras, 100.0, 0.0, 0.0)
                : MotuCore.route(
                        graphFile.getAbsolutePath(), latitude, longitude,
                        DEST_LATITUDE, DEST_LONGITUDE);
        JSONObject data = result.objectData();
        if (!result.ok || data == null) {
            alert.setText(getString(R.string.native_error, result.error));
            return;
        }
        JSONObject avoidance = data.optJSONObject("avoidance");
        JSONObject plan = data.optJSONObject("plan");
        if (plan != null) {
            data = plan;
        }
        JSONArray points = data.optJSONArray("points");
        routePoints.clear();
        if (points != null) {
            for (int index = 0; index < points.length(); index += 1) {
                JSONObject point = points.optJSONObject(index);
                if (point != null) {
                    routePoints.add(new double[]{
                            point.optDouble("latitude"), point.optDouble("longitude")});
                }
            }
        }
        routePlanned = true;
        routeSteps = data.optJSONArray("steps");
        render();
        double km = data.optDouble("distanceM") / 1000.0;
        double minutes = data.optDouble("durationS") / 60.0;
        String firstStep = "";
        JSONArray steps = data.optJSONArray("steps");
        if (steps != null && steps.length() > 0) {
            firstStep = steps.optJSONObject(0).optString("instruction");
        }
        lastRerouteAt = SystemClock.elapsedRealtime();
        String comparison = "";
        if (avoidance != null) {
            if (avoidance.optBoolean("applied")) {
                comparison = String.format(Locale.ROOT, " · 已知点位 %d→%d，多 %d 分钟",
                        avoidance.optInt("camerasBefore"),
                        avoidance.optInt("camerasAfter"),
                        Math.max(0, Math.round(avoidance.optDouble("extraSeconds") / 60.0)));
                String reason = avoidance.optString("reason").trim();
                if (!reason.isEmpty()) {
                    comparison += "（" + reason + "）";
                }
            } else if (avoidance.optBoolean("rejected")) {
                comparison = " · 保留最快路线（" + avoidance.optString("reason") + "）";
            }
        }
        if (gpsEnabled) {
            updateNavigationAlert();
        } else {
            alert.setText(String.format(Locale.ROOT,
                    getString(R.string.offline_route_result), km, minutes, firstStep)
                    + comparison);
        }
    }

    private void toggleGps() {
        if (CruiseService.isRunning()) {
            stopService(new Intent(this, CruiseService.class));
            gpsEnabled = false;
            syncGpsButton();
            alert.setText(R.string.gps_disabled);
            return;
        }
        if (checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION)
                != PackageManager.PERMISSION_GRANTED) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                requestPermissions(new String[]{
                        Manifest.permission.ACCESS_FINE_LOCATION,
                        Manifest.permission.ACCESS_COARSE_LOCATION,
                        Manifest.permission.POST_NOTIFICATIONS}, REQUEST_LOCATION);
            } else {
                requestPermissions(new String[]{
                        Manifest.permission.ACCESS_FINE_LOCATION,
                        Manifest.permission.ACCESS_COARSE_LOCATION}, REQUEST_LOCATION);
            }
            return;
        }
        startCruiseService();
    }

    private void startCruiseService() {
        Intent intent = new Intent(this, CruiseService.class)
                .setAction(CruiseService.ACTION_START);
        startForegroundService(intent);
        gpsEnabled = true;
        syncGpsButton();
        alert.setText(R.string.gps_enabled);
    }

    private void syncGpsButton() {
        Button button = findViewById(R.id.button_gps);
        if (button != null) {
            button.setText(gpsEnabled ? R.string.gps_stop : R.string.gps_start);
        }
    }

    /** Position/scan updates broadcast by {@link CruiseService}. */
    private void onCruiseUpdate(Intent intent) {
        latitude = intent.getDoubleExtra(CruiseService.EXTRA_LATITUDE, latitude);
        longitude = intent.getDoubleExtra(CruiseService.EXTRA_LONGITUDE, longitude);
        bearing = intent.getFloatExtra(CruiseService.EXTRA_BEARING, bearing);
        String scanJson = intent.getStringExtra(CruiseService.EXTRA_SCAN);
        if (scanJson != null && !scanJson.isEmpty()) {
            try {
                lastScanText = formatScan(new JSONObject(scanJson));
            } catch (JSONException ignored) {
                // Malformed scan payload; keep the previous alert.
            }
        }
        updateNavigationAlert();
        render();
        long now = SystemClock.elapsedRealtime();
        if (routePlanned && !routePoints.isEmpty() && now - lastRerouteAt > 10_000L) {
            lastRerouteAt = now;
            planOfflineRoute();
        }
    }

    /** Next turn + the latest camera alert, based on the local route. */
    private void updateNavigationAlert() {
        String guidance = "";
        if (routePlanned && routeSteps != null && !routePoints.isEmpty()) {
            int index = nearestRouteIndex();
            for (int stepIndex = 0; stepIndex < routeSteps.length(); stepIndex += 1) {
                JSONObject step = routeSteps.optJSONObject(stepIndex);
                if (step == null) {
                    continue;
                }
                int endIndex = step.optInt("endIndex", -1);
                if (!"arrive".equals(step.optString("direction"))
                        && endIndex > index && endIndex < routePoints.size()) {
                    double[] target = routePoints.get(endIndex);
                    double distance = distanceMeters(
                            latitude, longitude, target[0], target[1]);
                    guidance = String.format(Locale.ROOT, "下一步：%s · %d m",
                            step.optString("instruction"), Math.max(1, Math.round(distance)));
                    break;
                }
            }
        }
        if (!guidance.isEmpty() && !lastScanText.isEmpty()) {
            alert.setText(guidance + "\n" + lastScanText);
        } else if (!guidance.isEmpty()) {
            alert.setText(guidance);
        } else if (!lastScanText.isEmpty()) {
            alert.setText(lastScanText);
        }
    }

    private int nearestRouteIndex() {
        int best = 0;
        double bestDistance = Double.MAX_VALUE;
        for (int index = 0; index < routePoints.size(); index += 1) {
            double[] point = routePoints.get(index);
            double distance = distanceMeters(latitude, longitude, point[0], point[1]);
            if (distance < bestDistance) {
                bestDistance = distance;
                best = index;
            }
        }
        return best;
    }

    private static double distanceMeters(double lat1, double lon1, double lat2, double lon2) {
        double radius = 6_371_008.8;
        double phi1 = Math.toRadians(lat1);
        double phi2 = Math.toRadians(lat2);
        double dphi = Math.toRadians(lat2 - lat1);
        double dlambda = Math.toRadians(lon2 - lon1);
        double a = Math.sin(dphi / 2) * Math.sin(dphi / 2)
                + Math.cos(phi1) * Math.cos(phi2)
                * Math.sin(dlambda / 2) * Math.sin(dlambda / 2);
        return 2 * radius * Math.asin(Math.sqrt(a));
    }

    private void syncOfficialCameras() {
        CoreResult session = MotuCore.session();
        JSONObject sessionData = session.objectData();
        if (sessionData == null || sessionData.optString("token").isEmpty()) {
            alert.setText(R.string.sync_official_login_required);
            return;
        }

        syncButton.setEnabled(false);
        alert.setText(R.string.sync_official_in_progress);
        byte[] signingCertificate = SigningCertificate.read(this);
        String cityCode = configuredCityCode();
        double syncLatitude = latitude;
        double syncLongitude = longitude;
        new Thread(() -> {
            CoreResult result = MotuCore.syncOfficialCameras(
                    syncLatitude, syncLongitude, cityCode, signingCertificate);
            runOnUiThread(() -> {
                if (isFinishing()) {
                    return;
                }
                syncButton.setEnabled(true);
                if (!result.ok) {
                    alert.setText(getString(R.string.sync_official_failed, result.error));
                    return;
                }
                JSONObject data = result.objectData();
                JSONArray official = data == null ? null : data.optJSONArray("cameras");
                if (official == null) {
                    alert.setText(R.string.sync_official_empty);
                    return;
                }
                applyOfficialCameras(official);
                alert.setText(getString(R.string.sync_official_result, official.length()));
            });
        }, "motu-official-sync").start();
    }

    private void openCameraDetail(MapCanvasView.Marker marker) {
        if (detailLoading) {
            return;
        }
        JSONObject localDetail = offlineDetails.get(marker.id);
        if (localDetail != null) {
            showCameraDetailDialog(localDetail);
            alert.setText(R.string.camera_detail_loaded);
            return;
        }
        if (!marker.official) {
            alert.setText(marker.id.startsWith("demo-")
                    ? R.string.camera_detail_demo_unavailable
                    : R.string.camera_detail_missing_id);
            return;
        }
        CoreResult session = MotuCore.session();
        JSONObject sessionData = session.objectData();
        if (sessionData == null || sessionData.optString("token").isEmpty()) {
            alert.setText(R.string.camera_detail_login_required);
            return;
        }

        detailLoading = true;
        alert.setText(R.string.camera_detail_loading);
        byte[] signingCertificate = SigningCertificate.read(this);
        new Thread(() -> {
            CoreResult result = MotuCore.loadCameraDetail(marker.id, signingCertificate);
            runOnUiThread(() -> {
                detailLoading = false;
                if (isFinishing()) {
                    return;
                }
                if (!result.ok) {
                    alert.setText(getString(R.string.camera_detail_failed, result.error));
                    return;
                }
                JSONObject data = result.objectData();
                if (data == null) {
                    alert.setText(R.string.camera_detail_empty);
                    return;
                }
                alert.setText(R.string.camera_detail_loaded);
                offlineDetails.put(marker.id, data);
                saveOfflineDetails();
                showCameraDetailDialog(data);
            });
        }, "motu-camera-detail").start();
    }

    private void loadOfflineDetails() {
        String json = OfflineData.readText(OfflineData.detailsFile(this));
        if (json == null) {
            return;
        }
        try {
            JSONObject root = new JSONObject(json);
            JSONObject details = root.optJSONObject("details");
            if (details == null) {
                details = root;
            }
            Iterator<String> keys = details.keys();
            while (keys.hasNext()) {
                String key = keys.next();
                JSONObject detail = details.optJSONObject(key);
                if (detail != null) {
                    offlineDetails.put(key, detail);
                }
            }
        } catch (JSONException error) {
            android.util.Log.w("MotuOffline", "cannot parse details: " + error.getMessage());
        }
    }

    /** Persist opened details so they remain available offline. */
    private void saveOfflineDetails() {
        try {
            JSONObject root = new JSONObject();
            root.put("generatedAtMs", System.currentTimeMillis());
            root.put("count", offlineDetails.size());
            JSONObject details = new JSONObject();
            for (Map.Entry<String, JSONObject> entry : offlineDetails.entrySet()) {
                details.put(entry.getKey(), entry.getValue());
            }
            root.put("details", details);
            OfflineData.writeText(OfflineData.detailsFile(this), root.toString());
        } catch (JSONException error) {
            android.util.Log.w("MotuOffline", "cannot save details: " + error.getMessage());
        }
    }

    private void showCameraDetailDialog(JSONObject data) {
        View view = getLayoutInflater().inflate(R.layout.dialog_camera_detail, null);
        TextView title = view.findViewById(R.id.detail_title);
        TextView types = view.findViewById(R.id.detail_types);
        TextView address = view.findViewById(R.id.detail_address);
        TextView rules = view.findViewById(R.id.detail_rules);
        TextView detail = view.findViewById(R.id.detail_text);
        TextView ticketsHeader = view.findViewById(R.id.detail_tickets_header);
        TextView tickets = view.findViewById(R.id.detail_tickets);

        String titleText = data.optString("title").trim();
        bindText(title, titleText.isEmpty()
                ? getString(R.string.camera_detail_title)
                : titleText);
        bindText(types, formatTypes(data.optJSONArray("types")));
        bindText(address, data.optString("address"));
        bindText(rules, formatRules(data.optJSONArray("rules")));
        bindText(detail, data.optString("detail"));

        int ticketCount = data.optInt("ticketsCount", 0);
        ticketsHeader.setText(getString(R.string.camera_detail_tickets_header, ticketCount));
        tickets.setText(formatTickets(data.optJSONArray("tickets")));

        new AlertDialog.Builder(this)
                .setView(view)
                .setPositiveButton(R.string.camera_detail_close, null)
                .show();
    }

    private static void bindText(TextView view, String text) {
        String trimmed = text == null ? "" : text.trim();
        if (trimmed.isEmpty()) {
            view.setVisibility(View.GONE);
        } else {
            view.setVisibility(View.VISIBLE);
            view.setText(trimmed);
        }
    }

    private String formatTypes(JSONArray types) {
        if (types == null) {
            return "";
        }
        StringBuilder builder = new StringBuilder();
        for (int i = 0; i < types.length(); i++) {
            JSONObject type = types.optJSONObject(i);
            if (type == null) {
                continue;
            }
            String name = type.optString("name").trim();
            String remark = type.optString("remark").trim();
            if (builder.length() > 0) {
                builder.append('\n');
            }
            builder.append(name.isEmpty() ? getString(R.string.camera_detail_title) : name);
            if (!remark.isEmpty()) {
                builder.append(" · ").append(remark);
            }
        }
        return builder.toString();
    }

    private String formatRules(JSONArray rules) {
        if (rules == null) {
            return "";
        }
        StringBuilder builder = new StringBuilder();
        for (int i = 0; i < rules.length(); i++) {
            JSONObject rule = rules.optJSONObject(i);
            if (rule == null) {
                continue;
            }
            StringBuilder line = new StringBuilder();
            String plate = rule.optString("plateName").trim();
            if (!plate.isEmpty()) {
                line.append(getString(R.string.camera_detail_rule_plate, plate));
            }
            JSONArray days = rule.optJSONArray("dayTypeNames");
            if (days != null && days.length() > 0) {
                appendLine(line, getString(R.string.camera_detail_rule_days, join(days, "、")));
            }
            JSONArray tail = rule.optJSONArray("tail");
            if (tail != null && tail.length() > 0) {
                appendLine(line, getString(R.string.camera_detail_rule_tail, joinNumbers(tail)));
            }
            String time = rule.optString("time").trim();
            if (!time.isEmpty()) {
                appendLine(line, getString(R.string.camera_detail_rule_time, time));
            }
            if (line.length() == 0) {
                continue;
            }
            if (builder.length() > 0) {
                builder.append("\n\n");
            }
            builder.append(line);
        }
        return builder.toString();
    }

    private static void appendLine(StringBuilder builder, String line) {
        if (builder.length() > 0) {
            builder.append('\n');
        }
        builder.append(line);
    }

    private String formatTickets(JSONArray tickets) {
        if (tickets == null || tickets.length() == 0) {
            return getString(R.string.camera_detail_no_tickets);
        }
        SimpleDateFormat format = new SimpleDateFormat("yyyy.MM.dd HH:mm", Locale.CHINA);
        StringBuilder builder = new StringBuilder();
        for (int i = 0; i < tickets.length(); i++) {
            JSONObject ticket = tickets.optJSONObject(i);
            if (ticket == null) {
                continue;
            }
            if (builder.length() > 0) {
                builder.append("\n\n");
            }
            String title = ticket.optString("title").trim();
            builder.append(i + 1).append(". ").append(
                    title.isEmpty() ? getString(R.string.camera_detail_ticket_unknown) : title);
            String remark = ticket.optString("remark").trim();
            if (!remark.isEmpty()) {
                builder.append('\n').append(remark);
            }
            long createDate = ticket.optLong("createDate", 0L);
            if (createDate > 0) {
                builder.append('\n').append(format.format(new Date(createDate)));
            }
            if (!ticket.optString("photo").trim().isEmpty()) {
                builder.append('\n').append(getString(R.string.camera_detail_ticket_photo));
            }
        }
        return builder.toString();
    }

    private static String join(JSONArray values, String separator) {
        StringBuilder builder = new StringBuilder();
        for (int i = 0; i < values.length(); i++) {
            if (i > 0) {
                builder.append(separator);
            }
            builder.append(values.optString(i));
        }
        return builder.toString();
    }

    private static String joinNumbers(JSONArray values) {
        StringBuilder builder = new StringBuilder();
        for (int i = 0; i < values.length(); i++) {
            if (i > 0) {
                builder.append(", ");
            }
            builder.append(values.optInt(i));
        }
        return builder.toString();
    }

    private String configuredCityCode() {
        CoreResult state = MotuCore.loadState();
        JSONObject data = state.objectData();
        JSONObject settings = data == null ? null : data.optJSONObject("settings");
        if (settings != null) {
            String configured = settings.optString("cityCode").trim();
            if (!configured.isEmpty()) {
                return configured;
            }
        }
        return OFFICIAL_CITY_CODE;
    }

    private void applyOfficialCameras(JSONArray official) {
        cameras = official;
        markers.clear();
        int limit = Math.min(official.length(), MAX_OFFICIAL_MARKERS);
        for (int i = 0; i < limit; i++) {
            JSONObject camera = official.optJSONObject(i);
            if (camera == null) {
                continue;
            }
            JSONObject position = camera.optJSONObject("position");
            if (position == null) {
                continue;
            }
            double lat = position.optDouble("latitude", Double.NaN);
            double lon = position.optDouble("longitude", Double.NaN);
            if (!Double.isFinite(lat) || !Double.isFinite(lon)) {
                continue;
            }
            JSONArray tabTypes = camera.optJSONArray("tabTypes");
            int type = tabTypes == null || tabTypes.length() == 0 ? 0 : tabTypes.optInt(0);
            String label = "官方·" + typeName(type);
            if (camera.optBoolean("newFlag")) {
                label += "·新";
            }
            // `memberByString` is the id expected by trafficEyesDetailSec;
            // the synthetic id is not a valid detail id. Rust omits the field
            // when absent, and isNull covers both that and a JSON null.
            String cameraId = camera.isNull("memberByString")
                    ? ""
                    : camera.optString("memberByString").trim();
            markers.add(new MapCanvasView.Marker(
                    cameraId, lat, lon, label, type == 1, !cameraId.isEmpty()));
        }
        // The FFI already replaced the scan-engine set; this keeps the demo
        // path and the official path using the same bridge call.
        MotuCore.updateCameras(cameras);
        MotuCore.pause();
        render();
    }

    private String formatScan(JSONObject scan) {
        JSONArray types = scan.optJSONArray("cameraTypes");
        StringBuilder builder = new StringBuilder();
        builder.append("提醒：");
        if (types != null) {
            for (int i = 0; i < types.length(); i++) {
                if (i > 0) {
                    builder.append(" + ");
                }
                builder.append(typeName(types.optInt(i)));
            }
        }
        builder.append(" · ").append(scan.optInt("primaryDistanceM")).append(" m");
        if (scan.optBoolean("clustered")) {
            builder.append(" · 密集");
        }
        return builder.toString();
    }

    private static String typeName(int type) {
        switch (type) {
            case 1:
                return "闯限行";
            case 2:
                return "应急车道";
            case 3:
                return "非机动车道";
            case 4:
                return "闯红灯";
            case 5:
                return "超速";
            case 6:
                return "公交车道";
            case 7:
                return "车道违规";
            case 8:
                return "逆行";
            case 9:
                return "尾号限行";
            case 10:
                return "未戴头盔";
            case 11:
                return "噪音";
            case 12:
                return "现场罚单";
            case 13:
                return "压线";
            case 14:
                return "违规用灯";
            default:
                return "未知";
        }
    }

    private void render() {
        map.setData(routePoints, markers, latitude, longitude, bearing);
        TextView position = findViewById(R.id.text_position);
        position.setText(String.format(Locale.ROOT,
                "位置 %.6f, %.6f · 朝向 %d°", latitude, longitude, (int) bearing));
    }
}
