import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
    coverage: {
      provider: "v8",
      reporter: ["text", "json-summary"],
      // 不自动清空报告目录：部分 IDE/沙箱的批量删除守卫会拦截 vitest 的
      // clean()，导致覆盖率运行在写入报告前失败。报告按文件名覆盖即可。
      clean: false,
      cleanOnRerun: false,
      // 只统计业务源码；测试文件、i18n 资源、类型声明不参与分母。
      include: ["src/**/*.{ts,tsx}"],
      exclude: [
        "src/**/__tests__/**",
        "src/**/*.test.{ts,tsx}",
        "src/**/*.d.ts",
        "src/i18n/locales/**",
        "src/vite-env.d.ts",
      ],
      // 2026-11-04 实测基线：49.05% statements / 36.01% branches /
      // 35.29% functions / 50.35% lines。阈值取略低于基线，防止继续下滑；
      // 每完成一批整改应上调，而不是下调。
      thresholds: {
        statements: 45,
        branches: 32,
        functions: 31,
        lines: 46,
      },
    },
  },
});
