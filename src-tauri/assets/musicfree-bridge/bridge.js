/**
 * MusicFree 插件宿主桥（常驻子进程）。
 *
 * 协议：stdin 一行一个 JSON 请求 `{id, method, params}`，stdout 一行一个 JSON 响应
 * `{id, ok, result|error}`。**stdout 只走协议**。
 *
 * 为什么是 Node 子进程，而不是内嵌 JS 引擎（QuickJS 等）：
 * 真实 MusicFree 插件是**标准 CommonJS**（实测 audiomack 插件顶部就是
 * `require("axios") / require("cheerio") / require("crypto-js") / require("dayjs")`）。
 * 在 Node 里 `require` 就是原生能力，配一个真实 `node_modules` 即可；
 * 换内嵌引擎反而要自己实现 axios/cheerio 等一整套宿主 API，且行为必然有差异。
 */
'use strict';

const readline = require('readline');

function safeStringify(value) {
  try {
    return JSON.stringify(value);
  } catch (_) {
    return String(value);
  }
}

// 插件里到处是 console.log —— 若走 stdout 会把协议流冲烂（Rust 侧按行解析 JSON）。
// 所以协议独占 stdout，插件日志一律改道 stderr。
for (const level of ['log', 'info', 'debug', 'warn', 'error']) {
  console[level] = (...args) => {
    process.stderr.write(
      '[plugin] ' + args.map((a) => (typeof a === 'string' ? a : safeStringify(a))).join(' ') + '\n'
    );
  };
}
// 插件里未捕获的异步错误同样不能污染 stdout
process.on('unhandledRejection', (e) => {
  process.stderr.write('[plugin] unhandledRejection: ' + ((e && e.stack) || e) + '\n');
});

let plugin = null;
let pluginPath = '';

function requirePlugin(path) {
  // 先校验参数，**再**动状态。
  // 顺序反了的话，一次畸形请求就会把已载入的插件清掉：`plugin = null` 之后
  // `require(undefined)` 抛错，于是「只是想 load 一下」变成了「把宿主搞瘫」，
  // 后续 search / mediaSource 全部报「尚未加载插件」，非常难查。
  if (typeof path !== 'string' || path.length === 0) {
    throw new Error('load 需要字符串参数 path');
  }
  if (pluginPath && pluginPath !== path) {
    // 换插件时必须清缓存，否则拿到的是上一个插件（同名不同内容时尤其坑）
    try {
      delete require.cache[require.resolve(pluginPath)];
    } catch (_) {
      /* 缓存里没有就算了 */
    }
    plugin = null;
  }
  pluginPath = path;
  const loaded = require(path);
  if (!loaded || typeof loaded !== 'object') {
    throw new Error('插件没有导出对象（module.exports 为空）');
  }
  plugin = loaded;
  return {
    platform: plugin.platform || '',
    version: plugin.version || '',
    srcUrl: plugin.srcUrl || '',
    author: plugin.author || '',
    description: plugin.description || '',
    defaultSearchType: plugin.defaultSearchType || 'music',
    supportedSearchType: plugin.supportedSearchType || ['music'],
    userVariables: plugin.userVariables || [],
    // 按能力报告：我们只用到 search / getMediaSource，缺了要能提前说清
    hasSearch: typeof plugin.search === 'function',
    hasMediaSource: typeof plugin.getMediaSource === 'function',
  };
}

function ensure(method) {
  if (!plugin) throw new Error('尚未加载插件');
  if (typeof plugin[method] !== 'function') throw new Error(`插件不支持 ${method}`);
}

async function dispatch(method, params) {
  switch (method) {
    case 'ping':
      return { pong: true };
    case 'load':
      return requirePlugin(params.path);
    case 'search': {
      ensure('search');
      const type = params.type || plugin.defaultSearchType || 'music';
      const result = await plugin.search(params.keyword, params.page || 1, type);
      return { isEnd: !!(result && result.isEnd), data: (result && result.data) || [] };
    }
    case 'mediaSource': {
      ensure('getMediaSource');
      return (await plugin.getMediaSource(params.item, params.quality || 'standard')) || null;
    }
    default:
      throw new Error(`未知方法: ${method}`);
  }
}

const rl = readline.createInterface({ input: process.stdin });
rl.on('line', async (line) => {
  const text = line.trim();
  if (!text) return;
  let req;
  try {
    req = JSON.parse(text);
  } catch (_) {
    return; // 非法请求行直接丢弃：与其写坏协议，不如让调用方超时后自己发现
  }
  try {
    const result = await dispatch(req.method, req.params || {});
    process.stdout.write(JSON.stringify({ id: req.id, ok: true, result }) + '\n');
  } catch (e) {
    process.stdout.write(
      JSON.stringify({ id: req.id, ok: false, error: (e && e.message) || String(e) }) + '\n'
    );
  }
});
rl.on('close', () => process.exit(0));
