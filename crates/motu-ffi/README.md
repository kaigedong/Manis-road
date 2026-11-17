# manis-motu-ffi

`manis-motu-core` 的 JNI 桥。库名 `libmotu_ffi.so`，Java 侧入口是
`com.manisroad.app.core.MotuCore`。

## 约定

- 每个导出函数返回 UTF-8 JSON 信封：成功 `{"ok":true,"data":...}`，失败
  `{"ok":false,"error":"..."}`。Java 不需要捕获 native 异常。
- 输入 JSON 也走同一层，字段名与 Rust 的 `serde(rename_all = "camelCase")`
  一致（例如 `numberPlate`、`cameraAlertsEnabled`）。
- 全局状态用 `OnceLock<Mutex<FfiState>>` 保存：扫描引擎、`FileStore`、
  `CoreState`。`apply_settings` 把 `cameraRadiusOverrideM` 应用到扫描配置，
  `nativeScan` 还会叠加 `cameraAlertsEnabled` 开关。

## 导出（`Java_com_manisroad_app_core_MotuCore_*`）

| 方法 | 作用 |
|------|------|
| `nativeInit(dir)` | 打开 JSON 存储、载入状态、应用设置 |
| `nativeLoadState` / `nativeSaveState` | 读/写 `CoreState`（保存前校验车辆） |
| `nativeDefaultVehicle` | 默认车辆或 `null` |
| `nativeUpdateCameras` | 替换扫描器的电子眼集合，返回数量 |
| `nativeScan(lat, lon, bearing, allow)` | 一次扫描，返回 `ScanResult` 或 `null` |
| `nativePause` / `nativeResetScan` | 暂停扫描 / 清空 |
| `nativeBuildRequest` | 按接口名构造签名请求（GET/POST） |
| `nativeSafeEnvToken` | `safe-env-token` 头 |
| `nativeLogin(phone, password, signingCertificate)` | 官方密码登录；Rust 从证书 DER 计算签名 MD5；阻塞网络调用，成功后保存会话 |
| `nativeSession` / `nativeDeviceId` | 当前会话 / 每安装设备 ID |
| `nativeLogout` | 尽力调用官方退出，然后清除本地会话 |
| `nativeSyncOfficialCameras(lat, lon, city, signingCertificate)` | 登录后同步官方电子眼；仅更新内存扫描集合 |
| `nativeLoadCameraDetail(cameraId, signingCertificate)` | 登录后加载电子眼详情与罚单；返回合并 JSON，不持久化 |
| `nativeVehicleValidate` | 车辆校验报告 |
| `nativeLoadGraph(graphPath)` | 加载并缓存离线 OSM 路网图，返回节点/边/范围 |
| `nativeRoute(graphPath, oLat, oLon, dLat, dLon)` | 纯离线 A* 路线，返回 points/distance/duration/steps（含 `startIndex`/`endIndex`） |
| `nativeRouteAvoiding(graphPath, …, camerasJson, radiusM, maxRatio, maxMinutes)` | 电子眼加权绕行，返回 `{plan, avoidance}`；超限自动回退最快路线 |
| `nativeRouteAudit` | 路线对避让区/电子眼的审计 |
| `nativePlanStraightLine` | 离线直线规划（`RoutePlanner` 端口） |
| `nativeEncodePolyline` / `nativeDecodePolyline` / `nativeParseAmapPositions` | polyline 与 AMap 坐标 |
| `nativeEndpoints` / `nativeConstants` | 诊断信息 |

## 构建

Android（arm64-v8a，需 NDK）：

```bash
./android/build-rust.sh debug      # 输出到 android/app/src/main/jniLibs
```

`default = ["http"]` 会为非 Android 目标也编译 `reqwest`/`rustls`，因此桌面
`cargo test` 首次构建会稍慢。`nativeLogin`、`nativeSyncOfficialCameras` 与
`nativeLoadCameraDetail` 使用阻塞客户端，Java 侧必须放在工作线程调用。官方点位
同步和详情都要求已登录；同步结果只进入 `CameraScanEngine` 的内存列表，详情只
返回给 UI，二者都不写入 JSON 存储。

桌面自测（桥的 JSON 约定）：

```bash
cargo test -p manis-motu-ffi
```

JNI 导出依赖 `#[unsafe(no_mangle)]`，`unsafe` 只允许出现在本 crate
（`#![allow(unsafe_code)]`）。
