package com.manisroad.app;

import android.app.Activity;
import android.os.Bundle;
import android.util.Log;
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

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        // 必须在任何 MapView 之前初始化，MapLibre 13.x 要求显式 getInstance。
        MapLibre.getInstance(this);
        setContentView(R.layout.activity_base_map);

        mapView = findViewById(R.id.base_map_view);
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
}
