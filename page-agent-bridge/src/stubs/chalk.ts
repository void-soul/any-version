/**
 * chalk 桩：page-agent 仅用颜色方法做控制台着色。
 * headless 自动化里日志经 console 事件转发到 Rust 侧，无需 ANSI 颜色。
 * 每个方法同时可作模板标签使用（chalk.gray`text`）。
 */
const id = (s) => (typeof s === 'string' ? s : s.join(''))

const level = () => {
	const fn = (s) => (typeof s === 'string' ? s : s.join(''))
	return Object.assign(fn, { bold: fn, dim: fn, italic: fn })
}

export default Object.assign(level(), {
	blue: level(),
	cyan: level(),
	gray: level(),
	green: level(),
	red: level(),
	yellow: level(),
})
