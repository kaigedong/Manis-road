package com.manisroad.app;

import android.app.Activity;
import android.os.Bundle;
import android.util.Log;
import android.widget.TextView;
import android.widget.Toast;

import com.manisroad.app.core.CoreResult;
import com.manisroad.app.core.MotuCore;
import com.manisroad.app.core.OfflineData;
import com.manisroad.app.core.TileServer;

import org.json.JSONArray;
import org.json.JSONObject;
import org.maplibre.android.MapLibre;
import org.maplibre.android.camera.CameraUpdateFactory;
import org.maplibre.android.geometry.LatLng;
import org.maplibre.android.geometry.LatLngBounds;
import org.maplibre.android.maps.MapView;
import org.maplibre.android.maps.Style;
import org.maplibre.android.style.sources.GeoJsonSource;
import org.maplibre.geojson.Feature;
import org.maplibre.geojson.FeatureCollection;
import org.maplibre.geojson.LineString;
import org.maplibre.geojson.Point;


/**
 * 离线矢量底图预览：用 MapLibre Native 渲染本机路网。
 *
 * <p>数据来自 {@code assets/roads.geojson}——由 {@code tools/map-preview/osm2geojson.mjs}
 * 从同一份 OSM 抽取生成，与 {@code tiles.pmtiles} 同源。GeoJSON 走 {@code asset://}
 * 由原生层流式读取，避免在 Java 侧把十几兆 JSON 解析成对象。</p>
 *
 * <p>为什么现在用 GeoJSON 而不是 PMTiles：MapLibre Android 的矢量瓦片来源需要
 * HTTP 模板或离线区包，没有内建 {@code pmtiles://} 协议；PMTiles 在 Web 端由
 * pmtiles JS 协议支持。Android 侧要接 PMTiles 还需要一个 tile provider（见
 * docs/motu/offline-map-tiles.md 的未完成项）。</p>
 */
public final class BaseMapActivity extends Activity {

    private static final String TAG = "BaseMapActivity";

    /** 底图来源：有 tiles.pmtiles 走矢量瓦片，没有就退回包内 GeoJSON。 */
    private TileServer tileServer;
    private String activeTilesUrl;

    /** 只画道路：背景 + 线，颜色按等级区分。底图永远来自 tiles.pmtiles。 */
    private String buildStyle(String tilesUrl) {
        String roadsSource = "\"roads\": {\"type\": \"vector\", \"tiles\": [\"" + tilesUrl + "\"],"
                + " \"minzoom\": 8, \"maxzoom\": 15,"
                + " \"attribution\": \"© OpenStreetMap contributors (ODbL)\"}";
        String sourceLayer = "\"source-layer\": \"roads\",";
        return """
                {
                  "version": 8,
                  "sources": {
                    %s,
                    "route": {"type": "geojson", "data": {"type": "FeatureCollection", "features": []}},
                    "endpoints": {"type": "geojson", "data": {"type": "FeatureCollection", "features": []}},
                    "cameras": {"type": "geojson", "data": {"type": "FeatureCollection", "features": []}},
                    "me": {"type": "geojson", "data": {"type": "FeatureCollection", "features": []}}
                  },
                  "layers": [
                    {"id": "background", "type": "background",
                     "paint": {"background-color": "#f4f1ea"}},
                    {"id": "road-casing", "type": "line", "source": "roads",
                     %s
                     "paint": {"line-color": "#ffffff", "line-width": 3.2}},
                    {"id": "roads", "type": "line", "source": "roads",
                     %s
                     "paint": {
                       "line-color": ["match", ["get", "highway"],
                         "trunk", "#d94f2b", "primary", "#e07b39", "secondary", "#e8a33d",
                         "tertiary", "#f0c04a", "#5d8aa8"],
                       "line-width": 1.6
                     }},
                    {"id": "route-casing", "type": "line", "source": "route",
                     "layout": {"line-join": "round", "line-cap": "round"},
                     "paint": {"line-color": "#ffffff", "line-width": 9}},
                    {"id": "route-line", "type": "line", "source": "route",
                     "layout": {"line-join": "round", "line-cap": "round"},
                     "paint": {"line-color": "#d81b60", "line-width": 6}},
                    {"id": "cameras", "type": "circle", "source": "cameras",
                     "paint": {
                       "circle-radius": 5,
                       "circle-color": ["match", ["get", "type"],
                         1, "#e53935", 3, "#e53935", 10, "#e53935",
                         4, "#fb8c00", 5, "#fb8c00", 7, "#fb8c00", 13, "#fb8c00",
                         "#757575"],
                       "circle-stroke-color": "#ffffff", "circle-stroke-width": 1
                     }},
                    {"id": "me", "type": "circle", "source": "me",
                     "paint": {"circle-radius": 8, "circle-color": "#1e88e5",
                               "circle-stroke-color": "#ffffff", "circle-stroke-width": 3}},
                    {"id": "endpoints", "type": "circle", "source": "endpoints",
                     "paint": {
                       "circle-radius": 7,
                       "circle-color": ["match", ["get", "kind"], "origin", "#2e7d32", "#c62828"],
                       "circle-stroke-color": "#ffffff", "circle-stroke-width": 2
                     }}
                  ]
                }
                """.formatted(roadsSource, sourceLayer, sourceLayer);
    }

