# Manis Road · Android 界面层

仅包含界面与 `JNI` 转发。所有业务逻辑在 [`crates/motu-core`](../crates/motu-core/README.md)，
经 [`crates/motu-ffi`](../crates/motu-ffi/README.md) 暴露给
`com.manisroad.app.core.MotuCore`。

## 结构

```
app/src/main/
├── AndroidManifest.xml
├── java/com/manisroad/app/
│   ├── MainActivity.java        仪表盘：版本、接口数、车辆数、登录状态
│   ├── LoginActivity.java       官方密码登录（网络在 Rust 核心）
│   ├── MapActivity.java         电子眼提醒、点击详情、离线路线与 GPS 按钮
│   ├── CruiseService.java       GPS 前台服务（持续定位/扫描/通知）
│   ├── VehicleActivity.java     车辆资料（默认车辆 upsert）
│   ├── SettingsActivity.java    本地设置
│   ├── core/MotuCore.java       native 声明 + ensureInit（含 loadGraph/route）
│   ├── core/OfflineData.java    离线路网图位置（外部私有目录）
│   ├── core/CoreResult.java     信封解析
│   ├── core/SigningCertificate.java  读取 APK 签名证书（MD5 在 Rust）
│   └── ui/MapCanvasView.java    无依赖示意地图
└── res/{layout,values}
```

## 构建

前置：JDK 17+、Android SDK（platform 35 / build-tools 35）、NDK
`27.0.12077973`、Rust `aarch64-linux-android` 目标与 `cargo-ndk`。

```bash
rustup target add aarch64-linux-android
cargo install cargo-ndk

./android/build-rust.sh debug          # 生成 jniLibs/arm64-v8a/libmotu_ffi.so
cd android && ./gradlew assembleDebug  # 产出 app/build/outputs/apk/debug/app-debug.apk
```

`android/local.properties` 需要指向 SDK，例如：

```properties
sdk.dir=/path/to/Android/sdk
```

`build-rust.sh` 会从 `ANDROID_HOME` / `ANDROID_SDK_ROOT` / `~/Android/sdk`
推断 SDK，用 `ANDROID_NDK_HOME` 或 `$SDK/ndk/27.0.12077973` 指定 NDK。

## 离线底图（MapLibre Native）

`BaseMapActivity` 用 MapLibre Native 渲染本机路网，数据是包内 `assets/roads.geojson`
（由 `tools/map-preview/geojson-for-android.mjs` 从同一份 OSM 抽取生成）：

```bash
node tools/map-preview/geojson-for-android.mjs \
  var/offline/roads-od.geojson android/app/src/main/assets/roads.geojson
```

入口：仪表盘「离线底图（MapLibre）」。`app/build.gradle.kts` 依赖
`org.maplibre.gl:android-sdk:13.7.1`，`gradle.properties` 需要
`android.useAndroidX=true`（MapLibre 引入 AndroidX）。

为什么是 GeoJSON 而不是包里的 `tiles.pmtiles`：MapLibre Android 的矢量源只接受
HTTP 模板或离线区包，没有内建 `pmtiles://` 协议；Web 端的 pmtiles 协议是 JS 实现。
Android 接 PMTiles 需要一个 tile provider，尚未实现。

### 已验证（2026-11-17，真机）

- `assembleDebug` 通过，APK 30 MB（含 `libmaplibre.so` 13 MB、`libmotu_ffi.so` 9 MB、
  `assets/roads.geojson` 10.5 MB）
- 真机安装后从仪表盘进入「离线底图」，MapLibre 渲染出上海路网并按等级着色，
  平移缩放正常，logcat 无 MapLibre 报错
- 截图：[`docs/motu/images/maplibre-android.png`](../docs/motu/images/maplibre-android.png)

## 已验证

在 `Manis_API_35`（arm64-v8a）模拟器上安装并运行：

