#!/usr/bin/env python3
"""
Agent 文档自动更新工具 — 独立的研发辅助脚本

用法:
  python scripts/update_agent_docs.py              # 更新全部 Agent 的文档
  python scripts/update_agent_docs.py claude-code  # 更新指定 Agent
  python scripts/update_agent_docs.py --status      # 查看所有 Agent 状态
  python scripts/update_agent_docs.py --manifest    # 仅重新加载/验证 manifest
  python scripts/update_agent_docs.py --serve       # 启动本地 Web 文档浏览服务

数据类型 (docsSource.type):
  llms_txt            — 直接 GET llms.txt（Claude Code, Cline, Augment 等）
  documentation_md    — GitHub raw markdown（Gemini, Qwen, Hermes 等）
  readme_only         — 单个 README.md（OpenCode, Aider, Goose 等）
  documentation_scrape — HTML 页面（Cursor 等，回退到 README）
  documentation_index — HTML 索引页（Codex, Lingma 等）
"""

import argparse
import hashlib
import json
import os
import re
import sys
import time
from dataclasses import dataclass, field, asdict
from datetime import datetime
from pathlib import Path
from typing import Optional
from urllib.parse import urljoin

try:
    import requests
except ImportError:
    sys.exit("缺少 requests 库: pip install requests")

# ─── 路径 ──────────────────────────────────────────────

SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = SCRIPT_DIR.parent
DOCS_DIR = PROJECT_ROOT / "docs" / "agent-doc"
MANIFEST_PATH = SCRIPT_DIR / "agent_doc_manifest.json"
STATE_PATH = PROJECT_ROOT / "agent-docs-state.json"

# ─── HTTP 工具 ─────────────────────────────────────────

def fetch_url(url: str, timeout: int = 30) -> Optional[str]:
    """GET 一个 URL 并返回文本内容，失败返回 None。"""
    try:
        r = requests.get(url, timeout=timeout,
                         headers={"User-Agent": "AnyVersion-AgentDocsUpdater/1.0"})
        r.raise_for_status()
        return r.text
    except Exception as e:
        return None


def fetch_url_bytes(url: str, timeout: int = 30) -> Optional[bytes]:
    """GET 一个 URL 并返回字节内容。"""
    try:
        r = requests.get(url, timeout=timeout,
                         headers={"User-Agent": "AnyVersion-AgentDocsUpdater/1.0"})
        r.raise_for_status()
        return r.content
    except Exception:
        return None


# ─── llms.txt 格式解析 ─────────────────────────────────

def parse_llms_index(content: str) -> tuple[str, list[str]]:
    """Parse llms.txt 格式，返回 (标题, [(路径, 显示标题), ...])。
    路径可以是相对路径或完整 URL。"""
    title = ""
    pages = []
    for line in content.splitlines():
        line = line.strip()
        if line.startswith("# ") and not title:
            title = line[2:].strip()
        if not line or line.startswith("#") or line.startswith(">"):
            continue
        page = line.split("|")[0].strip()

        # 跳过列表项（如 "- [Title](url)"）
        if page.startswith("- "):
            m = re.match(r"\[(.*?)\]\((.+?)\)", page[2:])
            if m:
                page_url = m.group(2)
                page_title = m.group(1)
                # 完整 URL
                if page_url.startswith("http"):
                    pages.append((page_url, page_title))
                # 相对路径
                else:
                    pages.append((page_url.lstrip("/"), page_title))
            continue

        if page:
            if page.startswith("http"):
                pages.append((page, ""))
            elif not page.startswith("- "):
                pages.append((page, ""))
    return title, pages


# ─── GitHub markdown 解析 ──────────────────────────────

def parse_markdown_toc(content: str) -> list[str]:
    """尝试从 markdown 内容中提取目录/链接列表。"""
    pages = []
    # 提取 markdown 链接 [text](url)
    for m in re.finditer(r'\[([^\]]+)\]\(([^)]+)\)', content):
        url = m.group(2)
        if url.startswith("http") or url.startswith("#"):
            continue
        if "/" in url or url.endswith((".md", ".mdx")):
            pages.append(url)
    return pages


