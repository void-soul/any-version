use super::*;

// ---- 外观：顶级模块主题色 + 全局字体 ----

/// 供前端读取外观配置（模块主题色 + 全局字体 + 自定义字体 + 模块顺序）
#[tauri::command]
pub fn get_appearance_config() -> AppearanceConfig {
    let config = load_config();
    AppearanceConfig {
        module_theme_colors: config.module_theme_colors,
        global_font: config.global_font,
        custom_font_path: config.custom_font_path,
        module_order: config.module_order,
        toolbar_modules: config.toolbar_modules,
        disabled_modules: config.disabled_modules,
        background_texture: config.background_texture,
        language: config.language,
        motion_preference: config.motion_preference,
        density: config.density,
    }
}

/// 保存顶级模块的自定义顺序（模块 id 列表，空 = 恢复默认顺序）
#[tauri::command]
pub fn set_module_order(order: Vec<String>) -> Result<(), String> {
    mutate_config(|config| {
        config.module_order = order;
        Ok(())
    })
}

/// 保存模块布局：顶栏模块列表 + 禁用模块列表（所有模块平级，用户自由归类）。
pub fn set_module_layout(
    toolbar_modules: Vec<String>,
    disabled_modules: Vec<String>,
) -> Result<(), String> {
    mutate_config(|config| {
        config.toolbar_modules = toolbar_modules;
        config.disabled_modules = disabled_modules;
        Ok(())
    })
}

/// 设置单个顶级模块的主题色（hex，如 #8b5cf6）
#[tauri::command]
pub fn set_module_theme_color(module_id: String, color: String) -> Result<(), String> {
    mutate_config(|config| {
        if color.trim().is_empty() {
            config.module_theme_colors.remove(&module_id);
        } else {
            config.module_theme_colors.insert(module_id, color);
        }
        Ok(())
    })
}

/// 设置全局字体（CSS font-family 名称，空=恢复默认）
#[tauri::command]
pub fn set_global_font(font: String) -> Result<(), String> {
    mutate_config(|config| {
        config.global_font = font;
        Ok(())
    })
}

/// 设置全局背景底图纹理（grid / dots / scanline / aurora / solid；空=默认网格）。
#[tauri::command]
pub fn set_background_texture(texture: String) -> Result<(), String> {
    mutate_config(|config| {
        config.background_texture = texture;
        Ok(())
    })
}

/// 设置界面语言（"zh" / "en"；空=跟随系统）。
/// 语言变更后立即重建托盘菜单（菜单文案随语言切换）。
#[tauri::command]
pub fn set_language(language: String) -> Result<(), String> {
    mutate_config(|config| {
        config.language = language;
        Ok(())
    })?;
    let _ = crate::tray::rebuild_tray_menu_global();
    Ok(())
}

/// 导入自定义字体文件：把 src 拷贝到数据目录 fonts/，返回字体家族名与目标路径
#[tauri::command]
pub fn import_custom_font(src: String) -> Result<CustomFontInfo, String> {
    let src_path = Path::new(&src);
    if !src_path.exists() {
        return Err("字体文件不存在".to_string());
    }
    let ext = src_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .filter(|e| matches!(e.as_str(), "ttf" | "otf" | "woff" | "woff2"))
        .unwrap_or_default();
    if ext.is_empty() {
        return Err("仅支持 .ttf / .otf / .woff / .woff2 字体文件".to_string());
    }

    let fonts_dir = get_data_dir().join("fonts");
    let _ = fs::create_dir_all(&fonts_dir);

    let file_name = src_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("custom_font")
        .to_string();
    // 家族名用文件名去掉扩展名，并替换非法字符
    let family = file_name
        .trim_end_matches(&format!(".{ext}"))
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();

    let dest = fonts_dir.join(&file_name);
    fs::copy(src_path, &dest).map_err(|e| format!("拷贝字体失败: {e}"))?;

    mutate_config(|config| {
        config.custom_font_path = dest.to_string_lossy().to_string();
        config.global_font = family.clone();
        Ok(())
    })?;

    Ok(CustomFontInfo {
        family,
        path: dest.to_string_lossy().to_string(),
        ext,
    })
}

/// 移除自定义字体（恢复默认字体）
#[tauri::command]
pub fn clear_custom_font() -> Result<(), String> {
    let mut previous = String::new();
    mutate_config(|config| {
        previous = std::mem::take(&mut config.custom_font_path);
        config.global_font.clear();
        Ok(())
    })?;
    if !previous.is_empty() {
        let _ = fs::remove_file(previous);
    }
    Ok(())
}

