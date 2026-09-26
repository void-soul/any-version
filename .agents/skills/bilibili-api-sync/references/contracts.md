# B站接口契约（自持基线）

> **来源与法律注记**：以下契约来自 any-version 收藏模块的既有实现
> （`src-tauri/src/commands/favorites/{wbi.rs,bilibili.rs}`）与其历史上的公开出处
> `SocialSisterYi/bilibili-API-collect`（该仓已于 2026-01-28 收到 B站委托律所的律师函后
> 永久关停，文档与源码已删除，默认分支只剩 `deprecated`）。
> 本文档是**本项目内部的维护基线**，非公开传播物：不要在仓库 README、注释外链或分发物中
> 再次引用/镜像上游全文。修订以「我们自己的测试 + 真实请求」为准。

契约版本：**v1（2026-09-26 固化）** —— 与 `wbi.rs` HEAD 一致，导入链路实测可用。

---

## 1. WBI 签名（2023-03 起 web 端风控）

### 1.1 步骤

1. `GET https://api.bilibili.com/x/web-interface/nav`（带 Cookie）→ `data.wbi_img.img_url` / `data.wbi_img.sub_url`
2. `key_from_url`：取 URL 最后一段、去掉扩展名 → `img_key` / `sub_key`
3. `mixin_key = shuffle(img_key + sub_key)`：拼接串按 `MIXIN_KEY_ENC_TAB` 取前 32 位重排
4. 待签参数 + `wts`（秒级时间戳）→ 按 **key 升序** → `url_encode` 后 `k=v` 以 `&` 连接
5. `w_rid = md5(query + mixin_key)`（32 位小写十六进制）
6. 最终 query：`{query}&w_rid={w_rid}`

### 1.2 混洗表（`wbi.rs::MIXIN_KEY_ENC_TAB`，长 64，用前 32）

```
46,47,18,2,53,8,23,32,15,50,10,31,58,3,45,35,27,43,5,49,33,9,42,19,29,
28,14,39,12,38,41,13,37,48,7,16,24,55,40,61,26,17,0,1,60,51,30,4,22,25,
54,21,56,59,6,63,57,62,11,36,20,34,44,52
```

### 1.3 编码规则（`url_encode`）

- 保留：`A-Za-z0-9` 与 `-_.~`
- **剔除**这 5 个字符：`! ' ( ) *`
- 其余按 UTF-8 字节转 `%XX`，**十六进制大写**
- **空格 → `%20`**（不是 `+`）—— 写成 `+` 会算出错误的 `w_rid`

### 1.4 官方测试向量（`wbi.rs` 单测固定，改动必炸）

```
img_key = 7cd084941338484aae1ad9425b84077c
sub_key = 4932caff0ff746eab6f01bf08b70ac45

mixin_key = ea1db124af3c7062474693fa704f4ff8

params: foo=114, bar=514, zab=1919810, wts=1702204169
signed  = bar=514&foo=114&wts=1702204169&zab=1919810&w_rid=8f6f2b5b3d485fe1886cec6a0be8c5d4
```

编码用例：`one one four` → `one%20one%20four`；`五一四` → `%E4%BA%94%E4%B8%80%E5%9B%9B`；`a!b'c(d)e*f` → `abcdef`。

---

## 2. 端点（收藏域，全部只读）

| 用途 | Method | URL | 参数 | 签名 |
|---|---|---|---|---|
| 登录态 + WBI 口令 | GET | `https://api.bilibili.com/x/web-interface/nav` | — | 否 |
| 我创建的收藏夹 | GET | `https://api.bilibili.com/x/v3/fav/folder/created/list-all` | `up_mid`, `type=2` | 是 |
| 收藏夹内容（分页） | GET | `https://api.bilibili.com/x/v3/fav/resource/list` | `media_id`, `pn`, `ps=20`（定义域 1-20） | 是 |

请求头（三者一致）：

```
User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36
Referer:    https://www.bilibili.com/
Cookie:     <用户粘贴的整串，含 SESSDATA>
```

**响应判错**：HTTP 仍是 200，业务错误在 body 的 `code` 里，必须单独判 `code != 0`。

常见业务码（见 `bilibili.rs::map_bili_code`）：

- `-101` 账号未登录 → 提示重新粘贴 Cookie
- `-403` 访问权限不足
- `0` 成功
- HTTP `412` → 风控拦截，提示稍后重试 / 降低频率（不是签名错）

---

## 3. 风控与节奏参数（账号安全，勿为性能调整）

| 参数 | 值 | 理由 |
|---|---|---|
| 页间隔 | 600ms | 收藏夹是**账号态接口**（Cookie + WBI），连发轻则 412、重则 Cookie 失效/封号。对比知乎 400ms，这里更保守 |
| 每页条数 | 20（接口上限） | 少发请求 |
| UA / Referer | 浏览器伪装 / `https://www.bilibili.com/` | B站对默认 UA 风控更严 |

---

## 4. 写操作（**当前未实现**，仅供评估）

社区历史上记录过的收藏夹资源操作端点（现已无官方文档可核）：

- `POST https://api.bilibili.com/medialist/gateway/coll/resource/deal`
- `POST https://api.bilibili.com/x/v3/fav/resource/deal`（老版）

认证：Cookie（`SESSDATA`）+ **`csrf` = Cookie 里的 `bili_jct`**；Cookie 方式下要求 `Referer` 在 `.bilibili.com` 域名下。

**风险评估（决定不做或延后的依据）**：

1. `bili_jct` 是 CSRF token，等同于"能改你收藏夹"的完整写权限，风险等级远高于只读导入
2. 写接口比读接口更容易触发验证码/风控，失败语义复杂（要区分"本地已删 / 平台未删"）
3. 上游因律师函关停，说明 B站对这类非官方调用持明确法律立场；写操作会把风险从"读"抬到"改账号数据"
4. 收藏模块 `mod.rs` 的硬规则仍写着"不反向操作平台"，做之前必须先改规则并让用户知情

---

## 5. 失效检测（不走 B站接口）

`check.rs`：B站条目与书签/知乎一样走 **HTTP 探测**（HEAD 为主，被拒回退带 `Range: bytes=0-0` 的 GET）。
判定：404/410 = 失效；403/429/5xx/超时 = **未知**（不把活页面误标成失效）。

---

## 6. 修订记录

| 日期 | 版本 | 变更 |
|---|---|---|
| 2026-09-26 | v1 | 首次固化：上游关停后把 WBI 算法、测试向量、三个端点、风控参数自持进本项目 |
