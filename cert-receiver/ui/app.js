// 证书接收端界面：只用全局注入的 Tauri API（tauri.conf 里 withGlobalTauri），
// 因此没有 npm 依赖、也没有前端构建链，改完直接 cargo build 就生效。

const T = window.__TAURI__;
const invoke = T?.core?.invoke;
const listen = T?.event?.listen;

const $ = (id) => document.getElementById(id);

/** 当前配置（与 Rust 侧 Config 同名字段，snake_case） */
let form = {};
/** 监听地址里的主机部分，界面上只让用户改端口 */
let host = "127.0.0.1";
let running = false;
let autoScroll = true;
let logs = [];
let logCounter = 0;
/** 上一次刷新状态时的成功推送次数，用来判断「收到新证书了」 */
let lastPushes = -1;

/* ── 渲染 ── */

function hint(text, kind = "") {
  const el = $("hint");
  el.textContent = text || "";
  el.className = "hint" + (kind ? " " + kind : "");
  if (!text) return;
  clearTimeout(hint._timer);
  hint._timer = setTimeout(() => {
    el.textContent = "";
    el.className = "hint";
  }, 6000);
}

/** 复制到剪贴板：剪贴板 API 被拒时退回 execCommand（WebView 里偶发权限失败）。 */
async function copyText(text) {
  if (!text) return false;
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    let ok = false;
    try {
      ok = document.execCommand("copy");
    } catch {
      /* 放弃 */
    }
    ta.remove();
    return ok;
  }
}

/* ── 拉取示例 ── */

let pullTimer = 0;

async function refreshPullList() {
  const sel = $("pull_cert");
  const prev = sel.value;
  let list = [];
  try {
    list = await invoke("list_stored_certs");
  } catch {
    /* 读不到就按空处理 */
  }
  sel.innerHTML = "";
  if (!list || list.length === 0) {
    const o = document.createElement("option");
    o.value = "";
    o.textContent = "（还没有收到证书）";
    sel.appendChild(o);
  } else {
    for (const c of list) {
      const o = document.createElement("option");
      o.value = c.certificate_id;
      const dom = (c.domains || []).slice(0, 2).join(", ");
      const exp = (c.expired_time || "").slice(0, 10);
      o.textContent = [c.certificate_id, dom, exp].filter(Boolean).join(" · ");
      sel.appendChild(o);
    }
    if (prev && list.some((c) => c.certificate_id === prev)) sel.value = prev;
  }
  await rebuildPull();
}

async function rebuildPull() {
  const curlInput = $("pull_curl");
  const note = $("pull-note");
  if (!$("api_key").value.trim()) {
    curlInput.value = "";
    note.textContent = "上面还没填 API 密钥：生成/填写后点「保存并应用」，拉取接口才会启用。";
    return;
  }
  try {
    const ex = await invoke("pull_example", {
      apiId: $("pull_api_id").value,
      certificateId: $("pull_cert").value,
    });
    curlInput.value = ex.curl;
    const hosts = (ex.hosts || []).filter(Boolean);
    if (ex.listen_is_any) {
      note.textContent =
        "监听 0.0.0.0：示例里写的是 127.0.0.1，请换成服务器实际地址" +
        (hosts.length ? `（本机网卡：${hosts.join(" / ")}）` : "") +
        "；timestamp 5 分钟内有效。";
    } else {
      note.textContent =
        "当前只监听本机，只有同机能访问；要给容器或其它机器用，把「监听地址」改成 0.0.0.0。timestamp 5 分钟内有效。";
    }
  } catch (e) {
    curlInput.value = "";
    note.textContent = String(e);
  }
}

function renderStatus(st) {
  running = !!st.running;
  const badge = $("status");
  badge.classList.toggle("running", running);
  $("status-text").textContent = running ? `运行中 ${st.addr}` : "未启动";
  $("toggle").textContent = running ? "停止" : "启动";
  $("toggle").disabled = false;

  // 收到新证书就刷新一次可选列表（拉取示例依赖它）
  if (st.pushes !== lastPushes) {
    lastPushes = st.pushes;
    refreshPullList().catch(() => {});
  }

  if (st.pushes > 0 || st.fails > 0) {
    const result = st.last_push_ok ? "成功" : "失败";
    const dot = st.last_push_ok ? "✅" : "❌";
    document.title = `${dot} 证书接收端 · 已收 ${st.pushes} 次`;
    $("card_stats")?.remove();
    const el = document.createElement("div");
    el.id = "card_stats";
    el.className = "path";
    const failText = st.fails > 0 ? ` · 失败 ${st.fails} 次` : "";
    el.textContent = `已接收 ${st.pushes} 次${failText} · 最后请求 ${st.last_push_at}（${result}）`;
    $("cfg-path").after(el);
  }
}

