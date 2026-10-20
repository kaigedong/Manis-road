# 协议与加密

来源：`com.motumap.base.http` 拦截器链、`SecurityIntercepter`、
`AppendEncryptionIntercepter`、`HmacUtil`、`SecurityServerResponse`。
实现见 [`crates/motu-core/src/crypto.rs`](../../crates/motu-core/src/crypto.rs)
与 [`api.rs`](../../crates/motu-core/src/api.rs)。

## 拦截器顺序

1. `AppendHeaderParamIntercepter`：加 `version`、`platForm`，以及可选
   `token`、`deviceId`、`safe-env-token`。
2. `AppendEncryptionIntercepter`：加 `timeStamp` 与 MD5 `sign`。
3. `SecurityIntercepter`：对 `Security-Level: Encrypt` 的接口加密已经签过名的
   body。

因此**签名在加密之前**：加密的是「带 `timeStamp` 和 `sign` 的明文 body」。

## MD5 签名

- 秘钥常量：`6ccf59766be2ce07c8714fc54d768d42`。
- 规范化：把参数按 key 升序拼成 `k=v&...`，跳过 `null` 与空串，再追加
  `&key=<secret>`，对整串调用 native `encryptMD5`。
- `encryptMD5` 返回**大写**十六进制 MD5；服务端大小写敏感，不能用小写。
- 密码登录的 `pwd` 是 Java `MD5Utils.md5PasswordTo32` 的结果，为**小写**
  MD5，和请求签名不是同一个大小写规则。
- GET：`timeStamp` 进入待签名参数，`sign` 追加到 query。
- POST：`timeStamp` 以十进制字符串写入 body，`sign` 也写入 body。
- 免签名白名单：路径包含 `poi/getPoiTabs` 或 `https://tsapi.amap.com`。

## AES-CBC 回退

- 密钥 `rSsbdxSPjfsVWyVa`，IV `VsvgRCrL9RDNcE8J`，AES-128-CBC/PKCS#7，
  密文 Base64。用于旧接口/回退路径。

## 加密协议（AES-GCM + RSA-OAEP + HMAC-SHA256）

请求：

1. 生成 32 字节 AES key 和 12 字节 GCM IV（Base64）。
2. 用 `key + iv + AAD` 做 AES-GCM 加密已签名明文，得 `data`。
3. 用服务器 SPKI 公钥做 RSA-OAEP 包裹 Base64 key，得 `encryptedKey`。
   Java 的 `RSA/ECB/OAEPWithSHA-256AndMGF1Padding` 实际使用 **SHA-256 摘要 +
   MGF1-SHA1**（平台默认），Rust 侧必须使用 `Oaep::new_with_mgf_hash::<Sha256, Sha1>()`。
4. `sign = HMAC-SHA256(base64text(key), "<keyId>\n<timestamp>\n<nonce>\n<data>")`。

信封字段：`keyId`、`encryptedKey`、`iv`、`data`、`timestamp`、`nonce`、`sign`。

响应：`timestamp`、`nonce`、`iv`、`data`、`sign`；HMAC 覆盖
`<timestamp>\n<nonce>\n<data>`，用**同一把** per-request key 校验后解密。

注意：`HmacUtil.sign` 用的是 Base64 **文本**作为 HMAC 密钥（而不是解码后的
字节），Rust 侧保留了这个行为。

## AES-GCM AAD（`requireSalt`）

原版 4.1.9 的 native `requireSalt(String)` 只做非空校验，然后返回
`salt.getBytes(StandardCharsets.UTF_8)`；空字符串会抛
`IllegalArgumentException("salt 不能为空")`。因此 AAD 就是服务器下发的
Base64 salt 文本本身，而不是解码后的字节或哈希。Rust 侧默认
`Aad::SaltUtf8` 与之等价，`Aad::Custom` 仅保留给测试/未来变体。

## safe-env-token