- APK 启动，`libmotu_ffi.so` 由 `System.loadLibrary` 成功加载；
- 仪表盘显示 `4.1.9 / 平台 2 / 51 个接口`；
- 模拟导航前进时按 Rust 规则弹出提醒，转向后释放，路线审计返回命中数；
- 关闭“电子眼提醒”后扫描不再提醒；
- 车辆保存改为按默认车辆 upsert，JSON 状态写在
  `files/motu-core/motu-core-state.json`；设置页可修改官方点位的城市代码
  （默认上海 `021`，留空回退北京 `010`）；
- 登录页可打开，Rust/rustls 能向官方 `v4/user/login` 发起请求；无效测试账号
  返回官方业务错误并显示在界面上；
- 登录成功后会保存 `token`、`deviceId`、用户摘要，仪表盘显示登录状态，并可退出；
- 地图页「同步官方点位」在未登录时提示先登录；注入假 token 后，Rust/rustls
  能调用加密的 `searchRulePoiSec`，服务端返回的 token 错误会显示在界面上；
- 离线路线与 GPS 巡航：`MapActivity` 的「离线路线」调用 `MotuCore.route(graphPath, …)`，
  使用本机 `graph.bin`；「GPS 巡航」申请 `ACCESS_FINE_LOCATION`，使用
  `GPS_PROVIDER`（仅 GNSS），每秒扫描本地电子眼，每 10 秒按当前位置重算路线。
  两者都不访问网络。

### 推送离线路网图

图文件不随 APK 分发（全市约 48 MB）。调试版可直接推到应用外部私有目录：

```bash
adb push var/offline/graph.bin \
  /sdcard/Android/data/com.manisroad.app/files/offline/graph.bin
```

没有图文件时「离线路线」会提示缺失，不会回退到网络。`OfflineData` 也会尝试从
APK `assets/graph.bin` 复制（如果打包时放了进去）。

### 离线详情

`MapActivity` 启动时读入 `files/offline/details.json`（由 Web 的
`/api/export/details` 生成），点击点位优先显示本地详情，不需要登录或网络。
在线打开的新详情会合并写回同一文件；点位 id 使用 `memberByString`。

### 离线巡航的边界

- `CruiseService` 是 `foregroundServiceType="location"` 的前台服务，锁屏/切后台后
  仍持续定位并通过通知展示最近一次电子眼提醒；活动页负责渲染广播结果，并用
  路由步骤的 `startIndex`/`endIndex` 显示「下一步：… · N m」。
- 语音播报、屏幕常亮、后台重算路线尚未实现（重算只在活动页存活时每 10 秒执行）。
- 每 10 秒重算路线是最小可用策略，没有做“偏离 80 m 持续 4 秒”的精细判断。
- 定位权限被拒绝时只提示，不崩溃。
- 官方点位结果只进 Rust 内存扫描集合，地图最多绘制 200 个标记，不写本地 JSON
  （Web 调试台的本地缓存是另一套实现，见 `crates/motu-web/README.md`）；
- 地图标记点击命中已用演示点位验证：点击后命中 Rust 返回的标记并在界面上显示
  「演示点位没有官方详情」；未登录时同样提示，不会发起详情请求；
- 官方标记的详情路径已接通：Rust 调用 `trafficEyesDetailSec` 与
  `trafficTicketListSec`，Android 弹出类型、限行规则、说明和罚单列表。

以上为本地模拟器结果，尚未在真机、真实账号或真实数据上验证；电子眼详情的真实
返回内容仍需登录后确认。

## 说明

- 演示电子眼是虚构坐标，不含摩途数据。
- 未打包 AMap 地图 SDK；`MapCanvasView` 只是等距投影的示意图。
- 申请 `INTERNET` 权限，但 HTTP/TLS、签名、密码哈希、会话、官方点位与详情解析
  都在 Rust 核心；Java 只调用 `MotuCore.login/logout/session/syncOfficialCameras/
  loadCameraDetail` 并渲染结果。
- 官方点位只供已登录账号本人使用，不随仓库分发；重启应用后需重新同步。
- VIP/支付必须走官方渠道；当前未移植支付宝/微信 SDK，也不绕过支付或伪造权益。
