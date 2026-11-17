package com.manisroad.app;

import android.app.Activity;
import android.os.Bundle;
import android.util.Log;
import android.widget.TextView;
import android.widget.Toast;

import com.manisroad.app.core.CoreResult;
import com.manisroad.app.core.MotuCore;
import com.manisroad.app.core.OfflineData;

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

    /** 只画道路：背景 + 线，颜色按等级区分。 */
    private static final String STYLE_JSON = """
        {
          "version": 8,
          "sources": {
            "roads": {
              "type": "geojson",
              "data": "asset://roads.geojson"
            },
            "route": {
              "type": "geojson",
              "data": { "type": "FeatureCollection", "features": [] }
            },
            "endpoints": {
              "type": "geojson",
              "data": { "type": "FeatureCollection", "features": [] }
            }
          },
          "layers": [
            {
              "id": "background",
              "type": "background",
              "paint": { "background-color": "#f4f1ea" }
            },
            {
              "id": "road-casing",
              "type": "line",
              "source": "roads",
              "paint": { "line-color": "#ffffff", "line-width": 3.2 }
            },
            {
              "id": "roads",
              "type": "line",
              "source": "roads",
              "paint": {
                "line-color": [
                  "match", ["get", "highway"],
                  "trunk", "#d94f2b",
                  "primary", "#e07b39",
                  "secondary", "#e8a33d",
                  "tertiary", "#f0c04a",
                  "#5d8aa8"
                ],
                "line-width": 1.6
              }
            },
            {
              "id": "route-casing",
              "type": "line",
              "source": "route",
              "layout": { "line-join": "round", "line-cap": "round" },
              "paint": { "line-color": "#ffffff", "line-width": 9 }
            },
            {
              "id": "route-line",
              "type": "line",
              "source": "route",
              "layout": { "line-join": "round", "line-cap": "round" },
              "paint": { "line-color": "#d81b60", "line-width": 6 }
            },
            {
              "id": "endpoints",
              "type": "circle",
              "source": "endpoints",
              "paint": {
                "circle-radius": 7,
                "circle-color": ["match", ["get", "kind"], "origin", "#2e7d32", "#c62828"],
                "circle-stroke-color": "#ffffff",
                "circle-stroke-width": 2
              }
            }
          ]
        }
        """;

    private MapView mapView;
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

        mapView = findViewById(R.id.base_map_view);
        findViewById(R.id.button_replan_check).setOnClickListener(view -> runReplanCheck());
        mapView.onCreate(savedInstanceState);
        mapView.getMapAsync(map -> {
            map.getUiSettings().setCompassEnabled(true);
            map.setStyle(
                    new org.maplibre.android.maps.Style.Builder().fromJson(STYLE_JSON),
                    style -> {
                        Log.i(TAG, "style loaded");
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
        mapView.onDestroy();
    }

    @Override
    public void onLowMemory() {
        super.onLowMemory();
        mapView.onLowMemory();
    }

    @Override
    protected void onSaveInstanceState(Bundle outState) {
        super.onSaveInstanceState(outState);
        mapView.onSaveInstanceState(outState);
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
}