# ─── HTML 页面解析 ─────────────────────────────────────

def parse_html_for_links(content: str, base_url: str) -> list[str]:
    """从 HTML 中提取内部链接。"""
    links = set()
    # 匹配 href="..." 或 href='...'
    for m in re.finditer(r'href=["\']([^"\']+)["\']', content):
        url = m.group(1)
        if url.startswith(("#", "javascript:", "mailto:", "//")):
            continue
        if "docs." in url or "/docs/" in url:
            links.add(url)
        elif url.startswith("/"):
            links.add(urljoin(base_url, url))
    return list(links)


# ─── 核心：根据类型选择不同的获取策略 ────────────────

def get_docs_from_llms_txt(docs_source: dict) -> Optional[tuple[str, list[str]]]:
    """类型: llms_txt — 直接 GET llms.txt，解析出页面列表。"""
    content = fetch_url(docs_source["indexUrl"])
    if not content:
        return None
    title, pages = parse_llms_index(content)
    return title, pages


def get_docs_from_markdown(docs_source: dict) -> Optional[tuple[str, list[str]]]:
    """类型: documentation_md — 从 GitHub raw markdown 解析目录。"""
    content = fetch_url(docs_source.get("indexUrl", docs_source.get("url", "")))
    if not content:
        return None
    title = ""
    # 第一个 # 标题作为 title
    m = re.search(r'^#\s+(.+)', content, re.MULTILINE)
    if m:
        title = m.group(1).strip()
    pages = parse_markdown_toc(content)
    return title, pages


def get_docs_from_readme(docs_source: dict) -> tuple[str, list[str]]:
    """类型: readme_only — 单个 README 文件，没有子页面列表。"""
    url = docs_source.get("url", docs_source.get("indexUrl", ""))
    content = fetch_url(url)
    if not content:
        return "README", []
    title = ""
    m = re.search(r'^#\s+(.+)', content, re.MULTILINE)
    if m:
        title = m.group(1).strip()
    return title, [("README", url)]  # 特殊标记


def get_docs_from_scrape(docs_source: dict) -> Optional[tuple[str, list[str]]]:
    """类型: documentation_scrape — 从 HTML 文档站提取链接。"""
    content = fetch_url(docs_source["indexUrl"])
    if not content:
        # 回退到 README
        return get_docs_from_readme(docs_source)
    title = ""
    m = re.search(r'<title[^>]*>([^<]+)</title>', content, re.I)
    if m:
        title = m.group(1).strip()
    links = parse_html_for_links(content, docs_source.get("baseUrl", docs_source["indexUrl"]))
    return title, links


def get_docs_from_index(docs_source: dict) -> Optional[tuple[str, list[str]]]:
    """类型: documentation_index — HTML 索引页。"""
    return get_docs_from_scrape(docs_source)


# ─── 策略分发 ─────────────────────────────────────────

DOC_FETCHERS = {
    "llms_txt": get_docs_from_llms_txt,
    "documentation_md": get_docs_from_markdown,
    "readme_only": get_docs_from_readme,
    "documentation_scrape": get_docs_from_scrape,
    "documentation_index": get_docs_from_index,
}


# ─── 保存文件 ─────────────────────────────────────────

def save_file(local_dir: Path, filename: str, content: str):
    """保存文件到本地目录，自动创建父目录。"""
    if not filename.endswith((".md", ".mdx", ".txt")):
        filename += ".md"
    # 清理路径中的非法字符
    filename = re.sub(r'[<>:"|?*]', '_', filename)
    # 清理前导 ./
    filename = filename.lstrip("./")
    file_path = local_dir / filename
    file_path.parent.mkdir(parents=True, exist_ok=True)
    file_path.write_text(content, encoding="utf-8")
    return file_path


