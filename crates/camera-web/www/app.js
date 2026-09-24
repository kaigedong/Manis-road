import init, { render_catalog } from "../pkg/camera_web.js";

const app = document.getElementById("app");

try {
  const response = await fetch("../../camera-catalog/data/shanghai-cameras.json", {
    cache: "no-store",
  });
  if (!response.ok) throw new Error(`读取资料失败（HTTP ${response.status}）`);
  const catalogJson = await response.text();
  await init();
  // 文本从 Rust/WASM 输出；render_catalog 会先校验数据并转义所有非可信字符串。
  app.innerHTML = render_catalog(catalogJson);
} catch (error) {
  app.textContent = `页面加载失败：${error instanceof Error ? error.message : String(error)}`;
}