function renderLogs() {
  const box = $("logs");
  const nearBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 40;
  box.textContent = logs.join("\n");
  if (autoScroll && nearBottom) box.scrollTop = box.scrollHeight;
}

function pushLog(line) {
  logs.push(line);
  if (logs.length > 800) logs.shift();
  logCounter += 1;
  renderLogs();
}

function renderReloadExtra() {
  const mode = $("reload_mode").value;
  const box = $("reload-extra");
  box.innerHTML = "";

  /**
   * @param key      配置字段名
   * @param picker   "dir" | "file" | undefined —— 有的话在输入框后面加一个「选择…」
   */
  const add = (label, key, placeholder, picker) => {
    const l = document.createElement("label");
    l.textContent = label;
    const row = document.createElement("span");
    row.className = "row";

    const i = document.createElement("input");
    i.id = "f_" + key;
    i.value = form[key] ?? "";
    i.placeholder = placeholder ?? "";
    i.spellcheck = false;
    i.addEventListener("input", () => {
      form[key] = i.value;
    });
    row.appendChild(i);

    if (picker) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.textContent = "选择…";
      btn.addEventListener("click", async () => {
        try {
          const picked = await invoke(picker === "dir" ? "pick_dir" : "pick_file", {
            current: i.value,
          });
          if (!picked) return;
          i.value = picked;
          form[key] = picked;
          if (key === "nginx_prefix") guessNginxConf(picked);
        } catch (e) {
          hint(String(e), "err");
        }
      });
      row.appendChild(btn);
    }

    l.appendChild(row);
    box.appendChild(l);
  };

  if (mode === "command") {
    add("重载命令（cmd /c 执行）", "reload_command", "C:\\nginx\\nginx.exe -s reload");
  } else if (mode === "nginx") {
    add("Nginx 目录（含 nginx.exe）", "nginx_prefix", "C:\\nginx", "dir");
    add("配置文件", "nginx_conf", "C:\\nginx\\conf\\nginx.conf", "file");
  } else if (mode === "service") {
    add("Windows 服务名", "service_name", "nginx");
  }
  box.classList.toggle("hidden", mode === "none" || mode === "");
}

/** 监听 0.0.0.0 时把风险提示亮出来（只提示，不阻止 —— 服务器上就是要这么配）。 */
function renderHostWarn() {
  $("host-warn").classList.toggle("hidden", $("host").value !== "0.0.0.0");
}

/** 选完 Nginx 目录后顺手补全 conf 路径（只填空的，不覆盖用户手写的值）。 */
function guessNginxConf(prefix) {
  const input = $("f_nginx_conf");
  if (!input || input.value.trim()) return;
  const sep = prefix.includes("/") && !prefix.includes("\\") ? "/" : "\\";
  const guess = prefix.replace(/[\\/]+$/, "") + sep + "conf" + sep + "nginx.conf";
  input.value = guess;
  form.nginx_conf = guess;
  hint("已按所选目录补全配置文件路径，可再点「选择…」改成别的", "ok");
}

/* ── 表单 ── */

function fillForm(cfg) {
  form = { ...cfg };
  const addr = String(cfg.listen_addr || "127.0.0.1:9000");
  const idx = addr.lastIndexOf(":");
  if (idx > 0) host = addr.slice(0, idx);
  $("port").value = addr.slice(idx + 1).replace(/[^0-9]/g, "") || "9000";
  // 配置里如果是别的地址（比如指定了某块网卡），补一个选项进来，别让用户的值被悄悄改掉
  const hostSelect = $("host");
  if (!Array.from(hostSelect.options).some((o) => o.value === host)) {
    const o = document.createElement("option");
    o.value = host;
    o.textContent = host;
    hostSelect.appendChild(o);
  }
  hostSelect.value = host;
  renderHostWarn();
  $("token").value = cfg.token || "";
  $("api_key").value = cfg.api_key || "";
  $("output_dir").value = cfg.output_dir || "";
  $("layout").value = cfg.layout || "cert";
  $("reload_mode").value = cfg.reload_mode || "none";
  renderReloadExtra();
}

function collectConfig() {
  const port = Number($("port").value);
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error("监听端口必须是 1-65535 的整数");
  }
  return {
    ...form,
    listen_addr: `${$("host").value || host}:${port}`,
    token: $("token").value.trim(),
    api_key: $("api_key").value.trim(),
    output_dir: $("output_dir").value.trim(),
    layout: $("layout").value,
    reload_mode: $("reload_mode").value,
  };
}