def download_and_save_url(local_dir: Path, url: str, title: str = "") -> bool:
    """从 URL 下载内容并保存。根据 URL 推导文件名。"""
    content = fetch_url(url)
    if content is None:
        return False

    # 推导文件名
    # 去掉查询参数
    clean_url = url.split("?")[0].split("#")[0]
    filename = clean_url.split("/")[-1]
    if not filename or filename in ("/", ""):
        filename = "index.md"

    # 如果 URL 不含 .md 后缀且没有扩展名，加 .md
    if "." not in Path(filename).name.split("/")[-1]:
        filename += ".md"
    
    # 处理标题作为文件名（如果更清晰）
    if title and not filename.endswith((".md", ".mdx")):
        filename = re.sub(r'[^\w\s-]', '', title).strip().replace(" ", "-").lower() + ".md"

    save_file(local_dir, filename, content)
    return True


def save_readme_file(local_dir: Path, url: str) -> Optional[str]:
    """下载并保存 README 文件。"""
    content = fetch_url(url)
    if not content:
        return None
    readme_path = local_dir / "README.md"
    local_dir.mkdir(parents=True, exist_ok=True)
    readme_path.write_text(content, encoding="utf-8")
    return str(readme_path)


# ─── Manifest / State (复用之前的逻辑) ───────────────

def load_manifest() -> dict:
    if not MANIFEST_PATH.exists():
        sys.exit(f"清单文件不存在: {MANIFEST_PATH}")
    with open(MANIFEST_PATH, "r", encoding="utf-8") as f:
        return json.load(f)


def load_state() -> dict:
    if STATE_PATH.exists():
        with open(STATE_PATH, "r", encoding="utf-8") as f:
            return json.load(f)
    return {"agent_states": {}, "last_global_check": None}


def save_state(state: dict):
    with open(STATE_PATH, "w", encoding="utf-8") as f:
        json.dump(state, f, ensure_ascii=False, indent=2)


def parse_manifest(data: dict) -> list[dict]:
    """从 manifest JSON 解析 Agent 配置列表。"""
    agents = []
    for a in data.get("agents", []):
        src = a.get("docsSource", {})
        agents.append({
            "id": a["id"],
            "display_name": a.get("displayName", a["id"]),
            "category": a.get("category", "unknown"),
            "docs_source": src,
            "local_dir": a.get("localDir", a["id"]),
            "description": a.get("description", ""),
            "supports_protocols": a.get("supportsProtocols", []),
            "priority": a.get("priority", 99),
        })
    return agents


# ─── 状态计算 ─────────────────────────────────────────

def count_md_files(path: Path) -> int:
    if not path.exists():
        return 0
    return sum(1 for item in path.rglob("*") if item.is_file() and item.suffix in (".md", ".mdx"))


def dir_size_bytes(path: Path) -> int:
    if not path.exists():
        return 0
    return sum(item.stat().st_size for item in path.rglob("*") if item.is_file())


def format_size(n: int) -> str:
    if n < 1024:
        return f"{n} B"
    if n < 1024 * 1024:
        return f"{n / 1024:.1f} KB"
    return f"{n / (1024 * 1024):.1f} MB"


def now_iso() -> str:
    return datetime.now().strftime("%Y-%m-%dT%H:%M:%S")


# ─── 单个目录文字 (AgentDocStatus) ────────────────────

@dataclass
class AgentDocStatus:
    agent_id: str
    display_name: str
    local_dir: str
    description: str
    category: str
    supports_protocols: list = field(default_factory=list)
    installed: bool = False
    local_pages: int = 0
    remote_pages: Optional[int] = None
    local_size_bytes: int = 0
    last_updated: Optional[str] = None
    last_check: Optional[str] = None
    update_available: bool = False
    error: Optional[str] = None


# ─── 核心：更新单个 Agent ─────────────────────────────

