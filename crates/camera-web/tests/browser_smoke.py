"""原生 Chrome + 本地 HTTP 的 WASM 页面冒烟测试（可选）。"""
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from tempfile import TemporaryDirectory
from threading import Thread
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[3]
CHROME = shutil.which("google-chrome") or "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"


class QuietHandler(SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


def main():
    if not Path(CHROME).exists():
        raise SystemExit("请先安装 Chrome，再运行浏览器冒烟测试")
    handler = partial(QuietHandler, directory=str(ROOT))
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
    Thread(target=server.serve_forever, daemon=True).start()
    try:
        url = f"http://127.0.0.1:{server.server_port}/crates/camera-web/www/"
        with TemporaryDirectory() as profile:
            command = [
                CHROME, "--headless", "--disable-gpu", "--disable-background-networking",
                "--no-first-run", f"--user-data-dir={profile}",
                "--virtual-time-budget=4000", "--dump-dom", url,
            ]
            try:
                result = subprocess.run(
                    command, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                    timeout=20, check=False,
                )
                html = result.stdout.decode("utf-8", errors="replace")
            except subprocess.TimeoutExpired as error:
                # 部分 macOS Chrome 构建在输出 DOM 后仍不退出；只检查实际 DOM。
                html = (error.stdout or b"").decode("utf-8", errors="replace")
        for text in ("候选记录：3", "已审核地图点位：0", "罗秀路出龙州路东约160米", "照片拍摄日期：未知"):
            assert text in html, f"页面没有渲染：{text}"
        assert "页面加载失败" not in html
        print("WASM 浏览器页面冒烟测试通过")
    finally:
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