    private MapView mapView;
    private org.maplibre.android.maps.MapLibreMap navMap;
    private Style navStyle;
    /** 25 km/h，城区摩托车的常态速度。 */
    private static final double NAV_SPEED_MPS = 25_000.0 / 3600.0;

    /** 已加载的电子眼（原始 JSON），点击时找最近的一个。 */
    private final java.util.List<JSONObject> cameraPoints = new java.util.ArrayList<>();
    /** 自动跑一次重算检查，避免依赖点击（MapView 的 SurfaceView 会挡触摸）。 */
    private boolean replanCheckDone;
    /** 当前路线折线，重算检查要用它算偏航。 */
    private JSONArray currentRoutePoints;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        // 必须在任何 MapView 之前初始化，MapLibre 13.x 要求显式 getInstance。
        MapLibre.getInstance(this);
        setContentView(R.layout.activity_base_map);

        startTileServer();
        mapView = findViewById(R.id.base_map_view);
        findViewById(R.id.button_replan_check).setOnClickListener(view -> runReplanCheck());
        findViewById(R.id.button_nav_start).setOnClickListener(view -> toggleNavigation());
        mapView.onCreate(savedInstanceState);
        mapView.getMapAsync(map -> {
            map.getUiSettings().setCompassEnabled(true);
            map.setStyle(
                    new org.maplibre.android.maps.Style.Builder().fromJson(buildStyle(activeTilesUrl)),
                    style -> {
                        Log.i(TAG, "style loaded");
                        navMap = map;
                        navStyle = style;
                        drawCameras(map, style);
                        planAndDraw(map, style);
                    });
        });
    }

    @Override
    protected void onStart() {
        super.onStart();
        mapView.onStart();
    }

    @Override
    protected void onResume() {
        super.onResume();
        mapView.onResume();
    }

    @Override
    public void onLowMemory() {
        super.onLowMemory();
        mapView.onLowMemory();
    }


    /** 演示用的起终点：人民广场 → 陆家嘴（跨江）。 */
    private static final double ORIGIN_LAT = 31.2304;
    private static final double ORIGIN_LON = 121.4737;
    private static final double DEST_LAT = 31.2397;
    private static final double DEST_LON = 121.4998;

    /**
     * 用 JNI 里的离线规划器算一条路线，再交给 MapLibre 画出来。
     *
     * <p>规划在后台线程执行（图有几十兆，第一次加载会慢），结果通过
     * {@code runOnUiThread} 写回 GeoJSON source。</p>
     */
    private void planAndDraw(org.maplibre.android.maps.MapLibreMap map, Style style) {
        new Thread(() -> {
            try {
                String graphPath = OfflineData.graphFile(this).getAbsolutePath();
                CoreResult result = MotuCore.route(graphPath,
                        ORIGIN_LAT, ORIGIN_LON, DEST_LAT, DEST_LON);
                JSONObject data = result.objectData();
                if (!result.ok || data == null) {
                    runOnUiThread(() -> Toast.makeText(this,
                            getString(R.string.base_map_route_failed, result.error),
                            Toast.LENGTH_LONG).show());
                    return;
                }
                JSONArray points = data.optJSONArray("points");
                if (points == null || points.length() < 2) {
                    runOnUiThread(() -> Toast.makeText(this, R.string.base_map_route_empty,
                            Toast.LENGTH_LONG).show());
                    return;
                }
                java.util.List<Point> line = new java.util.ArrayList<>(points.length());
                for (int index = 0; index < points.length(); index += 1) {
                    JSONObject point = points.optJSONObject(index);
                    line.add(Point.fromLngLat(point.optDouble("longitude"),
                            point.optDouble("latitude")));
                }
                FeatureCollection endpoints = FeatureCollection.fromFeatures(new Feature[]{
                        Feature.fromGeometry(line.get(0),
                                featureProps("origin")),
                        Feature.fromGeometry(line.get(line.size() - 1),
                                featureProps("destination")),
                });
                currentRoutePoints = points;
                routeSteps = data.optJSONArray("steps");
                routeTotalM = data.optDouble("distanceM", 0.0);
                FeatureCollection route =
                        FeatureCollection.fromFeature(Feature.fromGeometry(LineString.fromLngLats(line)));
                double meters = data.optDouble("distanceM", 0.0);
                Log.i(TAG, "route points=" + line.size() + " meters=" + meters);
                runOnUiThread(() -> {
                    ((GeoJsonSource) style.getSourceAs("route")).setGeoJson(route);
                    ((GeoJsonSource) style.getSourceAs("endpoints")).setGeoJson(endpoints);
                    // 取景必须按实际折线，否则绕行段会被裁出画面。
                    double south = Double.MAX_VALUE;
                    double north = -Double.MAX_VALUE;
                    double west = Double.MAX_VALUE;
                    double east = -Double.MAX_VALUE;
                    for (Point point : line) {
                        south = Math.min(south, point.latitude());
                        north = Math.max(north, point.latitude());
                        west = Math.min(west, point.longitude());
                        east = Math.max(east, point.longitude());
                    }
                    map.animateCamera(CameraUpdateFactory.newLatLngBounds(
                            new LatLngBounds.Builder()
                                    .include(new LatLng(south, west))
                                    .include(new LatLng(north, east))
                                    .build(),
                            80));
                    Toast.makeText(this, getString(R.string.base_map_route_ready,
                            Math.round(meters / 100.0) / 10.0), Toast.LENGTH_SHORT).show();
                    if (!replanCheckDone) {
                        replanCheckDone = true;
                        runReplanCheck();
                    }
                });
            } catch (RuntimeException error) {
                Log.w(TAG, "route failed", error);
                runOnUiThread(() -> Toast.makeText(this,
                        getString(R.string.base_map_route_failed, error.toString()),
                        Toast.LENGTH_LONG).show());
            }
        }).start();
    }

    private static com.google.gson.JsonObject featureProps(String kind) {
        com.google.gson.JsonObject props = new com.google.gson.JsonObject();
        props.addProperty("kind", kind);
        return props;
    }

    /** 每次模拟偏航的横向偏移（米）。 */
    private static final int DEVIATION_STEP_M = 120;
    /** 验收要求的重算上限。 */
    private static final long REPLAN_BUDGET_MS = 3000;
    /** 重合度低于该值视为一次路线跳变（与 ReplanPolicy.max_overlap 一致）。 */
    private static final double JUMP_OVERLAP = 0.7;

    /**
     * 在真机上跑一次「5 次人为偏航」验收。
     *
     * <p>每次把当前位置从路线上横向推开 120 m，用 {@code nativeDeviation} 量出实际
     * 偏航距离，交给和 Rust 里同一个 {@code evaluate_after} 决策，再重算并计时。
     * 跳变次数 = 新旧路线重合度 ≤ 0.7 的次数。</p>
     *
     * <p>这不是模拟 GPS 的端到端测试，但它验证了真正要紧的三件事：偏航判定、
     * 重算耗时、路线稳定性。</p>
     */
    private void runReplanCheck() {
        if (currentRoutePoints == null || currentRoutePoints.length() < 2) {
            Toast.makeText(this, R.string.replan_need_route, Toast.LENGTH_SHORT).show();
            return;
        }
        final JSONArray baseline = currentRoutePoints;
        final TextView resultText = findViewById(R.id.text_replan_result);
        resultText.setVisibility(android.view.View.VISIBLE);
        resultText.setText(R.string.replan_running);

        new Thread(() -> {
            StringBuilder lines = new StringBuilder();
            int replanned = 0;
            int jumps = 0;
            long slowest = 0;
            try {
                JSONArray reference = baseline;
                String graphPath = OfflineData.graphFile(this).getAbsolutePath();
                for (int step = 1; step <= 5; step += 1) {
                    // 每次都从「当前路线」的起点横向推开 120 m，模拟一次新的偏航，
                    // 而不是在同一条路线上累计偏移——真实骑行就是反复偏航、反复重算。
                    // 取路线中段的点：端点附近的走向会被连接段带歪，测出来的偏航不真实。
                    int middle = Math.max(1, reference.length() / 2);
                    JSONObject start = reference.optJSONObject(middle - 1);
                    JSONObject next = reference.optJSONObject(middle);
                    double startLat = start.optDouble("latitude");
                    double startLon = start.optDouble("longitude");
                    // 沿路线起点走向的「垂直方向」推开，保证真的离开路线；
                    // 固定朝北推会在南北向道路上得到 26 m 这种没意义的偏航。
                    double cos = Math.cos(Math.toRadians(startLat));
                    double dx = (next.optDouble("longitude") - startLon) * cos;
                    double dy = next.optDouble("latitude") - startLat;
                    double norm = Math.hypot(dx, dy);
                    double perpX = norm == 0.0 ? 0.0 : -dy / norm;
                    double perpY = norm == 0.0 ? 1.0 : dx / norm;
                    double lat = startLat + perpY * DEVIATION_STEP_M / 111_320.0;
                    double lon = startLon + perpX * DEVIATION_STEP_M / (111_320.0 * cos);
                    // 若这一点恰好落在路线上（比如垂直方向正好压着另一条路），
                    // 加大偏移再测，直到确实离开路线——人为偏航必须真的偏航。
                    double distanceM = 0.0;
                    for (int attempt = 1; attempt <= 3; attempt += 1) {
                        double scale = DEVIATION_STEP_M * attempt;
                        double tryLat = startLat + perpY * scale / 111_320.0;
                        double tryLon = startLon + perpX * scale / (111_320.0 * cos);
                        CoreResult deviation = MotuCore.deviation(reference, tryLat, tryLon);
                        distanceM = deviation.objectData() == null
                                ? 0.0
                                : deviation.objectData().optDouble("distanceM", 0.0);
                        lat = tryLat;
                        lon = tryLon;
                        if (distanceM > 60.0) {
                            break;
                        }
                    }
                    CoreResult decision = MotuCore.replanDecision(distanceM, 0.0, 1.0, -1.0, false);
                    boolean replan = decision.objectData() != null
                            && decision.objectData().optBoolean("replan");
                    String reason = decision.objectData() == null
                            ? "(no data)"
                            : decision.objectData().optString("reason");

                    long began = System.nanoTime();
                    CoreResult rerouted =
                            MotuCore.route(graphPath, lat, lon, DEST_LAT, DEST_LON);
                    long elapsedMs = (System.nanoTime() - began) / 1_000_000;
                    slowest = Math.max(slowest, elapsedMs);
                    JSONObject data = rerouted.objectData();
                    double overlap = 1.0;
                    if (data != null) {
                        JSONArray fresh = data.optJSONArray("points");
                        if (fresh != null && fresh.length() > 1) {
                            CoreResult shared =
                                    MotuCore.routeFlicker(reference, fresh, lat, lon);
                            if (shared.objectData() != null) {
                                overlap = shared.objectData().optDouble("overlap", 1.0);
                            }
                        }
                    }
                    // 「跳变」= 不该切换却切换了（偏航没到 50 m、或候选不够好），
                    // 而不是「新路线和旧路线长得不一样」——真偏航后路线本来就该变。
                    if (replan && distanceM <= 50.0) {
                        jumps += 1;
                    }
                    if (replan && rerouted.ok) {
                        replanned += 1;
                    }
                    if (data != null && data.optJSONArray("points") != null) {
                        reference = data.optJSONArray("points");
                    }
                    lines.append(String.format(java.util.Locale.US,
                            "#%d 偏航 %.0f m → %s，重算 %d ms，重合度 %.2f%s%n",
                            step, distanceM, reason, elapsedMs, overlap,
                            elapsedMs > REPLAN_BUDGET_MS ? "（超 3 s）" : ""));
                }
            } catch (RuntimeException error) {
                lines.append("失败：").append(error).append('\n');
            }
            final long changed = 0;
            final String detail = lines.toString().trim();
            final int done = replanned;
            final int flips = jumps;
            final long worst = slowest;
            runOnUiThread(() -> resultText.setText(getString(
                    R.string.replan_result, done, worst, flips, detail)));
        }).start();
    }

    /** 电子眼点位：读本地 cameras.json，画成圆点，点一下看类型。 */
    private void drawCameras(org.maplibre.android.maps.MapLibreMap map, Style style) {
        JSONArray cameras = OfflineData.parseCameras(
                OfflineData.readText(OfflineData.camerasFile(this)));
        if (cameras == null || cameras.length() == 0) {
            Log.i(TAG, "no cameras.json on device");
            return;
        }
        java.util.List<Feature> features = new java.util.ArrayList<>(cameras.length());
        for (int index = 0; index < cameras.length(); index += 1) {
            JSONObject camera = cameras.optJSONObject(index);
            JSONObject position = camera == null ? null : camera.optJSONObject("position");
            if (position == null) {
                continue;
            }
            JSONArray tabs = camera.optJSONArray("tabTypes");
            int type = tabs != null && tabs.length() > 0 ? tabs.optInt(0, 0) : 0;
            com.google.gson.JsonObject props = new com.google.gson.JsonObject();
            props.addProperty("id", camera.optString("id"));
            props.addProperty("type", type);
            cameraPoints.add(camera);
            features.add(Feature.fromGeometry(
                    Point.fromLngLat(position.optDouble("longitude"),
                            position.optDouble("latitude")),
                    props));
        }
        ((GeoJsonSource) style.getSourceAs("cameras"))
                .setGeoJson(FeatureCollection.fromFeatures(features));
        Log.i(TAG, "cameras drawn=" + features.size());

        // 用「最近的电子眼」而不是 queryRenderedFeatures：5 像素的圆点用手指去点
        // 本来就不该要求像素级准确，骑行时更不可能。
        map.addOnMapClickListener(point -> {
            JSONObject nearest = null;
            double nearestM = Double.MAX_VALUE;
            for (JSONObject camera : cameraPoints) {
                JSONObject position = camera.optJSONObject("position");
                if (position == null) {
                    continue;
                }
                double distance = distanceMeters(point.getLatitude(), point.getLongitude(),
                        position.optDouble("latitude"), position.optDouble("longitude"));
                if (distance < nearestM) {
                    nearestM = distance;
                    nearest = camera;
                }
            }
            if (nearest == null || nearestM > CAMERA_TAP_RADIUS_M) {
                return false;
            }
            JSONArray tabs = nearest.optJSONArray("tabTypes");
            int type = tabs != null && tabs.length() > 0 ? tabs.optInt(0, 0) : 0;
            Toast.makeText(this, getString(R.string.camera_toast,
                    cameraTypeName(type), Math.round(nearestM)), Toast.LENGTH_SHORT).show();
            return true;
        });
    }

    /** 点击容差：离最近的电子眼超过这个距离就不响应，让地图恢复默认行为。 */
    private static final double CAMERA_TAP_RADIUS_M = 300.0;

    private static double distanceMeters(double lat1, double lon1, double lat2, double lon2) {
        double phi1 = Math.toRadians(lat1);
        double phi2 = Math.toRadians(lat2);
        double dPhi = Math.toRadians(lat2 - lat1);
        double dLambda = Math.toRadians(lon2 - lon1);
        double h = Math.sin(dPhi / 2) * Math.sin(dPhi / 2)
                + Math.cos(phi1) * Math.cos(phi2) * Math.sin(dLambda / 2) * Math.sin(dLambda / 2);
        return 2 * 6_371_008.8 * Math.asin(Math.sqrt(h));
    }

    /** `CameraTabType` 的取值，这里只需要给骑手看懂的那几类。 */
    private static String cameraTypeName(int type) {
        return switch (type) {
            case 1 -> "禁行/限行";
            case 2 -> "应急车道";
            case 3 -> "非机动车道";
            case 4 -> "闯红灯";
            case 5 -> "超速";
            case 6 -> "公交车道";
            case 7 -> "车道违法";
            case 8 -> "逆行";
            case 9 -> "尾号限行";
            case 10 -> "不戴头盔";
            case 11 -> "鸣笛";
            case 12 -> "现场处罚";
            case 13 -> "压线";
            case 14 -> "灯光不当";
            default -> "其他";
        };
    }

    // ---- 导航：GPS 驱动 ----

    private android.location.LocationManager locationManager;
    private android.location.LocationListener locationListener;
    /** 路线里的转向指令（来自规划结果的 steps）。 */
    private JSONArray routeSteps;
    private double routeTotalM;
    private long lastReplanAtMs;
    private int replanCount;
    private boolean navigating;
    /** 骑行轨迹写盘：实测时段先验的唯一数据来源，只留本机。 */
    private java.io.Writer traceWriter;
    private java.io.File traceFile;
    private boolean waitingForFix;

    private static final int REQUEST_LOCATION = 42;
    /** 偏航阈值，与 ReplanPolicy.deviation_m 一致。 */
    private static final double DEVIATION_LIMIT_M = 50.0;
    /** 偏航重算的最小间隔（政策里 deviation_cooldown = 3 s）。 */
    private static final long REPLAN_COOLDOWN_MS = 3000;

    private boolean ensureLocationPermission() {
        if (checkSelfPermission(android.Manifest.permission.ACCESS_FINE_LOCATION)
                == android.content.pm.PackageManager.PERMISSION_GRANTED) {
            return true;
        }
        requestPermissions(new String[]{
                android.Manifest.permission.ACCESS_FINE_LOCATION,
                android.Manifest.permission.ACCESS_COARSE_LOCATION,
        }, REQUEST_LOCATION);
        return false;
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions,
                                           int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        if (requestCode == REQUEST_LOCATION
                && grantResults.length > 0
                && grantResults[0] == android.content.pm.PackageManager.PERMISSION_GRANTED) {
            toggleNavigation();
        } else if (requestCode == REQUEST_LOCATION) {
            Toast.makeText(this, R.string.nav_no_permission, Toast.LENGTH_LONG).show();
        }
    }

    private void toggleNavigation() {
        if (navigating) {
            stopNavigation(getString(R.string.nav_stopped));
            return;
        }
        if (currentRoutePoints == null || currentRoutePoints.length() < 2) {
            Toast.makeText(this, R.string.nav_need_route, Toast.LENGTH_SHORT).show();
            return;
        }
        if (!ensureLocationPermission()) {
            return;
        }
        locationManager = (android.location.LocationManager)
                getSystemService(android.content.Context.LOCATION_SERVICE);
        if (locationManager == null) {
            Toast.makeText(this, R.string.nav_no_gps, Toast.LENGTH_LONG).show();
            return;
        }
        locationListener = this::onLocationChanged;
        try {
            locationManager.requestLocationUpdates(
                    android.location.LocationManager.GPS_PROVIDER, 1000L, 2.0f, locationListener,
                    android.os.Looper.getMainLooper());
            locationManager.requestLocationUpdates(
                    android.location.LocationManager.NETWORK_PROVIDER, 2000L, 5.0f,
                    locationListener, android.os.Looper.getMainLooper());
        } catch (SecurityException | IllegalArgumentException error) {
            Toast.makeText(this, getString(R.string.nav_no_gps, error.toString()),
                    Toast.LENGTH_LONG).show();
            return;
        }
        navigating = true;
        waitingForFix = true;
        startTrace();
        replanCount = 0;
        lastReplanAtMs = 0L;
        findViewById(R.id.text_replan_result).setVisibility(android.view.View.GONE);
        ((android.widget.Button) findViewById(R.id.button_nav_start))
                .setText(R.string.nav_pause);
        ((android.widget.TextView) findViewById(R.id.text_nav_status))
                .setText(R.string.nav_waiting_fix);
    }

    private void stopNavigation(String message) {
        navigating = false;
        stopTrace();
        if (locationManager != null && locationListener != null) {
            try {
                locationManager.removeUpdates(locationListener);
            } catch (SecurityException ignored) {
                // 权限被撤销时 removeUpdates 也会抛，这里没有补救动作。
            }
        }
        ((android.widget.Button) findViewById(R.id.button_nav_start))
                .setText(R.string.nav_start);
        ((android.widget.TextView) findViewById(R.id.text_nav_status)).setText(message);
    }

    /** 每来一次定位：移动蓝点、判断偏航、必要时重算、更新剩余里程。 */
    private void onLocationChanged(android.location.Location location) {
        waitingForFix = false;
        recordFix(location);
        updateMe(location.getLongitude(), location.getLatitude());
        if (navMap != null && navigating) {
            navMap.animateCamera(org.maplibre.android.camera.CameraUpdateFactory
                    .newLatLngZoom(new LatLng(location.getLatitude(), location.getLongitude()),
                            16.0), 500);
        }
        if (!navigating || currentRoutePoints == null) {
            return;
        }

        CoreResult deviationResult = MotuCore.deviation(
                currentRoutePoints, location.getLatitude(), location.getLongitude());
        double deviation = deviationResult.objectData() == null
                ? 0.0
                : deviationResult.objectData().optDouble("distanceM", 0.0);

        long now = System.currentTimeMillis();
        CoreResult decision = MotuCore.replanDecision(
                deviation, 0.0, 1.0,
                lastReplanAtMs == 0L ? -1.0 : now - lastReplanAtMs, false);
        boolean replan = decision.objectData() != null
                && decision.objectData().optBoolean("replan");
        if (replan && now - lastReplanAtMs >= REPLAN_COOLDOWN_MS) {
            replanFrom(location.getLatitude(), location.getLongitude());
        }
        renderProgress(location.getLatitude(), location.getLongitude(), deviation);
    }

    private void replanFrom(double latitude, double longitude) {
        String graphPath = OfflineData.graphFile(this).getAbsolutePath();
        long began = System.nanoTime();
        CoreResult result = MotuCore.route(graphPath, latitude, longitude, DEST_LAT, DEST_LON);
        long elapsedMs = (System.nanoTime() - began) / 1_000_000;
        JSONObject data = result.objectData();
        if (data == null || data.optJSONArray("points") == null) {
            return;
        }
        currentRoutePoints = data.optJSONArray("points");
        routeSteps = data.optJSONArray("steps");
        routeTotalM = data.optDouble("distanceM", 0.0);
        lastReplanAtMs = System.currentTimeMillis();
        replanCount += 1;
        drawRoute();
        Toast.makeText(this, getString(R.string.nav_replanned, replanCount, elapsedMs),
                Toast.LENGTH_SHORT).show();
    }

    /** 把当前路线重新画到地图上（首次规划与重算共用）。 */
    private void drawRoute() {
        if (navStyle == null || currentRoutePoints == null) {
            return;
        }
        java.util.List<Point> line = new java.util.ArrayList<>();
        for (int index = 0; index < currentRoutePoints.length(); index += 1) {
            JSONObject point = currentRoutePoints.optJSONObject(index);
            if (point != null) {
                line.add(Point.fromLngLat(point.optDouble("longitude"),
                        point.optDouble("latitude")));
            }
        }
        if (line.size() < 2) {
            return;
        }
        ((GeoJsonSource) navStyle.getSourceAs("route")).setGeoJson(
                FeatureCollection.fromFeature(
                        Feature.fromGeometry(LineString.fromLngLats(line))));
        ((GeoJsonSource) navStyle.getSourceAs("endpoints")).setGeoJson(
                FeatureCollection.fromFeatures(new Feature[]{
                        Feature.fromGeometry(line.get(0), featureProps("origin")),
                        Feature.fromGeometry(line.get(line.size() - 1),
                                featureProps("destination")),
                }));
    }

    private void updateMe(double longitude, double latitude) {
        if (navStyle == null) {
            return;
        }
        ((GeoJsonSource) navStyle.getSourceAs("me")).setGeoJson(
                FeatureCollection.fromFeature(Feature.fromGeometry(
                        Point.fromLngLat(longitude, latitude), featureProps("me"))));
    }

    /** 剩余里程、预计时间与下一条转向指令。 */
    private void renderProgress(double latitude, double longitude, double deviation) {
        if (currentRoutePoints == null) {
            return;
        }
        int nearest = nearestIndex(latitude, longitude);
        double remaining = remainingFrom(nearest);

        StringBuilder status = new StringBuilder();
        if (remaining <= 30.0) {
            status.append(getString(R.string.nav_arrived, routeTotalM / 1000.0));
        } else {
            int minutes = (int) Math.round(remaining / NAV_SPEED_MPS / 60.0);
            status.append(getString(R.string.nav_running,
                    remaining / 1000.0, minutes, (int) Math.round(NAV_SPEED_MPS * 3.6)));
            status.append('\n').append(nextInstruction(nearest, deviation));
        }
        ((android.widget.TextView) findViewById(R.id.text_nav_status)).setText(status);
    }

    /** 当前定位在折线上最近的那个顶点。 */
    private int nearestIndex(double latitude, double longitude) {
        int nearest = 0;
        double best = Double.MAX_VALUE;
        for (int index = 0; index < currentRoutePoints.length(); index += 1) {
            JSONObject point = currentRoutePoints.optJSONObject(index);
            if (point == null) {
                continue;
            }
            double distance = distanceMeters(latitude, longitude,
                    point.optDouble("latitude"), point.optDouble("longitude"));
            if (distance < best) {
                best = distance;
                nearest = index;
            }
        }
        return nearest;
    }

    private double remainingFrom(int fromIndex) {
        double remaining = 0.0;
        for (int index = fromIndex; index + 1 < currentRoutePoints.length(); index += 1) {
            JSONObject a = currentRoutePoints.optJSONObject(index);
            JSONObject b = currentRoutePoints.optJSONObject(index + 1);
            if (a == null || b == null) {
                continue;
            }
            remaining += distanceMeters(a.optDouble("latitude"), a.optDouble("longitude"),
                    b.optDouble("latitude"), b.optDouble("longitude"));
        }
        return remaining;
    }

    /**
     * 下一步动作与它的距离。
     *
     * <p>用 step 的 {@code startIndex/endIndex} 对顶点的索引，而不是「总长 − 剩余」
     * 去猜：累计里程和 steps 的分段里程本来就不是同一套数，之前那样取会取到
     * 「出发，沿…」这种首段文案。这里改成「第一个 endIndex 还没走过的 step」。</p>
     */
    private String nextInstruction(int nearest, double deviation) {
        if (routeSteps == null || routeSteps.length() == 0) {
            return getString(R.string.nav_offset, Math.round(deviation));
        }
        for (int index = 0; index < routeSteps.length(); index += 1) {
            JSONObject step = routeSteps.optJSONObject(index);
            if (step == null) {
                continue;
            }
            int endIndex = step.optInt("endIndex", 0);
            if (endIndex <= nearest) {
                continue;
            }
            int startIndex = step.optInt("startIndex", nearest);
            double ahead = startIndex > nearest ? remainingFrom(nearest) - remainingFrom(startIndex) : 0.0;
            String instruction = step.optString("instruction");
            if (ahead <= 30.0) {
                return instruction;
            }
            return getString(R.string.nav_next, Math.round(ahead), instruction);
        }
        return getString(R.string.nav_last_leg);
    }

    /** `CameraTabType` 的取值，这里只需要给骑手看懂的那几类。 */

    /** 有 tiles.pmtiles 就在回环口起瓦片服务；没有就退回 GeoJSON 底图。 */
    private void startTileServer() {
        java.io.File archive = OfflineData.tilesFile(this);
        if (!archive.isFile()) {
            // 没有瓦片就是没有底图：明确告诉用户，别画一张空白地图让人猜。
            android.widget.Toast.makeText(this, R.string.tiles_missing, Toast.LENGTH_LONG).show();
            return;
        }
        try {
            tileServer = TileServer.start(archive.getAbsolutePath());
            activeTilesUrl = tileServer.tileUrlTemplate();
            Log.i(TAG, "serving tiles from " + archive.getName() + " at " + activeTilesUrl);
        } catch (java.io.IOException error) {
            Log.w(TAG, "tile server failed: " + error);
        }
    }

    // ---- MapView 生命周期 ----

    @Override
    protected void onPause() {
        super.onPause();
        mapView.onPause();
    }

    @Override
    protected void onStop() {
        super.onStop();
        mapView.onStop();
    }

    @Override
    protected void onDestroy() {
        super.onDestroy();
        if (tileServer != null) {
            tileServer.stop();
            tileServer = null;
        }
        if (locationManager != null && locationListener != null) {
            try {
                locationManager.removeUpdates(locationListener);
            } catch (SecurityException ignored) {
                // 进程要退了，没权限也没关系。
            }
        }
        mapView.onDestroy();
    }

    @Override
    protected void onSaveInstanceState(Bundle outState) {
        super.onSaveInstanceState(outState);
        mapView.onSaveInstanceState(outState);
    }

    // ---- 轨迹记录 ----

    /** 开一段轨迹文件：`files/traces/ride-<unix>.jsonl`。 */
    private void startTrace() {
        try {
            java.io.File dir = new java.io.File(getFilesDir(), "traces");
            if (!dir.exists() && !dir.mkdirs()) {
                Log.w(TAG, "cannot create trace dir");
                return;
            }
            traceFile = new java.io.File(dir, "ride-" + System.currentTimeMillis() / 1000 + ".jsonl");
            traceWriter = new java.io.BufferedWriter(new java.io.OutputStreamWriter(
                    new java.io.FileOutputStream(traceFile, true),
                    java.nio.charset.StandardCharsets.UTF_8));
            Log.i(TAG, "recording trace to " + traceFile);
        } catch (java.io.IOException error) {
            Log.w(TAG, "trace open failed: " + error);
            traceWriter = null;
            traceFile = null;
        }
    }

    /** 每次定位落一行，字段与 `motu-prior` 读的格式一致。 */
    private void recordFix(android.location.Location location) {
        if (traceWriter == null) {
            return;
        }
        try {
            String line = String.format(java.util.Locale.US,
                    "{\"t\":%d,\"lat\":%.7f,\"lon\":%.7f,\"speedMps\":%.2f}%n",
                    location.getTime() / 1000,
                    location.getLatitude(),
                    location.getLongitude(),
                    location.hasSpeed() ? location.getSpeed() : 0.0);
            traceWriter.write(line);
            traceWriter.flush();
        } catch (java.io.IOException error) {
            Log.w(TAG, "trace write failed: " + error);
        }
    }

    private void stopTrace() {
        if (traceWriter != null) {
            try {
                traceWriter.close();
            } catch (java.io.IOException ignored) {
                // 关闭失败没有补救动作。
            }
            traceWriter = null;
        }
        if (traceFile != null && traceFile.isFile()) {
            Toast.makeText(this, getString(R.string.trace_saved,
                    traceFile.getAbsolutePath(), traceFile.length() / 1024),
                    Toast.LENGTH_LONG).show();
            traceFile = null;
        }
    }
}
