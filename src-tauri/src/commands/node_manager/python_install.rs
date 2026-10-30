use super::*;

/// pip 包模式安装/升级：直接从 PyPI 安装（可带 extras），无需 git clone。
///
/// 解释器选择复用 [`select_python_runtime`]：支持 venv 就建 `.venv` 装进去（隔离、卸载即删目录）；
/// 只有嵌入式 Python（无 venv 但有 pip）时降级 `--target .deps`，并把解释器/依赖路径记录给启动侧。
pub(super) fn pip_package_install(
    app: &tauri::AppHandle,
    def: &NodeProjectDef,
    dir: &Path,
    upgrade: bool,
) -> Result<(), String> {
    let spec = def.pip_package.trim().to_string();
    if spec.is_empty() {
        return Err("该项目未配置 pipPackage".to_string());
    }
    fs::create_dir_all(dir).map_err(|e| format!("创建托管目录失败: {}", e))?;

    emit_progress(app, &def.id, "install", "正在检测可用的 Python 解释器…");
    let probes = probe_python_candidates();
    match select_python_runtime(&probes) {
        PythonRuntimePlan::Venv(program, prefix) => {
            let venv_py = def.venv_python();
            if !venv_py.exists() {
                emit_progress(app, &def.id, "install", "正在创建 Python 虚拟环境 (.venv)…");
                let mut args = prefix.clone();
                args.extend(["-m".to_string(), "venv".to_string(), ".venv".to_string()]);
                let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
                let (ok, last_err, _out) = run_capture_live(
                    app,
                    &def.id,
                    "install",
                    &program,
                    &arg_refs,
                    Some(dir),
                    &[],
                    None,
                );
                if !ok {
                    let msg = last_err.trim();
                    return Err(format!(
                        "创建虚拟环境失败: {}",
                        if msg.is_empty() { "未知错误" } else { msg }
                    ));
                }
            }
            if !venv_py.exists() {
                return Err("虚拟环境创建后未找到 python 解释器".to_string());
            }
            emit_progress(
                app,
                &def.id,
                "install",
                &format!("正在 pip install {} …", spec),
            );
            let py = venv_py.to_string_lossy().to_string();
            let args = pip_install_args(&spec, &def.pip_extra_packages, None, upgrade);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let (ok, last_err, _out) = run_capture_live(
                app,
                &def.id,
                "install",
                &py,
                &arg_refs,
                Some(dir),
                &[],
                None,
            );
            if !ok {
                let msg = last_err.trim();
                return Err(format!(
                    "pip install 失败: {}",
                    if msg.is_empty() { "未知错误" } else { msg }
                ));
            }
        }
        PythonRuntimePlan::TargetDeps(program, prefix) => {
            emit_progress(
                app,
                &def.id,
                "install",
                &format!(
                    "当前 Python（{}）不带 venv 模块（常见于嵌入式发行版），改用 .deps 目录安装…",
                    program
                ),
            );
            let mut args = prefix.clone();
            args.extend(pip_install_args(
                &spec,
                &def.pip_extra_packages,
                Some(".deps"),
                upgrade,
            ));
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let (ok, last_err, _out) = run_capture_live(
                app,
                &def.id,
                "install",
                &program,
                &arg_refs,
                Some(dir),
                &[],
                None,
            );
            if !ok {
                let msg = last_err.trim();
                return Err(format!(
                    "pip install 失败: {}",
                    if msg.is_empty() { "未知错误" } else { msg }
                ));
            }
            record_deps_runtime(dir, &program, &prefix)?;
        }
        PythonRuntimePlan::None => {
            return Err(
                "未找到支持 venv 或 pip 的 Python。请安装官方 Python 3.8+（python.org），\
                 或在本应用 SDK 页为托管 Python 执行 install_pip 后重试"
                    .to_string(),
            )
        }
    }

    let marker = serde_json::json!({ "package": spec });
    fs::write(def.pip_marker_path(), marker.to_string())
        .map_err(|e| format!("写入安装标记失败: {}", e))?;
    Ok(())
}

/// Python 项目安装。
///
/// 首选 `venv`（隔离、卸载即删目录）；解释器不带 venv 模块时（典型：嵌入式/embeddable
/// 发行版，如 SDK 管理器装的 Python），降级为 `pip install --target .deps`，
/// 启动时通过 PYTHONPATH / ._pth 注入依赖路径。
pub(super) fn python_install(
    app: &tauri::AppHandle,
    def: &NodeProjectDef,
    dir: &Path,
) -> Result<(), String> {
    emit_progress(app, &def.id, "install", "正在检测可用的 Python 解释器…");
    let probes = probe_python_candidates();
    match select_python_runtime(&probes) {
        PythonRuntimePlan::Venv(program, prefix) => {
            let mut args = prefix.clone();
            args.extend(["-m".to_string(), "venv".to_string(), ".venv".to_string()]);
            emit_progress(app, &def.id, "install", "正在创建 Python 虚拟环境 (.venv)…");
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let (ok, last_err, _out) = run_capture_live(
                app,
                &def.id,
                "install",
                &program,
                &arg_refs,
                Some(dir),
                &[],
                None,
            );
            if !ok {
                let msg = last_err.trim();
                return Err(format!(
                    "创建虚拟环境失败: {}",
                    if msg.is_empty() { "未知错误" } else { msg }
                ));
            }
            let py = def.venv_python();
            if !py.exists() {
                return Err("虚拟环境创建后未找到 python 解释器".to_string());
            }
            emit_progress(
                app,
                &def.id,
                "install",
                "正在安装 requirements.txt 依赖（pip）…",
            );
            let py_s = py.to_string_lossy().to_string();
            let (ok, last_err, _out) = run_capture_live(
                app,
                &def.id,
                "install",
                &py_s,
                &["-m", "pip", "install", "-r", "requirements.txt"],
                Some(dir),
                &[],
                None,
            );
            if !ok {
                let msg = last_err.trim();
                return Err(format!(
                    "pip install 失败: {}",
                    if msg.is_empty() { "未知错误" } else { msg }
                ));
            }
            Ok(())
        }
        PythonRuntimePlan::TargetDeps(program, prefix) => {
            emit_progress(
                app,
                &def.id,
                "install",
                &format!(
                    "当前 Python（{}）不带 venv 模块（常见于嵌入式发行版），改用 .deps 目录隔离安装依赖…",
                    program
                ),
            );
            let mut args = prefix.clone();
            args.extend([
                "-m".to_string(),
                "pip".to_string(),
                "install".to_string(),
                "-r".to_string(),
                "requirements.txt".to_string(),
                "--target".to_string(),
                ".deps".to_string(),
                "--upgrade".to_string(),
            ]);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let (ok, last_err, _out) = run_capture_live(
                app,
                &def.id,
                "install",
                &program,
                &arg_refs,
                Some(dir),
                &[],
                None,
            );
            if !ok {
                let msg = last_err.trim();
                return Err(format!(
                    "pip install 失败: {}",
                    if msg.is_empty() { "未知错误" } else { msg }
                ));
            }
            record_deps_runtime(dir, &program, &prefix)?;
            Ok(())
        }
        PythonRuntimePlan::None => Err(
            "未找到支持 venv 或 pip 的 Python。请安装官方 Python 3.8+（python.org），\
             或在本应用 SDK 页为托管 Python 执行 install_pip 后重试"
                .to_string(),
        ),
    }
}
