import React, { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { 
  Rss, 
  Settings2, 
  Clock, 
  ExternalLink, 
  RefreshCw, 
  Plus, 
  Trash2, 
  Calendar,
  AlertTriangle,
  CheckCircle,
  HelpCircle,
  TrendingUp
} from "lucide-react";

interface RssArticle {
  title: string;
  link: string;
  pubDate: Date | null;
  summary: string;
  source: string;
}

interface RssConfig {
  rss_sources: string[];
  is_first_launch: boolean;
}

function stripHtml(html: string): string {
  const tmp = document.createElement("DIV");
  tmp.innerHTML = html;
  const text = tmp.textContent || tmp.innerText || "";
  return text.slice(0, 180).trim() + (text.length > 180 ? "..." : "");
}

function parseRssXml(xmlStr: string, feedUrl: string): RssArticle[] {
  const parser = new DOMParser();
  const xmlDoc = parser.parseFromString(xmlStr, "text/xml");
  
  // Detect parser error
  const parserError = xmlDoc.querySelector("parsererror");
  if (parserError) {
    throw new Error("XML 解析失败，非合法的 RSS 格式");
  }

  // Find feed title
  let feedTitle = "未定义源";
  const titleNode = xmlDoc.querySelector("channel > title, feed > title");
  if (titleNode) {
    feedTitle = titleNode.textContent || "未定义源";
  }

  const articles: RssArticle[] = [];

  // Parse RSS format items
  const items = xmlDoc.querySelectorAll("item");
  if (items.length > 0) {
    items.forEach((item) => {
      const title = item.querySelector("title")?.textContent || "无标题";
      const link = item.querySelector("link")?.textContent || "";
      const description = item.querySelector("description")?.textContent || "";
      
      const pubDateStr = item.querySelector("pubDate, date, pubdate")?.textContent || "";
      let pubDate: Date | null = null;
      if (pubDateStr) {
        const d = new Date(pubDateStr);
        if (!isNaN(d.getTime())) {
          pubDate = d;
        }
      }

      articles.push({
        title: title.trim(),
        link: link.trim(),
        pubDate,
        summary: stripHtml(description),
        source: feedTitle.trim(),
      });
    });
    return articles;
  }

  // Parse Atom format entries
  const entries = xmlDoc.querySelectorAll("entry");
  if (entries.length > 0) {
    entries.forEach((entry) => {
      const title = entry.querySelector("title")?.textContent || "无标题";
      
      let link = "";
      const linkNode = entry.querySelector("link");
      if (linkNode) {
        link = linkNode.getAttribute("href") || linkNode.textContent || "";
      }
      
      const summary = entry.querySelector("summary, content")?.textContent || "";
      
      const pubDateStr = entry.querySelector("published, updated")?.textContent || "";
      let pubDate: Date | null = null;
      if (pubDateStr) {
        const d = new Date(pubDateStr);
        if (!isNaN(d.getTime())) {
          pubDate = d;
        }
      }

      articles.push({
        title: title.trim(),
        link: link.trim(),
        pubDate,
        summary: stripHtml(summary),
        source: feedTitle.trim(),
      });
    });
  }

  return articles;
}

export default function RssReader() {
  const [sources, setSources] = useState<string[]>([]);
  const [articles, setArticles] = useState<RssArticle[]>([]);
  const [loading, setLoading] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  
  // Date filter state
  const [dateFilter, setDateFilter] = useState<"all" | "today" | "3days" | "week" | "custom">("today");
  const [customStartDate, setCustomStartDate] = useState<string>("");
  const [customEndDate, setCustomEndDate] = useState<string>("");

  // Configuration modal state
  const [showConfig, setShowConfig] = useState<boolean>(false);
  const [editSources, setEditSources] = useState<string[]>([]);
  const [newSourceUrl, setNewSourceUrl] = useState<string>("");
  const [testStatus, setTestStatus] = useState<Record<string, "testing" | "success" | "error">>({});
  const [configMessage, setConfigMessage] = useState<string | null>(null);

  // Deleted articles state (stores article links as unique identifiers)
  const [deletedLinks, setDeletedLinks] = useState<Set<string>>(new Set());

  // Load deleted links from localStorage on mount
  useEffect(() => {
    const stored = localStorage.getItem("rss_deleted_links");
    if (stored) {
      try {
        const parsed = JSON.parse(stored);
        if (Array.isArray(parsed)) {
          setDeletedLinks(new Set(parsed));
        }
      } catch (e) {
        console.error("解析已读 RSS 链接失败:", e);
      }
    }
  }, []);

  // Handle deleting an article
  const handleDeleteArticle = (id: string, e: React.MouseEvent) => {
    e.stopPropagation(); // Stop click from propagating to the article card (which opens the article)
    const newDeleted = new Set(deletedLinks);
    newDeleted.add(id);
    setDeletedLinks(newDeleted);
    localStorage.setItem("rss_deleted_links", JSON.stringify(Array.from(newDeleted)));
  };

  // Clear all deleted links (restore deleted articles)
  const handleRestoreArticles = () => {
    setDeletedLinks(new Set());
    localStorage.removeItem("rss_deleted_links");
  };

  // Load config & sources
  const loadRssConfig = async () => {
    try {
      const res = await invoke<RssConfig>("get_rss_config");
      setSources(res.rss_sources);
      setEditSources(res.rss_sources);
      return res.rss_sources;
    } catch (err: any) {
      setError(`加载 RSS 配置失败: ${err.message || err}`);
      return [];
    }
  };

  // Fetch articles from all sources
  const fetchAllFeeds = async (feedUrls: string[], force = false) => {
    if (feedUrls.length === 0) {
      setArticles([]);
      setError("未配置 RSS 订阅源，点击右上角设置配置订阅源。");
      return;
    }

    setLoading(true);
    setError(null);
    const allArticles: RssArticle[] = [];
    const errors: string[] = [];
    const now = Date.now();
    const cacheDuration = 24 * 60 * 60 * 1000; // 24 hours in milliseconds

    await Promise.all(
      feedUrls.map(async (url) => {
        try {
          let xmlText = "";
          let useCache = false;

          if (!force) {
            const cachedData = localStorage.getItem(`rss_cache_${url}`);
            if (cachedData) {
              try {
                const { xml, fetchedAt } = JSON.parse(cachedData);
                if (xml && fetchedAt && (now - fetchedAt < cacheDuration)) {
                  xmlText = xml;
                  useCache = true;
                }
              } catch (e) {
                console.error("解析 RSS 缓存失败:", e);
              }
            }
          }

          if (!useCache) {
            xmlText = await invoke<string>("fetch_rss_feed", { url });
            localStorage.setItem(`rss_cache_${url}`, JSON.stringify({
              xml: xmlText,
              fetchedAt: now
            }));
          }

          const parsed = parseRssXml(xmlText, url);
          allArticles.push(...parsed);
        } catch (err: any) {
          console.error(`Fetch feed error for ${url}:`, err);
          errors.push(`${url}: ${err.message || err}`);
        }
      })
    );

    // Sort articles by published date descending
    allArticles.sort((a, b) => {
      if (!a.pubDate) return 1;
      if (!b.pubDate) return -1;
      return b.pubDate.getTime() - a.pubDate.getTime();
    });

    setArticles(allArticles);
    setLoading(false);

    if (errors.length > 0 && allArticles.length === 0) {
      setError(`所有订阅源加载失败:\n${errors.join("\n")}`);
    } else if (errors.length > 0) {
      setError(`部分订阅源加载失败，已加载部分资讯。`);
    }
  };

  useEffect(() => {
    loadRssConfig().then((urls) => {
      if (urls.length > 0) {
        fetchAllFeeds(urls, false);
      }
    });
  }, []);

  // Filtered articles
  const filteredArticles = useMemo(() => {
    const now = new Date();
    return articles.filter((article) => {
      // 过滤掉已删除（已读）的资讯
      const articleId = article.link || article.title;
      if (deletedLinks.has(articleId)) return false;

      if (dateFilter === "all") return true;
      if (!article.pubDate) return false;

      const diffTime = now.getTime() - article.pubDate.getTime();
      const diffDays = diffTime / (1000 * 60 * 60 * 24);

      if (dateFilter === "today") {
        return diffDays <= 1;
      }
      if (dateFilter === "3days") {
        return diffDays <= 3;
      }
      if (dateFilter === "week") {
        return diffDays <= 7;
      }
      if (dateFilter === "custom") {
        const itemTime = article.pubDate.getTime();
        const start = customStartDate ? new Date(customStartDate + "T00:00:00").getTime() : 0;
        const end = customEndDate ? new Date(customEndDate + "T23:59:59").getTime() : Infinity;
        return itemTime >= start && itemTime <= end;
      }
      return true;
    });
  }, [articles, dateFilter, customStartDate, customEndDate, deletedLinks]);

  // Article open helper
  const handleOpenArticle = async (url: string) => {
    if (!url) return;
    try {
      await openUrl(url);
    } catch (err) {
      window.open(url, "_blank");
    }
  };

  // Test RSS link
  const testRssUrl = async (url: string) => {
    const trimmed = url.trim();
    if (!trimmed) return;

    setTestStatus(prev => ({ ...prev, [trimmed]: "testing" }));
    try {
      const xmlText = await invoke<string>("fetch_rss_feed", { url: trimmed });
      parseRssXml(xmlText, trimmed); // will throw error if invalid
      setTestStatus(prev => ({ ...prev, [trimmed]: "success" }));
    } catch (err) {
      console.error(err);
      setTestStatus(prev => ({ ...prev, [trimmed]: "error" }));
    }
  };

  // Add source to edit list
  const handleAddSource = () => {
    const trimmed = newSourceUrl.trim();
    if (!trimmed) return;
    if (editSources.includes(trimmed)) {
      setConfigMessage("该订阅源已在列表中");
      return;
    }
    setEditSources([...editSources, trimmed]);
    setNewSourceUrl("");
    setConfigMessage(null);
  };

  // Remove source from edit list
  const handleRemoveSource = (url: string) => {
    setEditSources(editSources.filter((s) => s !== url));
    const newStatus = { ...testStatus };
    delete newStatus[url];
    setTestStatus(newStatus);
  };

  // Save config
  const handleSaveConfig = async () => {
    try {
      await invoke("set_rss_sources", { sources: editSources });
      setSources(editSources);
      setShowConfig(false);
      setConfigMessage(null);
      fetchAllFeeds(editSources);
    } catch (err: any) {
      setConfigMessage(`保存失败: ${err.message || err}`);
    }
  };

  // Format date helper
  const formatDate = (date: Date | null) => {
    if (!date) return "未知时间";
    const y = date.getFullYear();
    const m = String(date.getMonth() + 1).padStart(2, "0");
    const d = String(date.getDate()).padStart(2, "0");
    const hh = String(date.getHours()).padStart(2, "0");
    const mm = String(date.getMinutes()).padStart(2, "0");
    return `${y}-${m}-${d} ${hh}:${mm}`;
  };

  return (
    <div className="flex-grow flex flex-col min-h-0 bg-slate-950/20 text-slate-100 rounded-xl overflow-hidden border border-white/5">
      {/* 顶部工具条 */}
      <div className="p-4 border-b border-white/5 bg-slate-900/40 backdrop-blur-md flex flex-wrap items-center justify-between gap-4">
        <div className="flex items-center gap-2">
          <div className="p-1.5 rounded-lg bg-orange-500/10 border border-orange-500/20 text-orange-400">
            <Rss className="w-4 h-4" />
          </div>
          <div>
            <h2 className="text-sm font-bold text-slate-200">订阅资讯中心</h2>
            <p className="text-[10px] text-slate-400 mt-0.5">获取您配置的 RSS/Atom 资讯列表，保持与最新开发动态同步。</p>
          </div>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={() => fetchAllFeeds(sources, true)}
            disabled={loading}
            className="px-2.5 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] font-semibold text-slate-300 hover:text-white hover:bg-white/10 transition-all flex items-center gap-1 cursor-pointer disabled:opacity-50"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`} />
            刷新
          </button>
          
          <button
            onClick={() => {
              setEditSources(sources);
              setShowConfig(true);
            }}
            className="px-3 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-500 text-white text-[10px] font-semibold flex items-center gap-1 transition-all shadow-lg shadow-blue-500/10 cursor-pointer"
          >
            <Settings2 className="w-3.5 h-3.5" />
            配置源
          </button>
        </div>
      </div>

      {/* 筛选过滤条 */}
      <div className="px-4 py-2 bg-slate-900/10 border-b border-white/5 flex flex-wrap items-center justify-between gap-2.5">
        <div className="flex items-center gap-1.5 flex-wrap">
          <span className="text-[10px] text-slate-500 font-semibold flex items-center gap-1 mr-1">
            <Clock className="w-3 h-3" /> 过滤时间：
          </span>
          {[
            { key: "all", label: "全部" },
            { key: "today", label: "今天" },
            { key: "3days", label: "三天内" },
            { key: "week", label: "本周" },
            { key: "custom", label: "自定义" }
          ].map((item) => (
            <button
              key={item.key}
              onClick={() => setDateFilter(item.key as any)}
              className={`px-2.5 py-1 rounded-md text-[10px] font-medium transition-all cursor-pointer ${
                dateFilter === item.key
                  ? "bg-slate-800 text-blue-400 border border-blue-500/20"
                  : "bg-white/5 text-slate-400 border border-transparent hover:text-slate-200"
              }`}
            >
              {item.label}
            </button>
          ))}
        </div>

        <div className="flex items-center gap-2 flex-wrap">
          {dateFilter === "custom" && (
            <div className="flex items-center gap-1 text-[10px]">
              <input
                type="date"
                value={customStartDate}
                onChange={(e) => setCustomStartDate(e.target.value)}
                className="bg-slate-900 border border-white/10 rounded-md px-2 py-0.5 text-slate-300 focus:outline-none"
              />
              <span className="text-slate-600">至</span>
              <input
                type="date"
                value={customEndDate}
                onChange={(e) => setCustomEndDate(e.target.value)}
                className="bg-slate-900 border border-white/10 rounded-md px-2 py-0.5 text-slate-300 focus:outline-none"
              />
            </div>
          )}

          {deletedLinks.size > 0 && (
            <button
              onClick={handleRestoreArticles}
              className="px-2 py-0.5 rounded-md text-[10px] font-medium bg-blue-500/10 text-blue-400 hover:text-blue-300 hover:bg-blue-500/20 border border-blue-500/20 transition-all cursor-pointer flex items-center gap-1"
              title="恢复所有已删除（已读过）的资讯"
            >
              <RefreshCw className="w-3 h-3" />
              恢复已读 ({deletedLinks.size})
            </button>
          )}
        </div>
      </div>

      {/* 错误提示 */}
      {error && (
        <div className="mx-4 mt-4 p-3 bg-red-500/10 border border-red-500/20 text-[10px] text-red-400 rounded-xl flex items-start gap-2">
          <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-0.5" />
          <div className="whitespace-pre-line">{error}</div>
        </div>
      )}

      {/* 内容主体 */}
      <div className="flex-grow min-h-0 overflow-y-auto p-4 space-y-3">
        {loading ? (
          <div className="h-48 flex flex-col items-center justify-center text-slate-500">
            <RefreshCw className="w-8 h-8 animate-spin mb-2 text-blue-500" />
            <span className="text-[11px]">正在解析并抓取资讯列表中，请稍候...</span>
          </div>
        ) : filteredArticles.length === 0 ? (
          <div className="h-64 border border-dashed border-white/5 rounded-2xl flex flex-col items-center justify-center text-slate-500 p-8 text-center bg-white/[0.01]">
            <Rss className="w-10 h-10 text-slate-700 mb-2 animate-pulse" />
            <span className="text-xs font-bold text-slate-400">暂无资讯数据</span>
            <span className="text-[10px] text-slate-600 mt-1 max-w-[280px]">
              没有匹配到当前的日期过滤条件，或是当前订阅源中没有文章。您也可以点击右上角“配置源”添加其他 RSS 源。
            </span>
          </div>
        ) : (
          filteredArticles.map((article, idx) => {
            const articleId = article.link || article.title;
            return (
              <div
                key={`${articleId}-${idx}`}
                onClick={() => handleOpenArticle(article.link)}
                className="p-3.5 rounded-xl border border-white/5 bg-slate-900/30 hover:border-blue-500/30 hover:bg-slate-900/50 transition-all duration-200 cursor-pointer group flex flex-col gap-2 relative overflow-hidden"
              >
                {/* 光晕装饰效果 */}
                <div className="absolute right-0 top-0 w-24 h-24 bg-blue-500/5 blur-2xl rounded-full group-hover:bg-blue-500/10 transition-all pointer-events-none" />

                <div className="flex items-center justify-between gap-4">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="px-1.5 py-0.5 bg-orange-500/10 border border-orange-500/20 text-orange-400 text-[8px] font-bold rounded-md flex items-center gap-0.5">
                      <TrendingUp className="w-2.5 h-2.5" />
                      {article.source}
                    </span>
                    <span className="text-[9.5px] text-slate-500 flex items-center gap-1">
                      <Calendar className="w-3 h-3" />
                      {formatDate(article.pubDate)}
                    </span>
                  </div>
                  
                  <div className="flex items-center gap-2 flex-shrink-0">
                    <button
                      onClick={(e) => handleDeleteArticle(articleId, e)}
                      className="p-1 rounded-md text-slate-500 hover:text-red-400 hover:bg-red-500/10 opacity-0 group-hover:opacity-100 transition-all duration-200 cursor-pointer"
                      title="删除此条资讯 (标记为已读)"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                    <ExternalLink className="w-3.5 h-3.5 text-slate-500 group-hover:text-blue-400 transition-all" />
                  </div>
                </div>

                <h3 className="text-xs font-bold text-slate-200 group-hover:text-white transition-all leading-relaxed">
                  {article.title}
                </h3>

                {article.summary && (
                  <p className="text-[10px] text-slate-400 leading-relaxed font-sans line-clamp-2">
                    {article.summary}
                  </p>
                )}
              </div>
            );
          })
        )}
      </div>

      {/* 配置模态窗 */}
      {showConfig && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 flex items-center justify-center p-4">
          <div className="w-full max-w-lg bg-slate-950/95 border border-white/10 rounded-2xl shadow-2xl flex flex-col max-h-[85vh] overflow-hidden">
            {/* Header */}
            <div className="p-4 border-b border-white/5 flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Settings2 className="w-4.5 h-4.5 text-blue-400" />
                <h3 className="text-xs font-bold text-slate-200">RSS 订阅源管理</h3>
              </div>
              <button 
                onClick={() => {
                  setShowConfig(false);
                  setConfigMessage(null);
                }}
                className="text-slate-500 hover:text-slate-300 text-xs"
              >
                关闭
              </button>
            </div>

            {/* Input to add */}
            <div className="p-4 border-b border-white/5 bg-white/[0.01]">
              <div className="flex gap-2">
                <input
                  type="text"
                  placeholder="添加新的 RSS 订阅源 URL（例如：https://36kr.com/feed）"
                  value={newSourceUrl}
                  onChange={(e) => setNewSourceUrl(e.target.value)}
                  className="flex-1 bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-[10.5px] text-slate-200 placeholder-slate-500 focus:outline-none focus:border-blue-500"
                />
                <button
                  onClick={() => testRssUrl(newSourceUrl)}
                  disabled={!newSourceUrl.trim()}
                  className="px-2.5 py-1 rounded-lg bg-white/5 hover:bg-white/10 border border-white/10 text-[10px] text-slate-300 disabled:opacity-40 cursor-pointer flex-shrink-0"
                >
                  测试链接
                </button>
                <button
                  onClick={handleAddSource}
                  disabled={!newSourceUrl.trim()}
                  className="px-3 py-1 rounded-lg bg-blue-600 hover:bg-blue-500 text-white text-[10px] font-semibold disabled:opacity-40 cursor-pointer flex-shrink-0 flex items-center gap-0.5"
                >
                  <Plus className="w-3 h-3" /> 添加
                </button>
              </div>
              {configMessage && (
                <div className="text-[9.5px] text-red-400 mt-2 flex items-center gap-1 font-medium">
                  <AlertTriangle className="w-3.5 h-3.5" />
                  {configMessage}
                </div>
              )}
            </div>

            {/* List */}
            <div className="flex-grow overflow-y-auto p-4 space-y-2">
              <div className="text-[10px] font-bold text-slate-500 mb-1">当前的订阅列表 ({editSources.length})</div>
              {editSources.length === 0 ? (
                <div className="py-8 text-center text-[10.5px] text-slate-600">无订阅源，请在上方添加新的订阅地址。</div>
              ) : (
                editSources.map((url) => {
                  const status = testStatus[url];
                  return (
                    <div 
                      key={url}
                      className="p-2.5 rounded-xl bg-white/5 border border-white/5 flex items-center justify-between gap-3 text-[10px]"
                    >
                      <span className="font-mono text-slate-300 break-all truncate max-w-[320px] select-all" title={url}>{url}</span>
                      
                      <div className="flex items-center gap-2 flex-shrink-0">
                        {status === "testing" && (
                          <span className="text-yellow-500 font-semibold flex items-center gap-0.5">
                            <RefreshCw className="w-3 h-3 animate-spin" />
                            测试中
                          </span>
                        )}
                        {status === "success" && (
                          <span className="text-green-400 font-semibold flex items-center gap-0.5">
                            <CheckCircle className="w-3 h-3" />
                            有效
                          </span>
                        )}
                        {status === "error" && (
                          <span className="text-red-400 font-semibold flex items-center gap-0.5">
                            <AlertTriangle className="w-3 h-3" />
                            无效
                          </span>
                        )}
                        {!status && (
                          <button
                            onClick={() => testRssUrl(url)}
                            className="text-slate-400 hover:text-slate-200 underline cursor-pointer"
                          >
                            测试
                          </button>
                        )}
                        
                        <button
                          onClick={() => handleRemoveSource(url)}
                          className="p-1 hover:bg-red-500/10 rounded text-slate-400 hover:text-red-400 transition-all cursor-pointer"
                          title="删除订阅"
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </div>
                    </div>
                  );
                })
              )}
            </div>

            {/* Footer */}
            <div className="p-4 border-t border-white/5 bg-slate-900/20 flex justify-end gap-2">
              <button
                onClick={() => {
                  setShowConfig(false);
                  setConfigMessage(null);
                }}
                className="px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-slate-400 hover:text-slate-200 text-[10px] font-semibold cursor-pointer"
              >
                取消
              </button>
              <button
                onClick={handleSaveConfig}
                className="px-3.5 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-500 text-white text-[10px] font-semibold cursor-pointer"
              >
                保存修改
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
