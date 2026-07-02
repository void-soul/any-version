# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

AnyVersion is a **Windows multi-language development environment version manager** built with Tauri 2 (Rust backend) + React 19 (TypeScript frontend). It manages SDK versions (Node.js, Java, Python, Go, etc.) — installing, switching, and cleaning up environment variables, caches, mirrors, and services.

## Build & Dev Commands

| Command | Description |
|---------|-------------|
| `pnpm install` | Install frontend dependencies |
| `pnpm dev` | Start Vite dev server (port 9999) |
| `pnpm tauri dev` | Full Tauri dev mode (auto-starts Vite on :9999) |
| `pnpm build` | Build frontend (tsc + vite build) |
| `pnpm tauri build` | Build production installer |
| `pnpm bump <x.y.z>` | Bump version across package.json, Cargo.toml, tauri.conf.json |

**Dev workflow**: Run `pnpm dev` in one terminal, then `pnpm tauri dev` in another (or just `pnpm tauri dev` which chains them).

## Architecture

### Two-Layer Structure

- **`src/`** — React/TypeScript frontend (Vite + Tailwind CSS 4)
- **`src-tauri/`** — Rust backend (Tauri 2 plugin architecture)

Communication is exclusively via Tauri's `invoke()` command system. The frontend never touches the filesystem or registry directly; all system operations are Rust commands invoked through `@tauri-apps/api/core`.

### Key Architectural Concepts

**`projects.json`** — Runtime manifest at repo root defining every SDK/tool the app can manage (Node.js, Java, Go, Python, databases, etc.). This is bundled into the app as a Tauri resource. The schema is documented in `docs/projects-json-schema.md`. Each entry (`ProjectDef`) contains version detection commands, download URL templates, find rules for PATH discovery, env vars to manage, cache/mirror configs, and optional inner package managers (e.g., npm/yarn/pnpm inside Node.js).

**Config persistence** — User config lives at `%USERPROFILE%\.any-version\config.json` (the `Config` struct in `commands/config.rs`). It tracks: managed items, active versions, delegations, PATH modifications, and original env backups for restoration.

**Symlink/Junction-based version switching** — Rather than modifying PATH repeatedly, the app installs versions to `versions_dir/<project>/<version>/` and creates Windows junctions in `links_dir/<project>/` pointing to the active version. PATH only needs the links directory once.

**Escalation architecture** — Many operations (registry writes, service management, env var changes) require admin rights. The Rust backend handles privilege escalation via Windows UAC prompts when needed.

### Rust Module Breakdown (`src-tauri/src/commands/`)

| Module | Responsibility |
|--------|---------------|
| `config.rs` | App config load/save, RSS |
| `env.rs` | Windows registry env vars (HKCU/HKLM), PATH management |
| `project/` | Core domain: registry loading, scanning, version management, commands |
| `project/types.rs` | All data types (ProjectDef, ProjectStatus, delegation, etc.) |
| `project/registry.rs` | Loads `projects.json` from bundled resource or disk |
| `project/scanner.rs` | Scans system for installed versions via find_rules |
| `project/versions.rs` | Install/switch/uninstall versions |
| `project/commands.rs` | Tauri command handlers for project operations |
| `service.rs` | Windows service start/stop/force-kill |
| `port.rs` | Port scanning, process killing |
| `cache.rs` | Cache directory size/migration |
| `mirror.rs` | Registry/mirror configuration |
| `pkg.rs` | Global package management (npm/pip/yarn) |
| `conflict.rs` | Detection of other version managers (nvm, pyenv, rustup) |
| `hidden_cmd.rs` | Hidden/special commands |
| `hosts.rs` | System hosts file editing |
| `http_server.rs` | Built-in HTTP server for tool features |
| `img_base64.rs` | Image Base64 conversion |
| `sdk_resolver.rs` | SDK version resolution logic |
| `utils.rs` | Shared utilities |

### Frontend Structure (`src/`)

- `App.tsx` — Top-level tab navigation (Projects / Tools / Settings) with custom window controls
- `components/project/` — Main feature: `ProjectManager`, `ProjectListPanel`, `ProjectDetailPanel`, `ProjectSubTabs`
- Other components map to tools: `CacheManager`, `MirrorManager`, `PkgManager`, `EnvBackupManager`, `PortScanner`, `PathEnvManager`, `RssReader`, `SystemTools`, `GlobalSettings`, `HttpServer`, `ImageBase64`, `TsPlayground`, `JsonUrlTools`

### Why `sync_process_path()` exists (`lib.rs`)

Windows apps with `windows_subsystem = "windows"` don't inherit the user PATH from the registry. The startup code manually rebuilds the process PATH from registry values (system + user) + current env, filtering out unmanaged installs that would shadow the managed junctions.

### Version Bumping

Use the script: `pnpm bump 2.4.7` (or `pnpm bump v2.4.7`). It updates all three version files atomically. Never edit version strings by hand.

## Platform Constraints

- **Windows-only** — Deep integration with Windows Registry (`winreg`), junction points, UAC, and Windows services
- The app uses a **borderless custom window** (`decorations: false`) with drag-region handling in React
- Single-instance enforcement via `tauri-plugin-single-instance`
- System tray for quick version switching without opening the main window
