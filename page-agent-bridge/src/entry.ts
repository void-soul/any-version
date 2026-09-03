/**
 * Kira Page 模块 · 浏览器内桥入口（IIFE，由 Rust 经 CDP 注入目标页面）
 *
 * 协议：
 * - 出站（页面 → Rust）：window 事件 `kira-page-agent-event`，detail 为 JSON 字符串
 *   {type:'log',line} | {type:'activity',activity} | {type:'step',event}
 *   | {type:'status',status} | {type:'result',success,data} | {type:'error',message}
 * - 入站（Rust → 页面）：window 全局函数
 *   __kiraPageAgentStart(cfg, task) / __kiraPageAgentStop() / __kiraPageAgentDispose()
 *
 * 设计：
 * - 每个任务用全新的 PageAgentCore + PageController（enableMask:false，不加载遮罩层）
 * - execute_javascript 工具对本地自动化场景风险可控（一次性临时 profile），
 *   但为稳妥默认关闭，用户可在面板中开启
 * - LLM 请求发生在页面上下文：headless 启动参数带 --disable-web-security 规避 CORS
 */
import { PageAgentCore } from '@page-agent/core'
import { PageController } from '@page-agent/page-controller'

/** 出站：统一 JSON 字符串 detail 的 window 事件（Rust 侧 Runtime.addBinding 接收） */
function emit(payload) {
	const encoded = JSON.stringify(payload)
	try {
		// CDP Runtime.addBinding installs this function before the page is navigated.
		// Keep the call optional so the bridge remains usable in a normal browser page.
		if (typeof window.__kiraPageAgentEmit === 'function') {
			window.__kiraPageAgentEmit(encoded)
		}
	} catch {
		/* binding may disappear while the page is unloading */
	}
	try {
		window.dispatchEvent(
			new CustomEvent('kira-page-agent-event', { detail: encoded })
		)
	} catch {
		/* 页面正在卸载等场景：静默 */
	}
}

/** 受控 console 转发：限流，避免巨型日志打爆 CDP */
const MAX_LINE = 2000
let logCount = 0
function log(line) {
	if (logCount > 500) return
	logCount++
	const s = String(line)
	emit({ type: 'log', line: s.length > MAX_LINE ? s.slice(0, MAX_LINE) + '…' : s })
}

/** 每步最多保留的步骤事件数（防止超长任务内存膨胀，历史事件对 LLM 由 agent 自管理） */
function wireAgent(agent) {
	agent.addEventListener('activity', (e) => {
		emit({ type: 'activity', activity: e.detail })
	})
	agent.addEventListener('statuschange', () => {
		emit({ type: 'status', status: agent.status })
	})
	agent.addEventListener('historychange', () => {
		const last = agent.history[agent.history.length - 1]
		if (!last) return
		if (last.type === 'step') {
			// raw 请求/响应体量过大，前端不展示，剔除后发送
			const { rawResponse: _r, rawRequest: _q, ...step } = last
			emit({ type: 'step', event: step })
		} else if (last.type === 'error') {
			emit({ type: 'log', line: '[agent-error] ' + String(last.message).slice(0, MAX_LINE) })
		} else if (last.type === 'retry') {
			emit({ type: 'log', line: `[retry] ${last.attempt}/${last.maxAttempts}` })
		}
	})
}

let pendingAnswer = null

async function start(cfg, task) {
	logCount = 0
	try {
		const pageController = new PageController({ enableMask: false })
		const agent = new PageAgentCore({
			pageController,
			baseURL: cfg.baseURL,
			model: cfg.model,
			apiKey: cfg.apiKey,
			maxSteps: cfg.maxSteps ?? 25,
			stepDelay: cfg.stepDelay ?? 0.4,
			language: cfg.language ?? 'zh-CN',
			experimentalScriptExecutionTool: cfg.allowScript === true,
			instructions: cfg.instructions ? { system: cfg.instructions } : undefined,
		})
		wireAgent(agent)

		window.__kiraPageAgentStop = () => agent.stop()
		window.__kiraPageAgentDispose = () => agent.dispose()
		// Login walls and ambiguous tasks are handed to the user without blocking the
		// browser's event loop. The Page panel displays the question and calls Answer
		// after the user has signed in or completed the requested action.
		agent.onAskUser = async (question) => {
			emit({ type: 'ask_user', question: String(question) })
			return await new Promise((resolve) => {
				pendingAnswer = resolve
			})
		}

		const result = await agent.execute(task)
		emit({ type: 'result', success: result.success === true, data: String(result.data ?? '') })
	} catch (e) {
		emit({ type: 'error', message: String(e?.message ?? e) })
	}
}

window.__kiraPageAgentAnswer = (answer) => {
	if (pendingAnswer) {
		const resolve = pendingAnswer
		pendingAnswer = null
		resolve(String(answer ?? ''))
	}
}

window.__kiraPageAgentStart = (cfg, task) => {
	// 不 await：Rust 侧通过事件流感知进度与结果
	start(cfg, task).catch((e) => emit({ type: 'error', message: String(e?.message ?? e) }))
}

log('[bridge] kira page-agent bridge ready')