function randomHex(bytes) {
  const buf = new Uint8Array(bytes);
  crypto.getRandomValues(buf);
  return Array.from(buf, (b) => b.toString(16).padStart(2, "0")).join("");
}

/* ── 事件绑定 ── */

function bindUi() {
  $("reload_mode").addEventListener("change", renderReloadExtra);
  $("host").addEventListener("change", renderHostWarn);

  $("gen-token").addEventListener("click", () => {
    $("token").value = randomHex(16);
    hint("已生成新的 Token，记得点「保存并应用」，并同步到 Kira 的节点配置", "ok");
  });

  $("toggle-token").addEventListener("click", () => {
    const el = $("token");
    const show = el.type === "password";
    el.type = show ? "text" : "password";
    $("toggle-token").textContent = show ? "隐藏" : "显示";
  });

  $("gen-api").addEventListener("click", () => {
    $("api_key").value = randomHex(16);
    hint("已生成新的 API 密钥，保存后拉取接口才生效", "ok");
  });

  $("pick").addEventListener("click", async () => {
    try {
      const dir = await invoke("pick_dir", { current: $("output_dir").value });
      if (dir) $("output_dir").value = dir;
    } catch (e) {
      hint(String(e), "err");
    }
  });

  $("open").addEventListener("click", async () => {
    try {
      await invoke("open_dir", { path: $("output_dir").value });
    } catch (e) {
      hint(String(e), "err");
    }
  });

  $("save").addEventListener("click", async () => {
    $("save").disabled = true;
    try {
      const cfg = collectConfig();
      const st = await invoke("save_config", { config: cfg });
      form = { ...cfg };
      renderStatus(st);
      await rebuildPull(); // api_key / 端口可能变了，签名示例要跟着更新
      hint("已保存并应用", "ok");
    } catch (e) {
      hint(String(e), "err");
    } finally {
      $("save").disabled = false;
    }
  });

  $("toggle-api").addEventListener("click", () => {
    const el = $("api_key");
    const show = el.type === "password";
    el.type = show ? "text" : "password";
    $("toggle-api").textContent = show ? "隐藏" : "显示";
  });

  $("copy-api").addEventListener("click", async () => {
    const ok = await copyText($("api_key").value.trim());
    hint(
      ok ? "已复制 API 密钥，可以贴给容器/脚本" : "复制失败，先点「显示」再手动选中复制",
      ok ? "ok" : "err",
    );
  });

  $("pull-refresh").addEventListener("click", () => rebuildPull());
  $("pull-copy").addEventListener("click", async () => {
    const ok = await copyText($("pull_curl").value);
    hint(ok ? "已复制 curl 命令" : "复制失败，请手动选中内容复制", ok ? "ok" : "err");
  });
  $("pull_cert").addEventListener("change", () => rebuildPull());
  $("pull_api_id").addEventListener("input", () => {
    clearTimeout(pullTimer);
    pullTimer = setTimeout(() => rebuildPull(), 300);
  });

  $("toggle").addEventListener("click", async () => {
    $("toggle").disabled = true;
    try {
      const st = await invoke("set_running", { running: !running });
      renderStatus(st);
      hint(running ? "服务已启动" : "服务已停止", "ok");
    } catch (e) {
      hint(String(e), "err");
      $("toggle").disabled = false;
    }
  });

  $("quit").addEventListener("click", async () => {
    await invoke("quit_app");
  });

  $("clear").addEventListener("click", async () => {
    logs = await invoke("clear_logs");
    renderLogs();
  });

  $("toggle-scroll").addEventListener("click", () => {
    autoScroll = !autoScroll;
    $("toggle-scroll").textContent = `自动滚动：${autoScroll ? "开" : "关"}`;
    if (autoScroll) {
      const box = $("logs");
      box.scrollTop = box.scrollHeight;
    }
  });
}

/* ── 启动 ── */

async function main() {
  if (!invoke) {
    document.body.innerHTML =
      '<pre style="padding:20px;color:#fb7185">未拿到 Tauri API（withGlobalTauri 未生效）</pre>';
    return;
  }

  bindUi();

  const boot = await invoke("bootstrap");
  $("ver").textContent = `v0.2.0`;
  $("cfg-path").textContent = `配置文件：${boot.cfg_path}`;
  fillForm(boot.config);
  logs = boot.logs || [];
  renderLogs();
  renderStatus(boot.status);

  await listen("receiver-log", (e) => pushLog(e.payload));
  await listen("receiver-status", (e) => renderStatus(e.payload));
}

main().catch((e) => {
  document.body.innerHTML = `<pre style="padding:20px;color:#fb7185">界面初始化失败：${e}</pre>`;
});
