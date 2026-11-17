package com.manisroad.app;

import android.app.Activity;
import android.os.Bundle;
import android.util.Log;

import org.maplibre.android.MapLibre;
import org.maplibre.android.maps.MapView;

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
                        map.animateCamera(
                                org.maplibre.android.camera.CameraUpdateFactory.newLatLngZoom(
                                        new org.maplibre.android.geometry.LatLng(31.2304, 121.4737),
                                        13.0));
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
}