`<unix_seconds>.<md5(deviceId + "-" + seconds + "-" + signatureMd5)>`。
`signatureMd5` 由 APK 签名证书的 DER 字节做 MD5 得到，native
`AppSignatureHelper.md5` 返回**大写**十六进制。Android 侧
`SigningCertificate` 只通过 `PackageManager` 读取证书字节；MD5
（`signing_certificate_md5`）与 token 拼接（`safe_env_token`）都在 Rust。
2026-10-20 的线上验证表明，公开加密接口（`v1/poi/mapShowAreaSec`）在缺少
该头、缺少 `deviceId`、甚至使用随机 `signatureMd5` 时仍然接受请求；
登录/支付等用户接口是否强制校验尚待验证。

## 安全配置引导

- 路径：`api/security/public-key`。
- 配置字段：`keyId`、`publicKey`(SPKI DER Base64)、`salt`、`fetchedAtMs`、`ttlMs`。
- TTL：`86400000` ms（24 小时）。

## 密码登录与会话

- `POST v4/user/login`，body：
  `{"phone":"...","pwd":"<小写 MD5(明文密码)>","type":0}`；随后由签名拦截器
  加入 `timeStamp` 与大写 `sign`。
- 成功响应 `data` 至少包含 `token`、`userId`、`phone`、`nickName`、`isPro`、
  `memEndTime`。`token` 存入本地状态，后续用户接口通过 `token` 头传递。
- `POST v4/user/logout` 需要 `token` 头；客户端无论服务端退出是否成功都会清除
  本地会话。
- 密码只做协议要求的小写 MD5，不写入本地状态；本地只保存 token 与用户摘要。

## 官方点位同步

- 路径：`POST v1/poi/searchRulePoiSec`，带 `Security-Level: Encrypt`，响应同样是
  AES-GCM/RSA/HMAC 信封。
- 请求体见 [`camera.md`](camera.md) 的「官方点位同步」一节；关键是 `cityCode`、
  `tabTypes`、默认车辆颜色/号牌。
- 该接口**要求登录**：无 `token` 或 token 无效时返回明文业务错误
  `{"code":20105,"message":"token校验异常：access_token非法 Token无效"}`。
- 同步结果只进入 Rust 内存扫描集合，不写本地 JSON，不随仓库分发。

## 线上验证

2026-10-20 使用官方服务当前下发的 salt
`TW90dVNlY3VyZUFwaVYyU2FsdDIwMjYwNzIy` 对
`POST v1/poi/mapShowAreaSec` 做了端到端验证：Rust 侧构造的
AES-GCM/RSA/HMAC 信封被服务端接受，响应 HMAC 校验与解密成功。同一轮
测试也确认了 `requireSalt` 的行为和 RSA-OAEP 的 MGF1-SHA1 参数。

登录请求形状用无效测试账号验证：修复大写 `sign` 后，服务端不再返回
`sign校验异常`，而是返回业务错误 `系统异常，请稍后再试！`。Android 模拟器上
的 Rust/rustls 登录请求也得到同一响应，说明 TLS、签名、JSON body 和错误解析
链路已贯通。真实账号登录、用户信息、VIP 与支付接口仍需使用者用自有账号验证。

官方点位同步用无 token 的加密请求验证到业务层：`searchRulePoiSec` 返回
`code 20105` 的 token 错误；Android 模拟器上用注入的假 token 也得到同一错误并
正确显示。加密信封、请求体和错误解析均已贯通；真实账号返回点位的端到端结果
仍需使用者用自有账号验证。

## 已知不确定点

- 真实账号登录后的 token 刷新、用户信息、VIP 商品/订单与官方点位返回尚未做
  线上端到端验证。
- 服务端是否对用户接口强制校验 `safe-env-token`/官方签名、设备指纹或
  账号风控，仍需用自有测试账号确认。
- 支付必须走官方下单/支付渠道；本项目不伪造订单、不绕过 VIP 或支付。