/// 枚举系统已安装的字体家族名（供前端「全局字体」下拉动态填充）。
///
/// 合并读取两个字体注册表位置，避免漏掉「仅当前用户安装」的字体：
/// - `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts`（为所有用户安装）
/// - `HKCU\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts`（为当前用户安装）
/// 值名形如 `Arial (TrueType)`，去掉括号后缀与扩展名得到家族名，去重排序返回。
#[tauri::command]
pub fn list_system_fonts() -> Result<Vec<String>, String> {
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        use winreg::RegKey;

        const FONTS_SUBKEY: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts";
        let mut families: Vec<String> = Vec::new();

        for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            let key = RegKey::predef(root).open_subkey(FONTS_SUBKEY);
            let key = match key {
                Ok(k) => k,
                Err(_) => continue, // 该位置不存在/无权访问则跳过
            };
            for name in key.enum_values().filter_map(|v| v.ok()).map(|(n, _)| n) {
                let fam = strip_font_suffix(&name);
                if !fam.is_empty() && !families.contains(&fam) {
                    families.push(fam);
                }
            }
        }
        families.sort();
        Ok(families)
    }
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }
}

/// 从字体注册表值名里提取家族名：去掉「(xxx)」后缀、去掉 Regular/Bold/Italic 等字重后缀。
pub(super) fn strip_font_suffix(name: &str) -> String {
    let trimmed = name.trim();
    // 去掉末尾括号及其内容，如 "Arial (TrueType)" -> "Arial"
    let no_paren = match trimmed.rfind('(') {
        Some(idx) => trimmed[..idx].trim_end().to_string(),
        None => trimmed.to_string(),
    };
    // 去掉末尾的常见字重/样式标记（Regular / Bold / Italic / Light / Medium ...）
    const STYLE_SUFFIXES: &[&str] = &[
        " Regular",
        " Bold",
        " Italic",
        " Bold Italic",
        " Light",
        " Medium",
        " SemiBold",
        " Semibold",
        " Black",
        " Heavy",
        " Thin",
        " ExtraLight",
        " ExtraBold",
        " UltraLight",
        " SemiLight",
        " DemiBold",
        " Demi",
    ];
    let mut fam = no_paren;
    for s in STYLE_SUFFIXES {
        if let Some(stripped) = fam.strip_suffix(s) {
            fam = stripped.to_string();
            break;
        }
    }
    // 去掉扩展名（部分字体值名带 .ttf/.ttc/.otf）
    for ext in [".ttf", ".ttc", ".otf", ".TTF", ".TTC", ".OTF"] {
        if let Some(stripped) = fam.strip_suffix(ext) {
            fam = stripped.to_string();
            break;
        }
    }
    fam.trim().to_string()
}

/// 自定义字体信息
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CustomFontInfo {
    pub family: String,
    pub path: String,
    pub ext: String,
}

/// 外观配置（模块主题色 + 全局字体 + 模块顺序 + 模块布局）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceConfig {
    pub module_theme_colors: std::collections::HashMap<String, String>,
    pub global_font: String,
    pub custom_font_path: String,
    pub module_order: Vec<String>,
    pub toolbar_modules: Vec<String>,
    pub disabled_modules: Vec<String>,
    /// 全局背景底图纹理。
    pub background_texture: String,
    /// 界面语言（"zh" / "en"）。
    pub language: String,
    /// 动效偏好：`"always"` = 无视系统「减少动效」；其余（含空）= 跟随系统。
    pub motion_preference: String,
    /// 列表密度：`"compact"` = 紧凑；其余（含空）= 舒适。
    pub density: String,
}

/// 设置动效偏好（`always` = 始终开启动效；其它值 = 跟随系统）。
///
/// 存在意义：Windows 被切成「最佳性能」（或开了「减少动效」）时，
/// `prefers-reduced-motion` 会变成 reduce，CSS 里那条无障碍兜底会把头像辉光闪烁、
/// 弹窗淡入这类自家动效全部关掉 —— 而用户往往并不知道系统被切过。
#[tauri::command]
pub fn set_motion_preference(preference: String) -> Result<(), String> {
    mutate_config(|config| {
        config.motion_preference = normalize_motion_preference(&preference);
        Ok(())
    })
}

/// 归一化动效偏好：只有 `always` 表示「无视系统设置」，其余（空 / `system` / 未知值）都为「跟随系统」。
pub(super) fn normalize_motion_preference(preference: &str) -> String {
    if preference.trim().eq_ignore_ascii_case("always") {
        "always".to_string()
    } else {
        String::new()
    }
}

/// 设置列表密度（`compact` = 紧凑；其它值 = 舒适）。
///
/// 只存一个开关值、具体像素交给前端 CSS 变量（`--density-py`）决定：
/// 密度是纯视觉手感，后端不该知道 8px / 4px 这种数。
#[tauri::command]
pub fn set_density(density: String) -> Result<(), String> {
    mutate_config(|config| {
        config.density = normalize_density(&density);
        Ok(())
    })
}

/// 归一化密度：只有 `compact` 是紧凑，其余（空 / `comfortable` / 未知值）都回落到舒适。
pub(super) fn normalize_density(density: &str) -> String {
    if density.trim().eq_ignore_ascii_case("compact") {
        "compact".to_string()
    } else {
        String::new()
    }
}