def update_agent(agent: dict) -> AgentDocStatus:
    """根据 docsSource.type 自动选择合适的策略，下载/刷新 Agent 文档。"""
    status = AgentDocStatus(
        agent_id=agent["id"],
        display_name=agent["display_name"],
        local_dir=agent["local_dir"],
        description=agent["description"],
        category=agent["category"],
        supports_protocols=agent["supports_protocols"],
    )

    try:
        source = agent["docs_source"]
        stype = source.get("type", "readme_only")
        fetcher = DOC_FETCHERS.get(stype)
        if not fetcher:
            raise RuntimeError(f"未知类型: {stype}")

        index_url = source.get("indexUrl", source.get("url", ""))
        print(f"  📥 获取索引: {index_url}")
        result = fetcher(source)
        if result is None:
            raise RuntimeError(f"获取索引失败（{fetcher.__name__}）")

        title, pages = result
        status.remote_pages = len(pages)
        print(f"  📄 发现 {len(pages)} 个页面")

        local_dir = DOCS_DIR / agent["local_dir"]
        local_dir.mkdir(parents=True, exist_ok=True)

        # 特殊处理 readme_only: 直接下载 README
        if stype == "readme_only":
            url = source.get("url", source.get("indexUrl", ""))
            if download_and_save_url(local_dir, url, "README"):
                print(f"  ✓ 保存 README")
            else:
                raise RuntimeError(f"下载 README 失败: {url}")
        else:
            # 逐个页面下载
            for i, page_info in enumerate(pages):
                # 兼容字符串和元组
                if isinstance(page_info, tuple):
                    page, page_title = page_info
                else:
                    page, page_title = page_info, ""

                # 完整 URL
                if page.startswith("http"):
                    url = page
                    # 推导本地文件名
                    clean = url.split("?")[0].split("#")[-1]
                    filename = clean.split("/")[-1]
                    if not filename:
                        filename = "index.md"
                    elif not filename.endswith((".md", ".mdx", ".txt")):
                        filename += ".md"
                else:
                    # 相对路径
                    if source.get("pagesUrlTemplate"):
                        url = source["pagesUrlTemplate"].replace("{page}", page)
                    else:
                        base = source.get("baseUrl", source.get("indexUrl", "")).rstrip("/")
                        url = f"{base}/{page}"
                    filename = page if not page.startswith("/") else page[1:]
                    if not filename.endswith((".md", ".mdx", ".txt")):
                        filename += ".md"

                # 如果有标题且文件名不清晰，使用标题
                if page_title and (filename == "index.md" or not filename):
                    filename = re.sub(r'[^\w\s-]', '', page_title).strip().replace(" ", "-").lower() + ".md"

                content = fetch_url(url)
                if content is not None:
                    save_file(local_dir, filename, content)

                if (i + 1) % 10 == 0 or i == len(pages) - 1:
                    print(f"  ⏳ 进度: {i + 1}/{len(pages)}")

        now = now_iso()
        status.installed = True
        status.local_pages = count_md_files(local_dir)
        status.local_size_bytes = dir_size_bytes(local_dir)
        status.last_updated = now
        status.last_check = now
        status.update_available = False
        print(f"  ✓ 完成: {status.local_pages} 页, {format_size(status.local_size_bytes)}")

    except Exception as e:
        status.error = str(e)
        status.last_check = now_iso()
        print(f"  ✗ 失败: {e}")

    return status


# ─── CLI 命令 ──────────────────────────────────────────

