import { readFileSync, writeFileSync } from 'fs';
import { resolve } from 'path';

let version = process.argv[2];

if (version && version.startsWith('v')) {
  version = version.slice(1);
}

if (!version || !/^\d+\.\d+\.\d+/.test(version)) {
  console.error('用法: node scripts/bump-version.js <x.y.z> 或 <vx.y.z>');
  process.exit(1);
}

const files = [
  { path: resolve('package.json'), type: 'json' },
  { path: resolve('src-tauri/Cargo.toml'), type: 'toml' },
  { path: resolve('src-tauri/tauri.conf.json'), type: 'json' },
];

for (const { path, type } of files) {
  let content = readFileSync(path, 'utf-8');
  const before = content;

  if (type === 'json') {
    const data = JSON.parse(content);
    data.version = version;
    content = JSON.stringify(data, null, 2) + '\n';
  } else if (type === 'toml') {
    content = content.replace(/^version = "[^"]*"/m, `version = "${version}"`);
  }

  writeFileSync(path, content, 'utf-8');
  console.log(`✓ ${path.replace(resolve() + '/', '')}: ${before.match(/version["']?\s*[:=]\s*["'][^"']+["']/)?.[0] ?? 'N/A'} → ${version}`);
}

console.log(`\n已统一更新版本号为 ${version}`);
