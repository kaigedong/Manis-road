# manis-motu-core

摩途（Motu）Android 4.1.9 的业务核心，用纯 Rust 重写。Android 层只保留界面与
`JNI` 转发；本 crate 不依赖 Android、不联网（可选的 `http` feature 除外），
可在桌面直接测试。

## 范围

| 模块 | 来源 | 内容 |
|------|------|------|
| `api` | `com.motu.api.service.*`、`com.motumap.base.http` | 51 个接口目录、请求头、签名请求构造、`Transport` 抽象、加密客户端 |
| `auth` | `SignInViewModel` / `IUserApiService` | 官方密码登录、退出、会话模型 |
| `crypto` | `SecurityIntercepter` / `AppendEncryptionIntercepter` / `HmacUtil` | 大写 native MD5 签名、小写密码 MD5、AES-CBC 回退、AES-GCM、HMAC-SHA256、RSA-OAEP（MGF1-SHA1）、安全信封 |
| `camera` | `com.motu.motumap.navi.camera.CameraScanEngine` | 前向电子眼扫描、聚类、提醒释放 |
| `official` | `MotuMapViewModel.searchCameras` / `loadTrafficCameraDetail` | 登录后调用 `searchRulePoiSec` 获取点位，并调用 `trafficEyesDetailSec` / `trafficTicketListSec` 合并详情与罚单；只在内存中使用 |
| `route` | `calPathAvoidSec` / `calPathAreaSec` | 避让区与电子眼的路线审计、polyline、AMap 坐标、离线直线规划 |
| `vehicle` | 车辆表单与 `userCar` 接口 | 号牌解析、颜色/动力/变速枚举、校验 |
| `store` | `SharedPreferences`/SQLite 的替代 | 单 JSON 文档持久化（原子写、schema 版本）、`deviceId` 与登录会话 |
| `geo` | `AMapUtils` | 距离、方位角、夹角、多边形/线段相交 |
| `model` | `com.motu...model.*` | 电子眼、区域、请求/响应模型 |

## 运行

```bash
cargo test -p manis-motu-core
cargo clippy -p manis-motu-core --all-targets --features http
```

离线验证请求构造：

```rust
use motu_core::{BASE_URL, HeaderInit, base_headers, build_request, endpoint_by_name};
let endpoint = endpoint_by_name("loadCameraTabType").unwrap();
let headers = base_headers(&HeaderInit::default());
let request = build_request(&endpoint, BASE_URL, &headers, &Default::default(), None, 0)?;
```

## 数据与合规

- 不附带、不内置摩途的电子眼数据或用户上报数据；`camera`、`route` 只消费调用方
  传入的点位，`official` 仅在登录后按需同步到内存。仓库内演示点位均为虚构坐标。
- 线上调用只应使用使用者自己的账号和已授权资源；`build_request` 只构造请求，
  是否发送由调用方决定。
- 登录密码只做协议要求的小写 MD5 后发送，不写入本地状态；本地只保存服务端返回的
  `token` 和用户摘要。
- 恢复到的密钥与常量用于兼容性研究，属于公开于 APK 内的客户端材料。

协议、接口与算法的细节见 [`docs/motu/`](../../docs/motu/README.md)。