def cmd_status(manifest: dict):
    entries = parse_manifest(manifest)
    state = load_state()

    print(f"\n{'─' * 90}")
    print(f"  Agent 文档状态  (共 {len(entries)} 个 Agent)")
    print(f"{'─' * 90}")
    print(f"  {'名称':<20s} {'状态':<10s} {'本地页数':>8s} {'远程页数':>8s} {'大小':>10s} {'最后更新':<20s}")
    print(f"{'─' * 90}")

    for entry in entries:
        local_dir = DOCS_DIR / entry["local_dir"]
        installed = local_dir.exists()
        local_pages = count_md_files(local_dir) if installed else 0
        local_size = dir_size_bytes(local_dir) if installed else 0
        agent_state = state.get("agent_states", {}).get(entry["id"], {})
        remote_pages = agent_state.get("remote_pages")
        last_updated = agent_state.get("last_updated", "")
        update_available = agent_state.get("update_available", False)
        status_icon = "✓ 已安装" if installed else "○ 未安装"
        if update_available:
            status_icon += " 🔄"

        print(f"  {entry['display_name']:<20s} {status_icon:<12s} "
              f"{local_pages:>6d}   {str(remote_pages) if remote_pages is not None else '?':>6s}   "
              f"{format_size(local_size) if installed else '-':>10s} "
              f"{(last_updated or '-')[:19]:<20s}")

    print(f"{'─' * 90}\n")


def cmd_update(manifest: dict, agent_id: str = None):
    entries = parse_manifest(manifest)
    if agent_id:
        entries = [e for e in entries if e["id"] == agent_id]
        if not entries:
            sys.exit(f"未知 Agent: {agent_id}")

    print(f"\n开始更新 {len(entries)} 个 Agent 的文档...\n")
    state = load_state()

    for i, entry in enumerate(entries):
        print(f"[{i+1}/{len(entries)}] {entry['display_name']} ({entry['id']})")
        status = update_agent(entry)
        agent_state = state.setdefault("agent_states", {}).setdefault(entry["id"], {})
        agent_state.update(asdict(status))
        state["last_global_check"] = now_iso()
        save_state(state)
        print()

    print("✓ 全部完成！")


def cmd_serve(manifest: dict, host: str = "127.0.0.1", port: int = 8765):
    try:
        from http.server import HTTPServer, SimpleHTTPRequestHandler
        import urllib.parse
    except ImportError:
        sys.exit("标准库异常")

    entries = parse_manifest(manifest)
    state = load_state()

    class DocHandler(SimpleHTTPRequestHandler):
        def do_GET(self):
            if self.path == "/" or self.path == "/index.html":
                self.send_response(200)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.end_headers()
                html = generate_index_html(entries, state)
                self.wfile.write(html.encode("utf-8"))
            elif self.path == "/api/status":
                self.send_response(200)
                self.send_header("Content-Type", "application/json; charset=utf-8")
                self.end_headers()
                json.dump(self._get_full_state(entries), self.wfile, ensure_ascii=False)
            elif self.path.startswith("/api/open?"):
                parsed = urllib.parse.urlparse(self.path)
                params = urllib.parse.parse_qs(parsed.query)
                aid = params.get("id", [""])[0]
                d = DOCS_DIR / aid
                if d.exists():
                    os.startfile(str(d))
                self.send_response(200)
                self.end_headers()
            else:
                super().do_GET()

        def _get_full_state(self, entries):
            result = []
            for entry in entries:
                local_dir = DOCS_DIR / entry["local_dir"]
                result.append({
                    "agent_id": entry["id"],
                    "display_name": entry["display_name"],
                    "description": entry["description"],
                    "category": entry["category"],
                    "supports_protocols": entry["supports_protocols"],
                    "installed": local_dir.exists(),
                    "local_pages": count_md_files(local_dir) if local_dir.exists() else 0,
                    "local_size": dir_size_bytes(local_dir) if local_dir.exists() else 0,
                    "last_updated": state.get("agent_states", {}).get(entry["id"], {}).get("last_updated"),
                    "update_available": state.get("agent_states", {}).get(entry["id"], {}).get("update_available", False),
                })
            return result

        def log_message(self, fmt, *args):
            pass

    server = HTTPServer((host, port), DocHandler)
    print(f"\n🌐 文档浏览服务已启动: http://{host}:{port}")
    print(f"   按 Ctrl+C 停止\n")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\n已停止")
        server.server_close()


