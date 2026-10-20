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
│   ├── MapActivity.java         模拟 GPS、电子眼提醒与点击详情
│   ├── VehicleActivity.java     车辆资料（默认车辆 upsert）
│   ├── SettingsActivity.java    本地设置
│   ├── core/MotuCore.java       native 声明 + ensureInit
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
- 官方点位结果只进 Rust 内存扫描集合，地图最多绘制 200 个标记，不写本地 JSON；
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
