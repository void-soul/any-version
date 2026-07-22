---
name: porting-from-reference-repos
description: Use when the user says "抄作业", "check reference repos", "port features", "copy homework", or asks to check what the reference repositories (EchoBird, cc-switch, CodexPlusPlus) have updated and what can be ported to the current project. Also use when the user mentions specific reference repo names and wants to sync features.
---

# Porting from Reference Repos (抄作业)

## Overview

Systematically check reference repositories for recent updates, identify valuable features, and port them to the current project while avoiding duplication of existing functionality.

## Reference Repos

| Repo | Path | Type | Architecture |
|------|------|------|-------------|
| **EchoBird** | `E:\pro\other-sdk\EchoBird` | AI Agent desktop tool | Tauri + Rust + React (TS) |
| **cc-switch** | `E:\pro\other-sdk\cc-switch` | Model switch/proxy tool | Tauri + Rust + React (TS) |
| **CodexPlusPlus** | `E:\pro\other-sdk\CodexPlusPlus` | Codex enhancement | — |

**Target project (any-version):** `e:\pro\my\any-version` — also Tauri + Rust + React (TS), shares architecture with EchoBird and cc-switch, making direct porting feasible.

## Workflow

### Phase 1: Discover (抄什么)

Check recent commits in each reference repo:

```bash
# For each reference repo
cd <repo-path>
git log --oneline --since="2 weeks ago" --no-pager | cat
git diff --stat HEAD~10..HEAD --no-pager | cat
```

Key areas to scan:
- New UI features (React components, state patterns)
- New backend commands (Rust `#[tauri::command]` functions)
- Configuration/settings changes
- Package.json dependency updates worth adopting
- CLI tool integration patterns
- Bug fixes and stability improvements

### Phase 2: Prioritize (抄什么优先级)

Rank features by:
1. **Direct portability** — same architecture (Tauri+Rust+React) = highest priority
2. **User-facing value** — visible features first, backend refinements second
3. **Low risk** — isolated features, no deep coupling to existing code
4. **Existing gaps** — features any-version is missing that reference repos have

### Phase 3: Audit (避免重复)

**CRITICAL**: Before implementing, audit what any-version already has:

```bash
# Search for similar feature names in any-version
cd e:\pro\my\any-version
rg -i "<feature-keyword>" src/ src-tauri/src/ --no-pager | cat
```

- Check both frontend (`src/`) and backend (`src-tauri/src/`)
- Search for similar Tauri commands, React components, config fields
- Skip features that already exist (user confirmation: "我们已经有了！")

### Phase 4: Implement (开抄)

For each feature to port, follow this order:

1. **Rust backend first** — add new structs, commands, and config fields
2. **React frontend second** — add components, state, and UI
3. **Wire together** — update `lib.rs` command registrations, ensure `invoke()` calls match

Key porting patterns:
- EchoBird's Tauri commands → port directly, adapt naming to any-version conventions
- cc-switch's React components → port with styling adaptation
- Both use TypeScript strict mode — maintain type safety

### Phase 5: Verify (检查)

```bash
cd e:\pro\my\any-version
# Rust compilation
cd src-tauri && cargo check 2>&1 | cat
# TypeScript compilation
cd .. && npx tsc --noEmit 2>&1 | cat
```

Both must pass with **zero errors** before declaring completion.

## Implementation Patterns

### Porting a Tauri Command (Rust)

```rust
// Reference: EchoBird or cc-switch command
// 1. Copy the #[tauri::command] fn and its structs
// 2. Adapt field names to any-version conventions
// 3. Register in lib.rs: .invoke_handler(tauri::generate_handler![..., new_command])
```

### Porting a React Component (TSX)

```tsx
// Reference: EchoBird or cc-switch component
// 1. Copy component structure and state logic
// 2. Adapt imports to any-version paths
// 3. Adapt styling (Tailwind classes or CSS modules)
// 4. Wire up invoke() calls to match Rust command names
```

### Common Pitfalls

| Pitfall | Fix |
|---------|-----|
| Porting a feature already exists | Phase 3 audit first |
| Tauri command signature mismatch | Check Rust fn name matches frontend `invoke()` string |
| Missing type definitions on frontend | Copy type interfaces alongside components |
| Cargo.toml missing new dependencies | Check reference's Cargo.toml for required crates |
| Environment-specific paths hardcoded | Replace absolute paths with relative or config-driven |

## Quick Reference

```bash
# Full discovery cycle (run from any-version root)
for repo in EchoBird cc-switch CodexPlusPlus; do
  echo "=== $repo ==="
  cd "E:\pro\other-sdk\$repo" && git log --oneline -20 | cat
done
cd e:\pro\my\any-version
```

## When NOT to Use

- When the task is a bug fix or feature that exists purely within any-version (no reference to copy from)
- When the user explicitly asks for original implementation
- When reference repos have no relevant recent changes
