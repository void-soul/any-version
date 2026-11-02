// Real Vite watcher fixture: cyclic skill junctions must not suppress normal source changes.
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, symlink, unlink, rm, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, sep, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer, loadConfigFromFile } from "vite";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const loaded = await loadConfigFromFile({ command: "serve", mode: "test" }, join(repo, "vite.config.ts"));
assert(loaded?.config.server?.watch, "Development watcher settings must exist");
const watch = loaded.config.server.watch;
assert.equal(watch.followSymlinks, false);
const tempRoot = await realpath(tmpdir());
const temporary = await mkdtemp(join(tempRoot, "kira-watcher-fixture-"));
const links = [];
let server;
function deadline(promise, label) {
  let timer;
  return Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} timed out`)), 8000);
  })]).finally(() => clearTimeout(timer));
}
try {
  const root = join(temporary, "project");
  const first = join(temporary, "library", "qa-log");
  const second = join(temporary, "library", "secure-git-commit");
  await Promise.all([mkdir(join(root, "src"), { recursive: true }), mkdir(first, { recursive: true }), mkdir(second, { recursive: true })]);
  const link = async (target, path) => {
    await mkdir(dirname(path), { recursive: true });
    await symlink(target, path, process.platform === "win32" ? "junction" : "dir");
    links.push(path);
  };
  await link(second, join(first, ".agents", "skills", "secure-git-commit"));
  await link(first, join(second, ".agents", "skills", "qa-log"));
  const mounts = [".agents", ".codebuddy", ".claude", ".workbuddy", ".openclaw"];
  for (const mount of mounts) await link(first, join(root, mount, "skills", "qa-log"));
  // A link outside ignored directories also must not recurse into the dependency cycle.
  await link(first, join(root, "linked-skill"));
  await mkdir(join(root, "src-tauri"));
  await writeFile(join(root, "src-tauri", "ignored.rs"), "// ignored");
  const source = join(root, "src", "example.ts");
  await writeFile(source, "export const value = 1;\n");
  const errors = [];
  let markReady;
  const ready = new Promise((resolve) => { markReady = resolve; });
  server = await createServer({
    root, configFile: false, appType: "custom", logLevel: "silent",
    optimizeDeps: { noDiscovery: true, include: [] },
    server: { watch, middlewareMode: true, hmr: false, ws: false },
    plugins: [{ name: "watcher-fixture", configureServer(instance) {
      instance.watcher.on("error", (error) => errors.push(error));
      instance.watcher.once("ready", markReady);
    } }],
  });
  await deadline(ready, "Initial watcher scan");
  const watched = Object.keys(server.watcher.getWatched()).map((path) => path.replaceAll("\\", "/"));
  const rootPath = root.replaceAll("\\", "/");
  assert(watched.includes(`${rootPath}/src`), "Frontend source directory must be watched");
  for (const directory of [...mounts, "src-tauri", "linked-skill"]) {
    assert(!watched.some((path) => path === `${rootPath}/${directory}` || path.startsWith(`${rootPath}/${directory}/`)), `Unexpected recursion into ${directory}`);
  }
  const changed = new Promise((resolve) => server.watcher.on("change", (path) => {
    if (resolvePath(path) === resolvePath(source)) resolve();
  }));
  await writeFile(source, "export const value = 'changed frontend source';\n");
  await deadline(changed, "Frontend source change");
  assert.equal(errors.length, 0, errors.map(String).join("\n"));
  console.log("Vite watcher passed: cyclic skill mounts excluded; links not followed; frontend source changes observed.");
} finally {
  await server?.close();
  // Remove only our links, before removing the owned fixture tree.
  for (const path of links.reverse()) await unlink(path);
  assert(temporary.startsWith(tempRoot + sep), "Unexpected cleanup target");
  await rm(temporary, { recursive: true, force: true });
}
function resolvePath(path) { return resolve(path).toLowerCase(); }