def generate_index_html(entries: list[dict], state: dict) -> str:
    rows_html = ""
    for entry in sorted(entries, key=lambda e: e["priority"]):
        local_dir = DOCS_DIR / entry["local_dir"]
        local_pages = count_md_files(local_dir)
        local_size = dir_size_bytes(local_dir)
        agent_state = state.get("agent_states", {}).get(entry["id"], {})
        last_updated = agent_state.get("last_updated", "")[:19] if agent_state.get("last_updated") else "-"
        update_available = agent_state.get("update_available", False)
        badge = "已安装" if local_dir.exists() else "未安装"
        if update_available:
            badge += " <span style='color:#f59e0b'>●</span>"

        rows_html += f"""
        <tr>
          <td><strong>{entry['display_name']}</strong>
            <br><small style="color:#94a3b8">{entry['description']}</small>
          </td>
          <td style="color:#94a3b8">{entry['category']}</td>
          <td style="color:{'#10b981' if local_dir.exists() else '#64748b'}">{badge}</td>
          <td style="text-align:right">{local_pages}</td>
          <td style="text-align:right">{format_size(local_size)}</td>
          <td style="color:#94a3b8">{last_updated}</td>
          <td>
            <button onclick="openDir('{entry['id']}')" style="background:#334155;border:1px solid #475569;color:#e2e8f0;padding:4px 10px;border-radius:4px;cursor:pointer;font-size:11px">
              打开目录
            </button>
          </td>
        </tr>"""

    return f"""<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<title>Agent 文档中心</title>
<style>
  body {{ background:#0d111d; color:#e2e8f0; font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif; margin:0; padding:20px; }}
  h1 {{ color:#e2e8f0; font-size:18px; margin:0 0 16px 0; }}
  .stats {{ display:flex; gap:12px; margin-bottom:16px; font-size:12px; color:#94a3b8; }}
  table {{ width:100%; border-collapse:collapse; font-size:12px; }}
  th {{ text-align:left; padding:8px 12px; font-size:11px; text-transform:uppercase; letter-spacing:0.5px; color:#64748b; border-bottom:1px solid #1e293b; }}
  td {{ padding:8px 12px; border-bottom:1px solid #1e293b; }}
  tr:hover td {{ background:rgba(139,92,246,0.05); }}
  button:hover {{ background:#475569 !important; }}
</style>
</head>
<body>
<h1>📚 Agent 文档中心</h1>
<div class="stats">
  <span>共 {len(entries)} 个 Agent</span>
  <span>{sum(1 for e in entries if (DOCS_DIR / e['local_dir']).exists())} 已安装</span>
</div>
<table>
<thead>
  <tr><th>Agent</th><th>分类</th><th>状态</th><th style="text-align:right">页数</th><th style="text-align:right">大小</th><th>最后更新</th><th>操作</th></tr>
</thead>
<tbody>
{rows_html}
</tbody>
</table>
<script>
function openDir(id) {{ fetch('/api/open?id=' + encodeURIComponent(id)); }}
</script>
</body>
</html>"""


# ─── 入口 ──────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description="Agent 文档自动更新工具")
    parser.add_argument("agent_id", nargs="?", help="仅更新指定 Agent ID")
    parser.add_argument("--status", action="store_true", help="显示所有 Agent 文档的状态摘要")
    parser.add_argument("--manifest", action="store_true", help="显示清单中的 Agent 列表")
    parser.add_argument("--serve", action="store_true", help="启动本地 Web 文档浏览服务")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    args = parser.parse_args()

    manifest = load_manifest()

    if args.manifest:
        entries = parse_manifest(manifest)
        print(f"\n清单包含 {len(entries)} 个 Agent:")
        for e in entries:
            src = e.get("docs_source", {})
            print(f"  [{e['priority']:2d}] {e['id']:<20s} — {e['display_name']:<18s} "
                  f"({src.get('type', '?')}) [{e['category']}]")
        print()
        return

    if args.status:
        cmd_status(manifest)
        return

    if args.serve:
        cmd_serve(manifest, args.host, args.port)
        return

    # 默认: 更新文档
    cmd_update(manifest, args.agent_id)


if __name__ == "__main__":
    main()
