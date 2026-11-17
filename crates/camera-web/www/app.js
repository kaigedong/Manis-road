import init, { render_catalog } from "../pkg/camera_web.js";

const app = document.getElementById("app");

try {
  const paths = [
    "../../../var/camera/catalog.json", // 已采集目录优先
    "../../camera-catalog/data/shanghai-cameras.json", // 尚未采集时退回种子资料
  ];
  let catalogJson;
  for (const path of paths) {
    const response = await fetch(path, { cache: "no-store" });
    if (response.ok) {
      catalogJson = await response.text();
      break;
    }
    if (response.status !== 404) throw new Error(`读取 ${path} 失败（HTTP ${response.status}）`);
  }
  if (catalogJson === undefined) throw new Error("未找到资料目录");
  await init();
  // 文本从 Rust/WASM 输出；render_catalog 会先校验数据并转义所有非可信字符串。
  app.innerHTML = render_catalog(catalogJson);
} catch (error) {
  app.textContent = `页面加载失败：${error instanceof Error ? error.message : String(error)}`;
}
