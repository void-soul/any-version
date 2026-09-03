// @ts-check
/**
 * Kira Page 模块桥构建脚本：
 * 将 page-agent（E:\pro\other-sdk\page-agent 源码，monorepo TS 源）以 IIFE 打包为
 * 单文件浏览器内运行时（dist/page-agent-bridge.iife.js），由 Rust 端经 CDP
 * `Page.addScriptToEvaluateOnNewDocument` 注入目标页面。
 *
 * 关键点：
 * - 直接 alias 到 page-agent 各包的 TS 源码（不安装其 monorepo 依赖）
 * - chalk 桩：仅颜色函数，无副作用
 * - mask/motion 桩：enableMask:false 时运行期不触达，这里保证 Vite 静态/动态
 *   import 分析均可解析，不把 ai-motion 拉进产物
 */
import { build } from 'vite'
import { fileURLToPath } from 'url'
import { dirname, resolve } from 'path'

const __dirname = dirname(fileURLToPath(import.meta.url))
const here = (p) => resolve(__dirname, p)

/** page-agent 源码根（用户环境固定路径，与 README 约定一致） */
const PAGE_AGENT_ROOT = 'E:/pro/other-sdk/page-agent'

const noopStub = resolve(__dirname, 'src/stubs/noop.ts')
const chalkStub = resolve(__dirname, 'src/stubs/chalk.ts')
// zod 从 page-agent 包目录解析（其 exports 在那边被 pnpm/monorepo 布局遮蔽），直接指向安装产物
const zodV4 = resolve(__dirname, 'node_modules/zod/v4/index.js')
const zodMain = resolve(__dirname, 'node_modules/zod/index.js')

await build({
	root: __dirname,
	publicDir: false,
	resolve: {
		alias: [
			// —— page-agent 包：直接指向 monorepo 的 TS 源码 ——
			{ find: '@page-agent/core', replacement: resolve(PAGE_AGENT_ROOT, 'packages/core/src/PageAgentCore.ts') },
			{ find: '@page-agent/page-controller', replacement: resolve(PAGE_AGENT_ROOT, 'packages/page-controller/src/PageController.ts') },
			{ find: '@page-agent/llms', replacement: resolve(PAGE_AGENT_ROOT, 'packages/llms/src/index.ts') },
			{ find: /^@page-agent\/ui$/, replacement: noopStub },
			// —— 无关运行时依赖打桩 ——
			// page-controller 内部动态 import('./mask/SimulatorMask')，按相对说明符打桩
			//（rolldown 下 plugin resolveId 不触发，alias 是被验证有效的机制）
			{ find: './mask/SimulatorMask', replacement: noopStub },
			{ find: /^ai-motion$/, replacement: noopStub },
			{ find: /^chalk$/, replacement: chalkStub },
			{ find: /^zod\/v4$/, replacement: zodV4 },
			{ find: /^zod$/, replacement: zodMain },
			{ find: /^page-agent$/, replacement: resolve(PAGE_AGENT_ROOT, 'packages/page-agent/src/PageAgent.ts') },
		],
	},
	plugins: [],
	build: {
		outDir: resolve(__dirname, 'dist'),
		emptyOutDir: true,
		target: 'chrome120',
		// Vite 8 requires a separately installed esbuild package for this mode.
		// Keep the prototype bundle dependency-light and unminified.
		minify: false,
		lib: {
			entry: resolve(__dirname, 'src/entry.ts'),
			name: 'KiraPageAgentBridge',
			formats: ['iife'],
			fileName: () => 'page-agent-bridge.iife.js',
		},
		rollupOptions: {
			onwarn(message, handler) {
				// page-agent 运行时在沙箱里构造 Function（execute_javascript 工具），属预期行为
				if (message.code === 'EVAL') return
				handler(message)
			},
		},
	},
})

console.log('[page-agent-bridge] 构建完成: dist/page-agent-bridge.iife.js')
