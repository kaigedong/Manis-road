# 摩途 HTTPS 调试：当前状态

最后核对：2026-10-20。

## REA 安装与实际分析

REA CLI 6.7.0 已安装，并已注册到 OpenCode。配套技能位于
`/Users/bobo/.agents/skills/reverse-engineer-anything/`。OpenCode 的 REA MCP
工具已在本会话出现，并成功完成一次 Android 类清单查询。

JADX headless MCP 0.7.1 已安装到
`/Users/bobo/.local/share/rea/android/jadx-headless-mcp-0.7.1-all.jar`，
SHA-256 与 REA 文档指定值一致。现有 OpenJDK 21 已成功运行该引擎。

对原始 APK 的静态检查结果：

- REA/JADX 识别到 34 个顶层类，主要位于 `com.Proxy`、`com.Install` 和 `com.appsec`。
- `com.motu.motumap.R` 出现在清单中，但没有恢复业务实现。
- 包检查返回的包名、版本和 SDK 摘要为空，清单结果是未正常解码的二进制内容，不能把它当成有效的 XML 清单。
- 类清单的完整覆盖仅针对该引擎可见的顶层类，不代表已恢复加固内容或全部业务代码。
- 没有运行 APK，没有产生真实 HTTPS 请求或响应日志。

类清单证据 ID：`ev_cdc8cdb5a860e61a2745c61b4383d0edb0e4813c3db7d98fe5781e562d137058`。
完整 MCP 证据导出位于 `var/motu/rea-static-evidence.json`；CLI 包检查结果位于
`var/motu/rea-package-evidence.json`。安装计划与结果分别位于
`var/motu/rea-setup-plan.json` 和 `var/motu/rea-setup-result.json`。

跨会话约束已核对：会话“探讨摩途上海电子眼数据拉取方案”记录了
`motu.motumap.com` 拒绝代理证书、TLS 握手失败。该结果无法区分用户 CA
信任策略与证书固定。用户已要求停止使用其手机，后续工作遵守此约束，
不重复此前 MITM 配置验证。

当前是未完成、未通过设备验证的实验材料，不能作为可安装的调试版交付。

## 已确认

- 已从此前连接的手机提取摩途 4.1.9，包名 `com.motu.motumap`，版本号 `4010900`。
- 原始安装包保存在 `var/motu/original/motu-device.apk`，大小为 162585625 字节。
- 原始安装包 SHA-256：`8dc13fcd688f24bfee4bd45593081c6ddbd145cf9fe6ab7215a311b00587d0ac`。
- 安装包包含加固加载代码；静态反编译主要得到壳代码，没有恢复业务源码。
- 未卸载或覆盖手机中的原版应用。

## 未完成与阻塞

- 构建在 `Assemble.java` 调用当前依赖中不存在的 `Opcodes.forApi(int)` 时失败。
- 后续运行时注入构建步骤被自动安全审核拦截，未继续执行。
- 没有生成 `classes2.dex`、调试 APK 或签名 APK，没有安装调试版。
- 最后核对时 `adb` 报告 `no devices/emulators found`，无法继续实机验证。
- JavaScript 打包成功及 Python 语法检查通过，仅说明相关文件可解析，不代表抓包有效。

## 日志覆盖限制

`network.js` 是未验证的日志原型：尝试记录 OkHttp 请求与响应，以及 Java URL 的打开事件。它没有实现全部 HTTPS 流量覆盖。

- 未验证加固后的类加载器是否能找到目标网络库。
- 未实现 native TLS、QUIC、WebView 或所有第三方 SDK 的请求和响应内容记录。
- 流式、未知长度、超过 1 MiB、非 JSON/表单等请求或响应体会被省略。
- 敏感字段遮盖依赖字段名称，不能保证消除所有敏感信息；URL 路径、非标准字段、错误文本等仍可能包含敏感数据。
- 原型使用 Android logcat，不会向远程服务器上传日志；logcat 的访问范围由设备系统决定。
- Android 日志缓冲区可能丢失分片；`collect.py` 可以报告仍在缓冲中的不完整记录，但无法证明没有完整记录整体丢失。

`collect.py` 仅收集和重组 `MotuHTTPS` 标签的已有日志，不会注入应用或解密 HTTPS。当前没有已验证的真实 HTTPS 日志样本。

## 文件说明

- `prepare.py`：实验性的 APK 组装代码，尚未完成端到端构建和验证。
- `DebugApplication.smali`、`Assemble.java`：实验性的启动及组装代码，当前构建失败。
- `network.js`：实验性的运行时日志代码，尚未实机验证。
- `collect.py`：本机 JSONL 日志收集器，输出文件采用仅当前用户读写权限。

安装包、工具依赖和生成文件保存在被 Git 忽略的 `var/motu/` 下。
