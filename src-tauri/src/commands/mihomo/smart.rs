//! Smart 内核覆写管理（1:1 对齐 clash-party src/main/config/smartOverride.ts）
//!
//! 使用 mihomo-smart 内核时自动注入一个全局 JS 覆写，把 url-test / load-balance
//! 策略组转换为 smart 类型；若不存在此类组则创建 "Smart Group" 并改写规则目标。

use super::config::{override_content_path, save_override_config, OverrideItem};
use super::MihomoInner;
use std::sync::Arc;

pub const SMART_OVERRIDE_ID: &str = "smart-core-override";

/// 生成 Smart 覆写脚本（模板参数与 clash-party 一致）
pub fn smart_override_template(
    use_lightgbm: bool,
    collect_data: bool,
    strategy: &str,
    collector_size: i64,
) -> String {
    let tpl = r#"
// 配置会在启用 Smart 内核时自动应用

function main(config) {
  try {
    if (!config || typeof config !== 'object') {
      console.log('[Smart Override] Invalid config object')
      return config
    }

    if (!config.profile) {
      config.profile = {}
    }
    config.profile['smart-collector-size'] = __COLLECTOR_SIZE__

    if (!config['proxy-groups']) {
      config['proxy-groups'] = []
    }
    if (!Array.isArray(config['proxy-groups'])) {
      console.log('[Smart Override] proxy-groups is not an array, converting...')
      config['proxy-groups'] = []
    }

    // 首先检查是否存在 url-test 或 load-balance 代理组
    let hasUrlTestOrLoadBalance = false
    for (let i = 0; i < config['proxy-groups'].length; i++) {
      const group = config['proxy-groups'][i]
      if (group && group.type) {
        const groupType = group.type.toLowerCase()
        if (groupType === 'url-test' || groupType === 'load-balance') {
          hasUrlTestOrLoadBalance = true
          break
        }
      }
    }

    // 存在 url-test / load-balance：只做类型转换
    if (hasUrlTestOrLoadBalance) {
      console.log('[Smart Override] Found url-test or load-balance groups, converting to smart type')
      const nameMapping = new Map()

      for (let i = 0; i < config['proxy-groups'].length; i++) {
        const group = config['proxy-groups'][i]
        if (group && group.type) {
          const groupType = group.type.toLowerCase()
          if (groupType === 'url-test' || groupType === 'load-balance') {
            const originalName = group.name
            group.type = 'smart'
            if (group.name && !group.name.includes('(Smart Group)')) {
              group.name = group.name + '(Smart Group)'
              nameMapping.set(originalName, group.name)
            }
            if (!group['policy-priority']) {
              group['policy-priority'] = ''
            }
            group.uselightgbm = __USE_LIGHTGBM__
            group.collectdata = __COLLECT_DATA__
            group.strategy = '__STRATEGY__'
            if (group.url) delete group.url
            if (group.interval) delete group.interval
            if (group.tolerance) delete group.tolerance
            if (group.lazy) delete group.lazy
            if (group.expected_status) delete group['expected-status']
          }
        }
      }

      if (nameMapping.size > 0) {
        if (config['proxy-groups'] && Array.isArray(config['proxy-groups'])) {
          config['proxy-groups'].forEach(group => {
            if (group && group.proxies && Array.isArray(group.proxies)) {
              group.proxies = group.proxies.map(p => (nameMapping.has(p) ? nameMapping.get(p) : p))
            }
          })
        }

        const ruleParamsSet = new Set(['no-resolve', 'force-remote-dns', 'prefer-ipv6'])
        if (config.rules && Array.isArray(config.rules)) {
          config.rules = config.rules.map(rule => {
            if (typeof rule === 'string') {
              const parts = rule.split(',').map(part => part.trim())
              if (parts.length >= 2) {
                let targetIndex = -1
                if (parts[0] === 'MATCH' && parts.length === 2) {
                  targetIndex = 1
                } else if (parts.length >= 3) {
                  for (let i = 2; i < parts.length; i++) {
                    if (!ruleParamsSet.has(parts[i])) {
                      targetIndex = i
                      break
                    }
                  }
                }
                if (targetIndex !== -1 && nameMapping.has(parts[targetIndex])) {
                  parts[targetIndex] = nameMapping.get(parts[targetIndex])
                  return parts.join(',')
                }
              }
              return rule
            } else if (typeof rule === 'object' && rule !== null) {
              ;['target', 'proxy'].forEach(field => {
                if (rule[field] && nameMapping.has(rule[field])) {
                  rule[field] = nameMapping.get(rule[field])
                }
              })
            }
            return rule
          })
        }

        ;['mode', 'proxy-mode'].forEach(field => {
          if (config[field] && nameMapping.has(config[field])) {
            config[field] = nameMapping.get(config[field])
          }
        })
      }

      console.log('[Smart Override] Conversion completed, skipping other operations')
      return config
    }

    // 否则：查找已有 smart 组并更新参数
    console.log('[Smart Override] No url-test or load-balance groups found, executing original logic')
    let smartGroupExists = false
    for (let i = 0; i < config['proxy-groups'].length; i++) {
      const group = config['proxy-groups'][i]
      if (group && group.type === 'smart') {
        smartGroupExists = true
        if (!group['policy-priority']) {
          group['policy-priority'] = ''
        }
        group.uselightgbm = __USE_LIGHTGBM__
        group.collectdata = __COLLECT_DATA__
        group.strategy = '__STRATEGY__'
        break
      }
    }

    // 没有 smart 组则用全部代理创建一个
    if (!smartGroupExists && config.proxies && Array.isArray(config.proxies) && config.proxies.length > 0) {
      const proxyNames = config.proxies
        .filter(proxy => proxy && typeof proxy === 'object' && proxy.name)
        .map(proxy => proxy.name)
      if (proxyNames.length > 0) {
        config['proxy-groups'].unshift({
          name: 'Smart Group',
          type: 'smart',
          'policy-priority': '',
          uselightgbm: __USE_LIGHTGBM__,
          collectdata: __COLLECT_DATA__,
          strategy: '__STRATEGY__',
          proxies: proxyNames
        })
        console.log('[Smart Override] Created smart group with proxies:', proxyNames.length)
      }
    }

    // 把规则目标替换为 Smart Group
    if (config.rules && Array.isArray(config.rules)) {
      const proxyGroupNames = new Set()
      if (config['proxy-groups'] && Array.isArray(config['proxy-groups'])) {
        config['proxy-groups'].forEach(group => {
          if (group && group.name) proxyGroupNames.add(group.name)
        })
      }
      const builtinTargets = new Set(['DIRECT', 'REJECT', 'REJECT-DROP', 'PASS', 'COMPATIBLE'])
      const ruleParams = new Set(['no-resolve', 'force-remote-dns', 'prefer-ipv6'])

      let replacedCount = 0
      config.rules = config.rules.map(rule => {
        if (typeof rule === 'string') {
          if (rule.includes('((') || rule.includes('))')) return rule
          const parts = rule.split(',').map(part => part.trim())
          if (parts.length >= 2) {
            let targetIndex = -1
            let targetValue = ''
            if (parts[0] === 'MATCH' && parts.length === 2) {
              targetIndex = 1
              targetValue = parts[1]
            } else if (parts.length >= 3) {
              for (let i = 2; i < parts.length; i++) {
                if (!ruleParams.has(parts[i])) {
                  targetIndex = i
                  targetValue = parts[i]
                  break
                }
              }
            }
            if (targetIndex !== -1 && targetValue) {
              const shouldReplace =
                !builtinTargets.has(targetValue) &&
                (proxyGroupNames.has(targetValue) || !ruleParams.has(targetValue))
              if (shouldReplace) {
                parts[targetIndex] = 'Smart Group'
                replacedCount++
                return parts.join(',')
              }
            }
          }
        } else if (typeof rule === 'object' && rule !== null) {
          let targetField = ''
          let targetValue = ''
          if (rule.target) {
            targetField = 'target'
            targetValue = rule.target
          } else if (rule.proxy) {
            targetField = 'proxy'
            targetValue = rule.proxy
          }
          if (targetField && targetValue) {
            const shouldReplace =
              !builtinTargets.has(targetValue) &&
              (proxyGroupNames.has(targetValue) || !ruleParams.has(targetValue))
            if (shouldReplace) {
              rule[targetField] = 'Smart Group'
              replacedCount++
            }
          }
        }
        return rule
      })
      console.log('[Smart Override] Rules processed, replaced', replacedCount, 'targets with Smart Group')
    }

    console.log('[Smart Override] Configuration processed successfully')
    return config
  } catch (error) {
    console.error('[Smart Override] Error processing config:', error)
    return config
  }
}
"#;
    tpl.replace("__COLLECTOR_SIZE__", &collector_size.to_string())
        .replace("__USE_LIGHTGBM__", if use_lightgbm { "true" } else { "false" })
        .replace("__COLLECT_DATA__", if collect_data { "true" } else { "false" })
        .replace("__STRATEGY__", strategy)
}

