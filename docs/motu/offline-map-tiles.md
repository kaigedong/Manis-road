# 离线底图瓦片

目标：App 断网时也能画出地图，而不是只有示意线段。

## 流水线

```text
Overpass JSON ──► GeoJSON ──► tippecanoe ──► .mbtiles ──► pmtiles convert ──► .pmtiles ──► .mrdp
```

工具（macOS，Homebrew）：

```bash
brew install tippecanoe pmtiles
```

### 1. OSM → GeoJSON

只保留道路几何与属性（`highway/name/lanes/surface/oneway/bridge/tunnel`），
坐标用 [lon, lat]。转换脚本在 `tools/map-preview/osm2geojson.mjs`。

本仓库实测数据：`var/offline/osm-od-area-tagged.json`（4 块合并，46,220 条 way）
→ `roads-od.geojson`（14 MB，46,220 条 LineString）。

### 2. GeoJSON → MBTiles

```bash
tippecanoe -o var/offline/roads-od.mbtiles -f --layer=roads -Z 8 -z 15 \
  --drop-densest-as-needed --extend-zooms-if-still-dropping --simplification=10 \
  var/offline/roads-od.geojson
```

结果：5.0 MB，z8–z15。

### 3. MBTiles → PMTiles

```bash
pmtiles convert var/offline/roads-od.mbtiles var/offline/roads-od.pmtiles
pmtiles show var/offline/roads-od.pmtiles
```

```text
pmtiles spec version: 3
tile type: mvt
bounds: (long: 121.302321, lat: 31.134522) (long: 121.637380, lat: 31.342711)
min zoom: 8   max zoom: 15
addressed tiles count: 793
internal compression: gzip   tile compression: gzip
```

结果：4.4 MB。

### 4. 进包

```bash
cargo run -p manis-motu-pack --bin motu-pack -- build \
  --out var/offline/shanghai-od.mrdp \
  --graph var/offline/graph-od.bin \
  --prior crates/motu-routing/examples/traffic-prior.json \
  --cameras var/offline/cameras.json \
  --tiles var/offline/roads-od.pmtiles \
  --bbox 31.13,121.30,31.33,121.63
```

```text
  graph.bin               7897788 bytes  crc32 915e25a6
  traffic-prior.json          796 bytes  crc32 4326fcf8
  avoid.json                  117 bytes  crc32 b657ff86
  cameras.json             454127 bytes  crc32 67c5ff16
  tiles.pmtiles           4650307 bytes  crc32 bac5743b
ok: 5 entries verified, 13003135 bytes
```

## 瓦片内容验证

单块瓦片解出来看图层与属性：

```bash
pmtiles tile var/offline/roads-od.pmtiles 14 13719 6692 > /tmp/tile.mvt
gunzip -c /tmp/tile.mvt | strings | head
```

```text
roads(
highway
name
lanes
surface
oneway
bridge
tunnel
trunk / gravel / asphalt / footway
```

`pmtiles` 的 JS 库直接读同一块瓦片也能拿到 15,889 字节（与上面 gzip 后大小一致），
说明容器、目录与瓦片编码都是通的。

## 预览页

`tools/map-preview/index.html` 是一个只用 MapLibre GL JS + pmtiles 协议渲染
`tiles.pmtiles` 的页面，`tools/map-preview/serve.mjs` 提供带 Range 的静态服务
（`pmtiles://` 靠 Range 只取需要的字节，普通静态服务器不给 206 就会失败）：

```bash
node tools/map-preview/serve.mjs var/offline 8089
# 打开 http://127.0.0.1:8089/index.html?pmtiles=./roads-od.pmtiles
```

## 未完成 / 已验证到什么程度

- ✅ 流水线与数据：PMTiles v3 容器、793 块瓦片、图层与属性、进 `.mrdp` 并校验通过
- ✅ MapLibre 能加载该源：服务端日志确认它按 Range 取到了瓦片数据
  （`GET /roads-od.pmtiles bytes=2452842-2506416`），`sourcedata` 事件正常、无报错
- ⚠️ **像素级渲染未验证**：本机 headless Chrome 的 WebGL 走 SwiftShader，
  `Page.captureScreenshot` 只拍到背景色，canvas 回读也没有道路——
  用正常桌面浏览器打开预览页可自行确认
- 🟡 Android 端已接入 MapLibre Native（`BaseMapActivity`），但用的是同源 GeoJSON
  （`assets/roads.geojson`，10.5 MB），**不是** `tiles.pmtiles`：
  MapLibre Android 的矢量源只接受 HTTP 模板或离线区包，没有内建 `pmtiles://`
  协议。APK 构建通过（30 MB，含 `libmaplibre.so`），真机运行未验证（设备未授权）
- ⛔ 底图只有道路层：没有建筑、水系、地名标注（Overpass 只抓了 `highway`）
