# 摩途（Motu）重写：证据与范围

最后核对：2026-10-20。本目录记录从摩途 Android 4.1.9 中恢复出的行为，以及
Rust 重写的取舍。分析产物位于 Git 忽略的 `var/motu/`。

## 目标包

- 包名：`com.motu.motumap`
- 版本：`4.1.9`（versionCode `4010900`）
- 原始安装包 SHA-256：`8dc13fcd688f24bfee4bd45593081c6ddbd145cf9fe6ab7215a311b00587d0ac`
- 服务器：`https://motu.motumap.com/`
- 参考来源：解包后的真实 DEX（`var/motu/dex/classes*.dex`）与反编译源码
  （`var/motu/src/sources`）。加固壳中的 REA/JADX 顶层类不作为业务行为依据，
  因为那部分主要是 `com.Proxy` / `com.Install` / `com.appsec` 壳代码。

## 文档索引

- [`protocol.md`](protocol.md)：签名、AES-CBC 回退、AES-GCM/RSA/HMAC 加密协议、
  `safe-env-token`、安全配置引导、密码登录与会话。
- [`endpoints.md`](endpoints.md)：51 个接口目录、请求头、免签名白名单。
- [`camera.md`](camera.md)：电子眼类型、扫描/聚类/释放规则、车辆枚举、路线审计。
- [`gaps.md`](gaps.md)：原生 `requireSalt`、AMap/广告/推送等未重写部分与限制。
- [`route-quality.md`](route-quality.md)：离线路由的成本模型、改造前后的实测质量对比与当前缺口。
- [`offline-package.md`](offline-package.md)：`.mrdp` 离线数据包的格式、打包/校验流程与 App 侧读取方式。
- [`offline-map-tiles.md`](offline-map-tiles.md)：离线底图瓦片流水线（OSM → GeoJSON → tippecanoe → PMTiles → .mrdp）。

## Rust 结构

- [`crates/motu-core`](../../crates/motu-core/README.md)：纯 Rust 业务核心。
- [`crates/motu-ffi`](../../crates/motu-ffi/README.md)：JNI JSON 桥。
- [`crates/motu-routing`](../../crates/motu-routing/README.md)：离线 OSM 路网图与摩托
  车 A* 路由（等级权重 + 转向惩罚 + 质量指标）。
- [`crates/motu-web`](../../crates/motu-web/README.md)：本机 Web 调试台，登录/同步/详情仍走 `motu-core`，Leaflet 只负责渲染和地图交互。
- [`crates/motu-pack`](../../crates/motu-pack/README.md)：离线数据包 `.mrdp`
  的读写与命令行工具。
- [`android/`](../../android/README.md)：Java 界面与构建脚本。

## 数据与合规

- 仓库不附带摩途电子眼数据或用户上报数据，也不随包分发。
- 演示点位是虚构坐标，与真实执法点位无关。
- 线上调用只使用使用者自己的账号与已授权资源；登录密码只做协议要求的 MD5。
- 恢复的客户端密钥来自 APK 自身，用于协议兼容性研究。
