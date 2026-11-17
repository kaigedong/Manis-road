# 发现

- 本机 codex-cli 0.156.0；ChatGPT 登录；配置 Astra/high/default。
- 官方仓库 rust-v0.156.0 的 responses_websocket.rs 在 debug 级别记录升级响应头。
- 该版本对 response.model 与 openai-model 头使用不同处理；需直接采集服务端消息。
- 检测依据是响应元数据；不将它当作模型权重证明。

- 首次真实请求：升级头未包含 openai-model；两次 metadata 事件均含 safety-buffering-enabled=true 与 faster-model=gpt-5.6-luna。预热与正式回答 response.model 均为 gpt-6-astra。正式回答 17093 input / 5 output tokens。

- 实测窗口 2026-09-24T08:51:46.676Z 至 2026-09-24T08:58:33.407Z。8 次 Astra / 2 次 Luna，全部匹配。10 次升级握手均无 openai-model；全部 Astra 请求的 metadata.headers 出现 Luna faster-model 候选。
- 精确版本源码实际在 info 级别记录升级响应头（纠正此前的 debug 描述）。
- 正式请求报告 input=156423, output=4426, reasoning=1845。预热另计，不解释为实际计费。
