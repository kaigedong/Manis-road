# Astra 路由核验

目标：实际发送请求，检查 Astra 是否在响应模型字段或路由头中变为 Luna。

1. 环境与采集点核验：complete
2. Node.js 白名单脱敏采集器与首次请求：complete
3. Astra 不同推理级别/任务与 Luna 对照：complete
4. 头部/传输验证：complete（原生 CLI 采集到 10 个升级响应头和 20 个服务端 metadata 事件，无需代理）
5. 汇总请求级证据、结论和限制：complete

结果：8 次 Astra 正式请求、2 次 Luna 对照以及 10 次预热均完成；0 次响应模型标识不一致。8/8 Astra 带 Luna faster-model 候选标记。

排错记录：model_provider_info.rs URL 返回 404，改为核对当前 tag 目录和已有响应实现；5 个非 JSON 小帧无法按 JSON 解析，已保留帧形状，全部正式响应生命周期和模型字段均完整。