/// 根据应用配置创建 / 更新 / 删除 Smart 覆写（对齐 manageSmartOverride）
pub fn manage_smart_override(inner: &Arc<MihomoInner>) {
    let app = inner.app_config.lock().unwrap().clone();
    let core = app
        .extra
        .get("core")
        .and_then(|v| v.as_str())
        .unwrap_or("mihomo")
        .to_string();
    let get_bool = |k: &str, d: bool| app.extra.get(k).and_then(|v| v.as_bool()).unwrap_or(d);
    let enable_smart_core = get_bool("enableSmartCore", true);
    let enable_smart_override = get_bool("enableSmartOverride", true);

    let should_enable = enable_smart_core && enable_smart_override && core == "mihomo-smart";

    let mut cfg = inner.override_config.lock().unwrap().clone();
    if should_enable {
        let content = smart_override_template(
            get_bool("smartCoreUseLightGBM", false),
            get_bool("smartCoreCollectData", false),
            app.extra
                .get("smartCoreStrategy")
                .and_then(|v| v.as_str())
                .unwrap_or("sticky-sessions"),
            app.extra
                .get("smartCollectorSize")
                .and_then(|v| v.as_i64())
                .unwrap_or(100),
        );
        let path = override_content_path(&inner.data_dir, SMART_OVERRIDE_ID, "js");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&path, content) {
            eprintln!("[mihomo] 写入 Smart 覆写失败: {e}");
            return;
        }
        match cfg.items.iter_mut().find(|i| i.id == SMART_OVERRIDE_ID) {
            Some(exist) => {
                exist.name = "Smart Core Override".into();
                exist.ext = "js".into();
                exist.global = true;
                exist.type_ = "local".into();
            }
            None => cfg.items.push(OverrideItem {
                id: SMART_OVERRIDE_ID.to_string(),
                name: "Smart Core Override".to_string(),
                ext: "js".to_string(),
                global: true,
                type_: "local".to_string(),
                url: None,
                updated: None,
            }),
        }
    } else {
        if !cfg.items.iter().any(|i| i.id == SMART_OVERRIDE_ID) {
            return;
        }
        cfg.items.retain(|i| i.id != SMART_OVERRIDE_ID);
        let _ = std::fs::remove_file(override_content_path(
            &inner.data_dir,
            SMART_OVERRIDE_ID,
            "js",
        ));
    }
    let _ = save_override_config(&inner.data_dir, &cfg);
    *inner.override_config.lock().unwrap() = cfg;
}
