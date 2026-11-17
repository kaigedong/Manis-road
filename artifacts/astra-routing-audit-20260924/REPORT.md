# Astra 请求路由实测

时间：2026/9/24 16:51:46 至 2026/9/24 16:58:33（Asia/Shanghai）。

## 结论

本轮 8 次 Astra 正式请求全部检查到 response.created / response.completed 的模型字段。完成 8 次；模型标识不一致 0 次。2 次 Luna 对照完成 2 次。

Astra 正式请求中 8/8 次返回了 x-codex-safety-buffering-faster-model=gpt-5.6-luna；这个值来自 WebSocket 的 codex.response.metadata.headers。它是与安全缓冲相关的更快重试模型候选，单凭该值不能证明当前生成已经切换。实际 response.model 仍为 gpt-6-astra。

## 方法和范围

- 本机 codex-cli 0.156.0，使用现有 ChatGPT OAuth；实际连接 wss://chatgpt.com/backend-api/codex/responses。这不是 Platform API key 路径，也不是 Pi 的实际请求。
- 原生 CLI 顺序执行，分别测试 low/high/xhigh/max；任务包括短回答、排列计算、Rust 整数边界审查和较长 LRU 设计。
- inherit 保留用户配置；clean 使用 --ignore-user-config，继续使用相同 CODEX_HOME 的登录信息；每次显式设置模型和推理强度。继承配置与干净配置分别验证。
- 获取原始 WebSocket 消息的 JSON 后才提取模型字段，避免仅从本地界面/会话配置推断服务端模型。
- 除第一次冒烟外，客户端 websocket timing 记录额外提供 requested model 与 warmup 标记。共 10 次正式请求、10 次预热请求，分别统计。
- 每个 response ID 都核对开始和完成模型名；没有使用模型自称、输出质量或 token 速度作为身份判据。
- 采集器只保存白名单字段，不保存 Authorization、Cookie、x-codex-turn-state、完整请求体、完整原始日志或回答内容。
- 记录到 5 个非 JSON Received-message 帧（均为观测记录所列形状）；正式响应的创建、进行与完成事件已独立保留，不将这些帧作为模型证据。
- 10 次 WebSocket HTTP 升级响应中，openai-model 头出现 0 次；流式 metadata 中出现 0 次。未出现与出现但值相同分别对待。
- 用户 config.toml 校验哈希前后一致：true。

## 正式请求结果

| 测试 | effort | 配置 | 开始模型 | 完成模型 | 输出 tokens | faster-model 候选 |
|---|---|---|---|---|---:|---|
| astra-high-smoke | high | inherit | gpt-6-astra | gpt-6-astra | 5 | gpt-5.6-luna |
| astra-high-reasoning | high | inherit | gpt-6-astra | gpt-6-astra | 116 | gpt-5.6-luna |
| luna-control-before | high | clean | gpt-5.6-luna | gpt-5.6-luna | 5 | 无 |
| astra-low-pong | low | clean | gpt-6-astra | gpt-6-astra | 5 | gpt-5.6-luna |
| astra-high-coding | high | inherit | gpt-6-astra | gpt-6-astra | 482 | gpt-5.6-luna |
| astra-xhigh-coding | xhigh | clean | gpt-6-astra | gpt-6-astra | 1048 | gpt-5.6-luna |
| astra-max-reasoning | max | clean | gpt-6-astra | gpt-6-astra | 239 | gpt-5.6-luna |
| astra-high-long | high | clean | gpt-6-astra | gpt-6-astra | 2350 | gpt-5.6-luna |
| astra-high-repeat | high | inherit | gpt-6-astra | gpt-6-astra | 5 | gpt-5.6-luna |
| luna-control-after | high | clean | gpt-5.6-luna | gpt-5.6-luna | 171 | 无 |

## 可追溯证据

| 测试 | Response ID | 脱敏原始字段 |
|---|---|---|
| astra-high-smoke | `resp_0681b4e8548e424e016ab4e4aafb4087d283f895b877c383f4` | [事件](runs/astra-high-smoke/events.jsonl) |
| astra-high-reasoning | `resp_0685c4c30408f5ec016ab4e4f6779c87d19551faeb18c32872` | [事件](runs/astra-high-reasoning/events.jsonl) |
| luna-control-before | `resp_05190d0ca5c94e2e016ab4e53b0c9487d194b94fae76266bf2` | [事件](runs/luna-control-before/events.jsonl) |
| astra-low-pong | `resp_0cbbce52a74ecd81016ab4e54e163087d29cecb35e0714c175` | [事件](runs/astra-low-pong/events.jsonl) |
| astra-high-coding | `resp_0f61176b4a087026016ab4e55f967087d1ad8c1680b565cb2c` | [事件](runs/astra-high-coding/events.jsonl) |
| astra-xhigh-coding | `resp_0c58c175ba374772016ab4e5839bfc87d2a7390dcc1c7013fe` | [事件](runs/astra-xhigh-coding/events.jsonl) |
| astra-max-reasoning | `resp_04ec2debbf3173cb016ab4e5b21f2087d29ddc2991ac02e041` | [事件](runs/astra-max-reasoning/events.jsonl) |
| astra-high-long | `resp_08e166f90d96ab60016ab4e5cc51f887d289c9c4f23130ce46` | [事件](runs/astra-high-long/events.jsonl) |
| astra-high-repeat | `resp_0e96648417fcd67f016ab4e61c918c87d2b048b9c3c6114551` | [事件](runs/astra-high-repeat/events.jsonl) |
| luna-control-after | `resp_05580c8605739649016ab4e6308b2c87d2944339c5e12704e9` | [事件](runs/luna-control-after/events.jsonl) |

机器可读汇总见 [summary.json](summary.json)，完整计划见 [task_plan.md](task_plan.md)，采集器见 [probe.mjs](probe.mjs)。正式请求报告的 input/output/reasoning tokens 为 156423/4426/1845。预热报告的 token 数单列在 summary.json；不将这些数字直接解释为计费或额度扣减。

## 判定依据与限制

- 当前证据支持：在以上时间窗口和请求样本内，没有发现 Astra 请求的服务器响应模型标识变成 Luna。
- 服务端返回了模型名，但没有提供可独立验证模型权重的证明。因此 Astra 标签一致不代表已证明权重身份，也无法排除其他时段、会话、客户端或账号出现不同路由。
- 若未来出现 requestedModel=Astra，而相同正式响应 ID 的 modelAtCreated/modelAtCompleted=Luna，应保留响应 ID、时间、两个模型字段以及路由头；若只有 faster-model=Luna，仍不足以判定替换。
- 当前版本官方源码：升级头采集位于 [responses_websocket.rs](https://github.com/openai/codex/blob/rust-v0.156.0/codex-rs/codex-api/src/endpoint/responses_websocket.rs)；安全缓冲候选头解析位于 [safety_buffering.rs](https://github.com/openai/codex/blob/rust-v0.156.0/codex-rs/codex-api/src/safety_buffering.rs)；更快模型重试流程位于 [TUI retry](https://github.com/openai/codex/blob/rust-v0.156.0/codex-rs/tui/src/app/safety_buffering.rs)。
