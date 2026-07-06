use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tauri::AppHandle;
use tauri::Emitter;
use crate::commands::ai_registry::{registry, AiToolDefDto, ToolConfig, PathConfig};
use crate::commands::config::get_base_dir;
use crate::commands::tool_version::is_newer;
use crate::commands::hidden_cmd;
use crate::commands::cache::{get_dir_size, format_bytes, create_junction, migrate_pkg_storage_impl, clean_pkg_cache_impl};
use super::models::*;


fn usage_path() -> PathBuf {
    get_base_dir().join("ai_usage.json")
}

fn load_usage() -> UsageFile {
    let path = usage_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(usage) = serde_json::from_str::<UsageFile>(&data) {
                return usage;
            }
        }
    }
    UsageFile::default()
}

fn save_usage(data: &UsageFile) -> Result<(), String> {
    let path = usage_path();
    let _ = fs::create_dir_all(path.parent().unwrap());
    let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

// ─── AI 配置 ───

#[tauri::command]
pub fn record_usage(tool_id: String, model: String, provider: Option<String>, input_tokens: u64, output_tokens: u64) -> Result<(), String> {
    let mut usage = load_usage();
    usage.records.push(UsageRecord {
        tool_id,
        model,
        provider,
        input_tokens,
        output_tokens,
        timestamp: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
    });
    save_usage(&usage)
}

#[tauri::command]
pub fn get_usage_summary() -> Result<UsageSummary, String> {
    let usage = load_usage();
    let total_records = usage.records.len() as u64;
    let total_input_tokens: u64 = usage.records.iter().map(|r| r.input_tokens).sum();
    let total_output_tokens: u64 = usage.records.iter().map(|r| r.output_tokens).sum();

    // by_tool
    let mut tool_map: HashMap<String, (u64, u64)> = HashMap::new();
    for r in &usage.records {
        let entry = tool_map.entry(r.tool_id.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += r.input_tokens + r.output_tokens;
    }
    let mut by_tool: Vec<UsageByTool> = tool_map
        .into_iter()
        .map(|(tool_id, (count, tokens))| UsageByTool { tool_id, request_count: count, total_tokens: tokens })
        .collect();
    by_tool.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));

    // by_model
    let mut model_map: HashMap<(String, String), (u64, u64)> = HashMap::new();
    for r in &usage.records {
        let key = (r.model.clone(), r.provider.clone().unwrap_or_default());
        let entry = model_map.entry(key).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += r.input_tokens + r.output_tokens;
    }
    let mut by_model: Vec<UsageByModel> = model_map
        .into_iter()
        .map(|((model, provider), (count, tokens))| UsageByModel { model, provider, request_count: count, total_tokens: tokens })
        .collect();
    by_model.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));

    // daily
    let mut daily_map: HashMap<String, (u64, u64)> = HashMap::new();
    for r in &usage.records {
        let date = &r.timestamp[..10];
        let entry = daily_map.entry(date.to_string()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += r.input_tokens + r.output_tokens;
    }
    let mut daily: Vec<UsageDaily> = daily_map
        .into_iter()
        .map(|(date, (count, tokens))| UsageDaily { date, request_count: count, total_tokens: tokens })
        .collect();
    daily.sort_by(|a, b| a.date.cmp(&b.date));

    Ok(UsageSummary {
        total_records,
        total_input_tokens,
        total_output_tokens,
        total_tokens: total_input_tokens + total_output_tokens,
        by_tool,
        by_model,
        daily,
    })
}

#[tauri::command]
pub fn clear_usage() -> Result<(), String> {
    save_usage(&UsageFile::default())
}
