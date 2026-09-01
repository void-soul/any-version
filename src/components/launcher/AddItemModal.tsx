import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  X,
  FileText,
  Folder,
  Globe,
  Settings as SettingsIcon,
  Layers,
  Shield,
  Search,
  Upload,
  Sparkles,
  RefreshCw,
  Plus,
  Trash2,
  Check,
  Package,
  AppWindow,
  Image as ImageIcon,
  RotateCcw,
  Link2,
  Download,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Item, ItemData, Classification, MultiItemEntry, ScannedProgram, AppxItem, UrlMetadata, ShortcutInfo } from "./types";
import CategoryTreeSelect from "./CategoryTreeSelect";

interface AddItemModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSave: (item: Item) => Promise<void>;
  editingItem: Item | null;
  classificationId: number;
  classifications: Classification[];
}

// 预设 Windows 核心系统工具
const PRESET_SYSTEM_TOOLS = [
  { name: "任务管理器", target: "taskmgr.exe", params: "", icon: "⚡", desc: "查看进程、性能与服务" },
  { name: "控制面板", target: "control.exe", params: "", icon: "🎛️", desc: "Windows 传统控制面板" },
  { name: "环境变量与系统属性", target: "sysdm.cpl", params: "", icon: "🌿", desc: "配置用户与系统 PATH 变量" },
  { name: "设备管理器", target: "devmgmt.msc", params: "", icon: "🖥️", desc: "硬件与驱动设备管理" },
  { name: "计算机管理", target: "compmgmt.msc", params: "", icon: "🏢", desc: "磁盘管理、事件查看器与计划任务" },
  { name: "服务管理", target: "services.msc", params: "", icon: "🛠️", desc: "查看与配置 Windows 后台服务" },
  { name: "注册表编辑器", target: "regedit.exe", params: "", icon: "📝", desc: "查看与修改 Windows 注册表" },
  { name: "组策略编辑器", target: "gpedit.msc", params: "", icon: "📜", desc: "配置本地组策略 (专业版及以上)" },
  { name: "网络连接", target: "ncpa.cpl", params: "", icon: "🌐", desc: "查看网络适配器与 IP 配置" },
  { name: "资源监视器", target: "perfmon.exe", params: "/res", icon: "📊", desc: "细粒度监控 CPU/内存/磁盘/网络" },
  { name: "磁盘清理", target: "cleanmgr.exe", params: "", icon: "🧹", desc: "清理系统垃圾与临时文件" },
  { name: "系统配置 (msconfig)", target: "msconfig.exe", params: "", icon: "🔧", desc: "系统引导与启动项诊断" },
  { name: "DirectX 诊断工具", target: "dxdiag.exe", params: "", icon: "🎮", desc: "显卡与声卡 DirectX 诊断" },
  { name: "命令提示符 (CMD)", target: "cmd.exe", params: "", icon: "💻", desc: "Windows 命令解释器" },
  { name: "Windows PowerShell", target: "powershell.exe", params: "", icon: "🔷", desc: "强大自动化脚本命令行" },
  { name: "Windows 终端 (WT)", target: "wt.exe", params: "", icon: "⬛", desc: "现代多标签终端窗口" },
  { name: "计算器", target: "calc.exe", params: "", icon: "🔢", desc: "Windows 计算器" },
  { name: "记事本", target: "notepad.exe", params: "", icon: "📄", desc: "轻量文本编辑器" },
  { name: "截图工具", target: "SnippingTool.exe", params: "", icon: "✂️", desc: "屏幕截图与贴图工具" },
  { name: "画图", target: "mspaint.exe", params: "", icon: "🎨", desc: "Windows 画图" },
  { name: "锁定计算机", target: "static:LockWorkstation", params: "", icon: "🔒", desc: "立即锁定屏幕" },
  { name: "关闭显示器", target: "static:TurnOffMonitor", params: "", icon: "🌙", desc: "节能关闭显示屏" },
  { name: "清空回收站", target: "static:EmptyRecycleBin", params: "", icon: "🗑️", desc: "一键静默清理回收站" },
  { name: "重启资源管理器", target: "static:RestartExplorer", params: "", icon: "🔄", desc: "重启 Windows 桌面外壳" },
  { name: "上帝模式 (God Mode)", target: "shell:::{ED7BA470-8E54-465E-825C-99712043E01C}", params: "", icon: "👑", desc: "聚合全部系统设置项" },
];

export default function AddItemModal({
  isOpen,
  onClose,
  onSave,
  editingItem,
  classificationId,
  classifications,
}: AddItemModalProps) {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<number>(
    editingItem ? (editingItem.itemType === 5 ? 6 : editingItem.itemType) : 0
  );
  const [targetClassificationId, setTargetClassificationId] = useState<number>(
    editingItem ? editingItem.classificationId : classificationId
  );
  const [name, setName] = useState(editingItem ? editingItem.name : "");
  const [target, setTarget] = useState(editingItem?.data?.target || "");
  const [params, setParams] = useState(editingItem?.data?.params || "");
  const [startLocation, setStartLocation] = useState(editingItem?.data?.startLocation || "");
  const [runAsAdmin, setRunAsAdmin] = useState(!!editingItem?.data?.runAsAdmin);
  const [icon, setIcon] = useState<string | null>(editingItem?.data?.icon || null);
  const [htmlIcon, setHtmlIcon] = useState<string | null>(editingItem?.data?.htmlIcon || null);
  const [iconBg, setIconBg] = useState<boolean>(!!editingItem?.data?.iconBackgroundColor);
  // 图标背景色块色值（hex），默认 Windows 蓝
  const [iconBgValue, setIconBgValue] = useState<string>(
    editingItem?.data?.iconBackgroundColorValue || "#0078D7"
  );
  // 网络图标下载
  const [netIconOpen, setNetIconOpen] = useState(false);
  const [netIconUrl, setNetIconUrl] = useState("");
  const [netIconLoading, setNetIconLoading] = useState(false);
  const [netIconError, setNetIconError] = useState<string | null>(null);
  const [remark, setRemark] = useState(editingItem?.data?.remark || "");
  const [saving, setSaving] = useState(false);

  // 网页网址抓取
  const [urlFetching, setUrlFetching] = useState(false);

  // 开始菜单扫描
  const [startMenuScanning, setStartMenuScanning] = useState(false);
  const [startMenuList, setStartMenuList] = useState<ScannedProgram[]>([]);
  const [startMenuSearch, setStartMenuSearch] = useState("");

  // Appx 扫描
  const [appxScanning, setAppxScanning] = useState(false);
  const [appxList, setAppxList] = useState<AppxItem[]>([]);
  const [appxSearch, setAppxSearch] = useState("");

  // 多项目
  const [multiItems, setMultiItems] = useState<MultiItemEntry[]>(editingItem?.data?.multiItems || []);

  // State is already cleanly initialized from props in useState() upon modal mount (via key)

  // 按 Escape 关闭弹窗（暂时注释以测试）
  /*
  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isOpen, onClose]);
  */

  if (!isOpen) return null;   // 选择文件 / 快捷方式
  const handleSelectFile = async () => {
    try {
      // 用自定义对话框（.NET OpenFileDialog，不跟随 .lnk），能拿到快捷方式原始路径，
      // 便于 resolve_shortcut 正确解析目标/参数/工作目录/图标（openDialog 会跟随 .lnk 返回其目标）。
      const selected = await invoke<string | null>("launcher_pick_file");
      if (selected) {
        setTarget(selected);
        // 解析快捷方式或提取图标
        try {
          const info = await invoke<ShortcutInfo | null>("launcher_resolve_shortcut", { path: selected });
          if (info) {
            if (!name) setName(info.name);
            if (info.targetPath && info.targetPath !== selected) {
              setTarget(info.targetPath);
            }
            if (info.arguments) setParams(info.arguments);
            if (info.workingDir) setStartLocation(info.workingDir);
            if (info.iconBase64) setIcon(info.iconBase64);
          }
        } catch {
          // fallback icon
          const extIcon = await invoke<string | null>("launcher_extract_icon", { path: selected });
          if (extIcon) setIcon(extIcon);
        }
      }
    } catch (e) {
      console.error(e);
    }
  };

  // 选择文件夹
  const handleSelectFolder = async () => {
    try {
      const selected = await openDialog({
        directory: true,
        multiple: false,
        title: t("additem.pickFolderTitle"),
      });
      if (selected && typeof selected === "string") {
        setTarget(selected);
        if (!name) {
          const parts = selected.split(/[\\/]/);
          setName(parts[parts.length - 1] || t("additem.folderDefault"));
        }
        const extIcon = await invoke<string | null>("launcher_extract_icon", { path: selected });
        if (extIcon) setIcon(extIcon);
      }
    } catch (e) {
      console.error(e);
    }
  };

  // 一键抓取网址信息与高清 Favicon
  const handleFetchUrl = async () => {
    if (!target.trim()) return;
    let urlStr = target.trim();
    if (!urlStr.startsWith("http://") && !urlStr.startsWith("https://")) {
      urlStr = `https://${urlStr}`;
      setTarget(urlStr);
    }
    setUrlFetching(true);
    try {
      const meta = await invoke<UrlMetadata>("launcher_fetch_url_info", { url: urlStr });
      if (meta.title && !name) setName(meta.title);
      if (meta.icon) setIcon(meta.icon);
    } catch (e) {
      console.error("抓取网页信息失败:", e);
    } finally {
      setUrlFetching(false);
    }
  };

  // 扫描开始菜单
  const handleScanStartMenu = async () => {
    setStartMenuScanning(true);
    try {
      const list = await invoke<ScannedProgram[]>("launcher_scan_start_menu");
      setStartMenuList(list);
    } catch (e) {
      console.error(e);
    } finally {
      setStartMenuScanning(false);
    }
  };

  // 扫描 Appx 应用
  const handleScanAppx = async () => {
    setAppxScanning(true);
    try {
      const list = await invoke<AppxItem[]>("launcher_scan_appx");
      setAppxList(list);
    } catch (e) {
      console.error(e);
    } finally {
      setAppxScanning(false);
    }
  };

  // ---- 图标编辑（参考 DawnLauncher）----

  // 从本地选择图片文件作为项目图标（转为 base64 data URL）
  const handleUploadIcon = async () => {
    try {
      const selected = await openDialog({
        directory: false,
        multiple: false,
        title: t("additem.pickIconTitle"),
        filters: [
          {
            name: t("additem.imageFilter"),
            extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg"],
          },
        ],
      });
      if (selected && typeof selected === "string") {
        const dataUrl = await invoke<string>("launcher_load_image_as_icon", { path: selected });
        setIcon(dataUrl);
        setHtmlIcon(null);
      }
    } catch (e) {
      console.error("上传图标失败:", e);
    }
  };

  // 从目标程序/文件重新提取默认图标
  const handleRestoreDefaultIcon = async () => {
    if (!target.trim()) return;
    try {
      const extIcon = await invoke<string | null>("launcher_extract_icon", { path: target.trim() });
      if (extIcon) {
        setIcon(extIcon);
        setHtmlIcon(null);
      }
    } catch (e) {
      console.error("恢复默认图标失败:", e);
    }
  };

  // 清除当前图标（恢复为按项目类型显示的默认图标）
  const handleClearIcon = () => {
    setIcon(null);
    setHtmlIcon(null);
  };

  // 下载远程图片作为图标（参考 DawnLauncher 网络图标）
  const handleDownloadNetIcon = async () => {
    const url = netIconUrl.trim();
    if (!url) return;
    setNetIconLoading(true);
    setNetIconError(null);
    try {
      const dataUrl = await invoke<string>("launcher_download_image", { url });
      setIcon(dataUrl);
      setHtmlIcon(null);
      setNetIconOpen(false);
      setNetIconUrl("");
    } catch (e: any) {
      console.error("下载网络图标失败:", e);
      // 把具体错误透传给用户，便于定位（保持弹窗打开方便重试）
      setNetIconError(typeof e === "string" ? e : e?.message || t("additem.downloadFailedDefault"));
    } finally {
      setNetIconLoading(false);
    }
  };

  // 提交保存
  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;

    setSaving(true);
    try {
      let finalItemType = activeTab;
      if (activeTab === 5) finalItemType = 0; // 开始菜单作为普通文件/程序
      if (activeTab === 6) finalItemType = 5; // 多项目

      const data: ItemData = {
        target: target.trim() || undefined,
        params: params.trim() || undefined,
        startLocation: startLocation.trim() || undefined,
        runAsAdmin,
        icon: icon || undefined,
        htmlIcon: htmlIcon || undefined,
        iconBackgroundColor: iconBg,
        iconBackgroundColorValue: iconBg ? iconBgValue : undefined,
        remark: remark.trim() || undefined,
        multiItems: finalItemType === 5 ? multiItems : undefined,
      };

      const itemToSave: Item = {
        id: editingItem ? editingItem.id : 0,
        classificationId: targetClassificationId,
        name: name.trim(),
        itemType: finalItemType,
        data,
        shortcutKey: editingItem?.shortcutKey || null,
        globalShortcutKey: !!editingItem?.globalShortcutKey,
        order: editingItem?.order || 0,
      };

      await onSave(itemToSave);
      onClose();
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-[100] modal-mask flex items-center justify-center bg-black/60 backdrop-blur-sm p-4"
    >
      <div
        className="bg-[#141927] border border-white/10 rounded-2xl w-full max-w-2xl shadow-2xl overflow-hidden flex flex-col max-h-[92vh] animate-in fade-in zoom-in-95 duration-150 text-slate-100"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-white/10 bg-white/[0.02]">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-xl bg-purple-600/20 border border-purple-500/30 flex items-center justify-center text-purple-400">
              {icon ? (
                <img src={icon} className="w-5 h-5 object-contain" alt="" />
              ) : htmlIcon ? (
                <span className="text-base">{htmlIcon}</span>
              ) : (
                <Sparkles className="w-4 h-4" />
              )}
            </div>
            <div>
              <h3 className="text-sm font-semibold text-white">
                {editingItem ? t("additem.editTitle") : t("additem.addTitle")}
              </h3>
              <p className="text-[11px] text-slate-400">
                {t("additem.subtitle")}
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-1.5 rounded-lg text-slate-400 hover:text-white hover:bg-white/5 transition cursor-pointer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Tab navigation */}
        <div className="flex items-center gap-1 px-5 py-2.5 border-b border-white/5 bg-white/[0.01] overflow-x-auto">
          {[
            { id: 0, label: t("additem.tabFile"), icon: <FileText className="w-3.5 h-3.5" /> },
            { id: 1, label: t("additem.tabFolder"), icon: <Folder className="w-3.5 h-3.5" /> },
            { id: 2, label: t("additem.tabUrl"), icon: <Globe className="w-3.5 h-3.5" /> },
            { id: 3, label: t("additem.tabSystem"), icon: <SettingsIcon className="w-3.5 h-3.5" /> },
            { id: 4, label: t("additem.tabAppx"), icon: <Package className="w-3.5 h-3.5" /> },
            { id: 5, label: t("additem.tabStartMenu"), icon: <AppWindow className="w-3.5 h-3.5" /> },
            { id: 6, label: t("additem.tabMulti"), icon: <Layers className="w-3.5 h-3.5" /> },
          ].map((t) => (
            <button
              type="button"
              key={t.id}
              onClick={() => {
                setActiveTab(t.id);
                if (t.id === 5 && startMenuList.length === 0) handleScanStartMenu();
                if (t.id === 4 && appxList.length === 0) handleScanAppx();
              }}
              className={`px-3 py-1.5 rounded-xl text-xs font-medium flex items-center gap-1.5 transition cursor-pointer whitespace-nowrap ${
                activeTab === t.id
                  ? "bg-purple-600 text-white shadow-md shadow-purple-600/20"
                  : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
              }`}
            >
              {t.icon}
              {t.label}
            </button>
          ))}
        </div>

        {/* Content Form */}
        <form onSubmit={handleSubmit} className="p-5 overflow-y-auto space-y-4 flex-1">
          {/* Classification Selection */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <div>
              <label className="block text-xs font-medium text-slate-400 mb-1.5">{t("additem.classification")}</label>
              <CategoryTreeSelect
                classifications={classifications}
                value={targetClassificationId}
                onChange={setTargetClassificationId}
                placeholder={t("additem.selectCategory")}
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-slate-400 mb-1.5">{t("additem.itemName")}</label>
              <input
                autoFocus
                type="text"
                required
                autoComplete="off"
                spellCheck={false}
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={t("additem.nameExample")}
                className="w-full bg-white/5 border border-white/10 rounded-xl px-3 py-2 text-xs text-white focus:outline-none focus:border-purple-500 select-text transition"
              />
            </div>
          </div>

          {/* 图标编辑（参考 DawnLauncher） */}
          <div className="pt-1">
            <label className="block text-xs font-medium text-slate-400 mb-2">{t("additem.itemIcon")}</label>
            <div className="flex items-center gap-3">
              {/* 图标预览 */}
              <div
                className="w-12 h-12 rounded-xl flex items-center justify-center overflow-hidden flex-shrink-0 border border-white/10"
                style={{
                  backgroundColor: iconBg ? iconBgValue : "rgba(255,255,255,0.05)",
                }}
              >
                {icon ? (
                  <img src={icon} className="w-10 h-10 object-contain" alt="" />
                ) : htmlIcon ? (
                  <span className="text-2xl leading-none">{htmlIcon}</span>
                ) : (
                  <FileText className="w-6 h-6 text-slate-500" />
                )}
              </div>
              {/* 图标操作按钮 */}
              <div className="flex flex-wrap gap-1.5 flex-1">
                <button
                  type="button"
                  onClick={handleUploadIcon}
                  title={t("additem.uploadImageTitle")}
                  className="px-2.5 py-1.5 bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 hover:text-white text-xs rounded-lg transition cursor-pointer flex items-center gap-1.5"
                >
                  <ImageIcon className="w-3.5 h-3.5 text-purple-400" />
                  {t("additem.uploadImage")}
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setNetIconOpen((v) => !v);
                    if (!netIconUrl) setNetIconUrl(target.trim().startsWith("http") ? target.trim() : "");
                  }}
                  title={t("additem.netIconTitle")}
                  className="px-2.5 py-1.5 bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 hover:text-white text-xs rounded-lg transition cursor-pointer flex items-center gap-1.5"
                >
                  <Link2 className="w-3.5 h-3.5 text-blue-400" />
                  {t("additem.netIcon")}
                </button>
                <button
                  type="button"
                  onClick={handleRestoreDefaultIcon}
                  disabled={!target.trim()}
                  title={t("additem.restoreDefaultTitle")}
                  className="px-2.5 py-1.5 bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 hover:text-white text-xs rounded-lg transition cursor-pointer flex items-center gap-1.5 disabled:opacity-40 disabled:cursor-not-allowed"
                >
                  <RotateCcw className="w-3.5 h-3.5 text-cyan-400" />
                  {t("additem.restoreDefault")}
                </button>
                <button
                  type="button"
                  onClick={handleClearIcon}
                  disabled={!icon && !htmlIcon}
                  title={t("additem.clearIconTitle")}
                  className="px-2.5 py-1.5 bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 hover:text-white text-xs rounded-lg transition cursor-pointer flex items-center gap-1.5 disabled:opacity-40 disabled:cursor-not-allowed"
                >
                  <X className="w-3.5 h-3.5 text-red-400" />
                  {t("additem.clearIcon")}
                </button>
                <div className="flex items-center gap-1">
                  <button
                    type="button"
                    onClick={() => setIconBg((v) => !v)}
                    title={t("additem.iconBgTitle")}
                    className={`px-2.5 py-1.5 border text-xs rounded-lg transition cursor-pointer flex items-center gap-1.5 ${
                      iconBg
                        ? "bg-white/10 border-[var(--module-accent-ring)] text-white"
                        : "bg-white/5 border-white/10 text-slate-300 hover:bg-white/10 hover:text-white"
                    }`}
                  >
                    <span
                      className="w-3 h-3 rounded-sm"
                      style={{ backgroundColor: iconBgValue }}
                    />
                    {t("additem.iconBg")}
                  </button>
                  {iconBg && (
                    <label
                      className="w-8 h-8 rounded-lg overflow-hidden border border-white/10 cursor-pointer flex items-center justify-center hover:bg-white/10 transition"
                      title={t("additem.iconBgColorTitle")}
                    >
                      <input
                        type="color"
                        value={iconBgValue}
                        onChange={(e) => setIconBgValue(e.target.value)}
                        className="w-10 h-10 -m-1 cursor-pointer"
                      />
                    </label>
                  )}
                </div>
              </div>
            </div>

            {/* 网络图标输入（参考 DawnLauncher NetworkIcon） */}
            {netIconOpen && (
              <>
                <div className="mt-2 flex items-center gap-2">
                  <input
                    type="text"
                    autoComplete="off"
                    spellCheck={false}
                    value={netIconUrl}
                    onChange={(e) => setNetIconUrl(e.target.value)}
                    placeholder={t("additem.netIconPlaceholder")}
                    className="flex-1 bg-white/5 border border-white/10 rounded-xl px-3 py-1.5 text-xs text-white focus:outline-none focus:border-purple-500 select-text"
                  />
                  <button
                    type="button"
                    onClick={handleDownloadNetIcon}
                    disabled={netIconLoading || !netIconUrl.trim()}
                    className="px-3 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white text-xs rounded-xl transition cursor-pointer flex items-center gap-1.5"
                  >
                    <Download className={`w-3.5 h-3.5 ${netIconLoading ? "animate-pulse" : ""}`} />
                    {netIconLoading ? t("additem.downloading") : t("additem.download")}
                  </button>
                </div>
                {netIconError && (
                  <p className="mt-1.5 text-[11px] text-red-400 leading-relaxed break-all">
                    {t("additem.downloadFailed", { err: netIconError })}
                  </p>
                )}
              </>
            )}
            <p className="text-[10px] text-slate-500 mt-1.5 leading-relaxed">
              {t("additem.iconHint")}
            </p>
          </div>

          {/* TAB 0: File / Executable */}
          {activeTab === 0 && (
            <div className="space-y-3">
              <div>
                <label className="block text-xs font-medium text-slate-400 mb-1.5">{t("additem.fileTarget")}</label>
                <div className="flex items-center gap-2">
                  <input
                    type="text"
                    required
                    autoComplete="off"
                    spellCheck={false}
                    value={target}
                    onChange={(e) => setTarget(e.target.value)}
                    placeholder={t("additem.fileTargetPlaceholder")}
                    className="flex-1 bg-white/5 border border-white/10 rounded-xl px-3 py-2 text-xs text-white focus:outline-none focus:border-purple-500 select-text"
                  />
                  <button
                    type="button"
                    onClick={handleSelectFile}
                    className="px-3.5 py-2 bg-purple-600 hover:bg-purple-500 text-white text-xs rounded-xl transition cursor-pointer flex items-center gap-1.5"
                  >
                    <Upload className="w-3.5 h-3.5" />
                    {t("additem.browseFile")}
                  </button>
                </div>
              </div>

              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                <div>
                  <label className="block text-xs font-medium text-slate-400 mb-1.5">{t("additem.params")}</label>
                  <input
                    type="text"
                    autoComplete="off"
                    spellCheck={false}
                    value={params}
                    onChange={(e) => setParams(e.target.value)}
                    placeholder={t("additem.paramsExample")}
                    className="w-full bg-white/5 border border-white/10 rounded-xl px-3 py-2 text-xs text-white focus:outline-none focus:border-purple-500 select-text"
                  />
                </div>
                <div>
                  <label className="block text-xs font-medium text-slate-400 mb-1.5">{t("additem.startDir")}</label>
                  <input
                    type="text"
                    autoComplete="off"
                    spellCheck={false}
                    value={startLocation}
                    onChange={(e) => setStartLocation(e.target.value)}
                    placeholder={t("additem.startDirPlaceholder")}
                    className="w-full bg-white/5 border border-white/10 rounded-xl px-3 py-2 text-xs text-white focus:outline-none focus:border-purple-500 select-text"
                  />
                </div>
              </div>
            </div>
          )}

          {/* TAB 1: Folder */}
          {activeTab === 1 && (
            <div>
              <label className="block text-xs font-medium text-slate-400 mb-1.5">{t("additem.folderTarget")}</label>
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  required
                  autoComplete="off"
                  spellCheck={false}
                  value={target}
                  onChange={(e) => setTarget(e.target.value)}
                  placeholder={t("additem.folderPlaceholder")}
                  className="flex-1 bg-white/5 border border-white/10 rounded-xl px-3 py-2 text-xs text-white focus:outline-none focus:border-purple-500 select-text"
                />
                <button
                  type="button"
                  onClick={handleSelectFolder}
                  className="px-3.5 py-2 bg-purple-600 hover:bg-purple-500 text-white text-xs rounded-xl transition cursor-pointer flex items-center gap-1.5"
                >
                  <Folder className="w-3.5 h-3.5" />
                  {t("additem.browseFolder")}
                </button>
              </div>
            </div>
          )}

          {/* TAB 2: Web URL */}
          {activeTab === 2 && (
            <div className="space-y-3">
              <div>
                <label className="block text-xs font-medium text-slate-400 mb-1.5">{t("additem.urlLabel")}</label>
                <div className="flex items-center gap-2">
                  <input
                    type="text"
                    required
                    autoComplete="off"
                    spellCheck={false}
                    value={target}
                    onChange={(e) => setTarget(e.target.value)}
                    placeholder={t("additem.urlExample")}
                    className="flex-1 bg-white/5 border border-white/10 rounded-xl px-3 py-2 text-xs text-white focus:outline-none focus:border-purple-500 select-text"
                  />
                  <button
                    type="button"
                    onClick={handleFetchUrl}
                    disabled={urlFetching || !target.trim()}
                    className="px-3.5 py-2 bg-purple-600 hover:bg-purple-500 disabled:opacity-50 text-white text-xs rounded-xl transition cursor-pointer flex items-center gap-1.5"
                  >
                    <RefreshCw className={`w-3.5 h-3.5 ${urlFetching ? "animate-spin" : ""}`} />
                    {urlFetching ? t("additem.fetching") : t("additem.fetchInfo")}
                  </button>
                </div>
              </div>
            </div>
          )}

          {/* TAB 3: System Tools */}
          {activeTab === 3 && (
            <div>
              <label className="block text-xs font-medium text-slate-400 mb-1.5">{t("additem.pickSystemTool")}</label>
              <div className="grid grid-cols-2 sm:grid-cols-3 gap-2 max-h-48 overflow-y-auto p-1 bg-white/[0.02] border border-white/5 rounded-xl">
                {PRESET_SYSTEM_TOOLS.map((sys) => (
                  <button
                    type="button"
                    key={sys.name}
                    onClick={() => {
                      // 英文界面存储/显示英文名与说明（预设中文原义见词典 t(sys.name)/t(sys.desc)）
                      setName(t(sys.name));
                      setTarget(sys.target);
                      setParams(sys.params);
                      setHtmlIcon(sys.icon);
                      setIcon(null);
                      setRemark(t(sys.desc));
                    }}
                    className={`p-2 rounded-xl text-left border transition cursor-pointer flex items-center gap-2 ${
                      target === sys.target
                        ? "bg-purple-600/20 border-purple-500 text-white"
                        : "bg-white/[0.02] border-white/5 text-slate-300 hover:bg-white/[0.05]"
                    }`}
                  >
                    <span className="text-lg">{sys.icon}</span>
                    <div className="min-w-0">
                      <p className="text-xs font-medium truncate">{t(sys.name)}</p>
                    </div>
                  </button>
                ))}
              </div>
            </div>
          )}

          {/* TAB 4: Appx Applications */}
          {activeTab === 4 && (
            <div className="space-y-2.5">
              <div className="flex items-center justify-between">
                <label className="text-xs font-medium text-slate-400">{t("additem.appxLabel")}</label>
                <button
                  type="button"
                  onClick={handleScanAppx}
                  disabled={appxScanning}
                  className="text-xs text-purple-400 hover:text-purple-300 flex items-center gap-1 cursor-pointer"
                >
                  <RefreshCw className={`w-3 h-3 ${appxScanning ? "animate-spin" : ""}`} />
                  {t("additem.rescan")}
                </button>
              </div>

              <div className="relative">
                <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-slate-500" />
                <input
                  type="text"
                  autoComplete="off"
                  spellCheck={false}
                  value={appxSearch}
                  onChange={(e) => setAppxSearch(e.target.value)}
                  placeholder={t("additem.searchAppx")}
                  className="w-full bg-white/5 border border-white/10 rounded-xl pl-9 pr-3 py-1.5 text-xs text-white focus:outline-none focus:border-purple-500 select-text"
                />
              </div>

              <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 max-h-48 overflow-y-auto p-1 bg-white/[0.02] border border-white/5 rounded-xl">
                {appxList
                  .filter((a) =>
                    !appxSearch.trim() ||
                    a.displayName.toLowerCase().includes(appxSearch.toLowerCase()) ||
                    a.familyName.toLowerCase().includes(appxSearch.toLowerCase())
                  )
                  .map((app) => (
                    <button
                      type="button"
                      key={`${app.familyName}-${app.appId}`}
                      onClick={() => {
                        setName(app.displayName);
                        setTarget(`${app.familyName}!${app.appId}`);
                        if (app.logo) setIcon(app.logo);
                        setHtmlIcon("📱");
                      }}
                      className={`p-2 rounded-xl text-left border transition cursor-pointer flex items-center gap-2.5 ${
                        target.includes(app.familyName)
                          ? "bg-purple-600/20 border-purple-500 text-white"
                          : "bg-white/[0.02] border-white/5 text-slate-300 hover:bg-white/[0.05]"
                      }`}
                    >
                      {app.logo ? (
                        <img src={app.logo} className="w-6 h-6 object-contain flex-shrink-0" alt="" />
                      ) : (
                        <span className="text-base">📱</span>
                      )}
                      <div className="min-w-0">
                        <p className="text-xs font-medium truncate">{app.displayName}</p>
                        <p className="text-[10px] text-slate-500 truncate">{app.familyName}</p>
                      </div>
                    </button>
                  ))}
              </div>
            </div>
          )}

          {/* TAB 5: Start Menu Programs */}
          {activeTab === 5 && (
            <div className="space-y-2.5">
              <div className="flex items-center justify-between">
                <label className="text-xs font-medium text-slate-400">{t("additem.startMenuLabel")}</label>
                <button
                  type="button"
                  onClick={handleScanStartMenu}
                  disabled={startMenuScanning}
                  className="text-xs text-purple-400 hover:text-purple-300 flex items-center gap-1 cursor-pointer"
                >
                  <RefreshCw className={`w-3 h-3 ${startMenuScanning ? "animate-spin" : ""}`} />
                  {t("additem.rescanMenu")}
                </button>
              </div>

              <div className="relative">
                <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-slate-500" />
                <input
                  type="text"
                  autoComplete="off"
                  spellCheck={false}
                  value={startMenuSearch}
                  onChange={(e) => setStartMenuSearch(e.target.value)}
                  placeholder={t("additem.searchStartMenu")}
                  className="w-full bg-white/5 border border-white/10 rounded-xl pl-9 pr-3 py-1.5 text-xs text-white focus:outline-none focus:border-purple-500 select-text"
                />
              </div>

              <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 max-h-48 overflow-y-auto p-1 bg-white/[0.02] border border-white/5 rounded-xl">
                {startMenuList
                  .filter((p) =>
                    !startMenuSearch.trim() ||
                    p.name.toLowerCase().includes(startMenuSearch.toLowerCase()) ||
                    p.target.toLowerCase().includes(startMenuSearch.toLowerCase())
                  )
                  .map((prog, idx) => (
                    <button
                      type="button"
                      key={`${prog.target}-${idx}`}
                      onClick={() => {
                        setName(prog.name);
                        setTarget(prog.target);
                        if (prog.params) setParams(prog.params);
                        if (prog.icon) setIcon(prog.icon);
                      }}
                      className={`p-2 rounded-xl text-left border transition cursor-pointer flex items-center gap-2.5 ${
                        target === prog.target
                          ? "bg-purple-600/20 border-purple-500 text-white"
                          : "bg-white/[0.02] border-white/5 text-slate-300 hover:bg-white/[0.05]"
                      }`}
                    >
                      {prog.icon ? (
                        <img src={prog.icon} className="w-6 h-6 object-contain flex-shrink-0" alt="" />
                      ) : (
                        <span className="text-base">🚀</span>
                      )}
                      <div className="min-w-0">
                        <p className="text-xs font-medium truncate">{prog.name}</p>
                        <p className="text-[10px] text-slate-500 truncate">{prog.category || prog.target}</p>
                      </div>
                    </button>
                  ))}
              </div>
            </div>
          )}

          {/* TAB 6: Multi-Item */}
          {activeTab === 6 && (
            <div className="space-y-2.5">
              <div className="flex items-center justify-between">
                <label className="text-xs font-medium text-slate-400">{t("additem.multiLabel")}</label>
                <button
                  type="button"
                  onClick={() =>
                    setMultiItems((prev) => [
                      ...prev,
                      { name: t("additem.subTaskName", { n: prev.length + 1 }), target: "", params: "", runAsAdmin: false, delayMs: 500 },
                    ])
                  }
                  className="px-2.5 py-1 bg-purple-600/30 hover:bg-purple-600/50 text-purple-300 text-xs rounded-lg flex items-center gap-1 cursor-pointer"
                >
                  <Plus className="w-3 h-3" />
                  {t("additem.addSubItem")}
                </button>
              </div>

              <div className="space-y-2 max-h-48 overflow-y-auto">
                {multiItems.map((sub, idx) => (
                  <div key={idx} className="p-2.5 rounded-xl bg-white/[0.02] border border-white/5 flex items-center gap-2">
                    <span className="text-xs text-slate-500 w-5 text-center">{idx + 1}</span>
                    <input
                      type="text"
                      autoComplete="off"
                      spellCheck={false}
                      value={sub.name}
                      onChange={(e) => {
                        const next = [...multiItems];
                        next[idx] = { ...next[idx], name: e.target.value };
                        setMultiItems(next);
                      }}
                      placeholder={t("additem.subItemName")}
                      className="w-24 bg-black/20 border border-white/10 rounded-lg px-2 py-1 text-xs text-white select-text"
                    />
                    <input
                      type="text"
                      autoComplete="off"
                      spellCheck={false}
                      value={sub.target}
                      onChange={(e) => {
                        const next = [...multiItems];
                        next[idx] = { ...next[idx], target: e.target.value };
                        setMultiItems(next);
                      }}
                      placeholder={t("additem.subTarget")}
                      className="flex-1 bg-black/20 border border-white/10 rounded-lg px-2 py-1 text-xs text-white select-text"
                    />
                    <div className="flex items-center gap-1">
                      <span className="text-[10px] text-slate-400">{t("additem.delay")}</span>
                      <input
                        type="number"
                        min={0}
                        step={100}
                        value={sub.delayMs}
                        onChange={(e) => {
                          const next = [...multiItems];
                          next[idx] = { ...next[idx], delayMs: Number(e.target.value) };
                          setMultiItems(next);
                        }}
                        className="w-16 bg-black/20 border border-white/10 rounded-lg px-1.5 py-1 text-xs text-white text-center select-text"
                      />
                      <span className="text-[10px] text-slate-500">ms</span>
                    </div>
                    <label className="flex items-center gap-1 text-[10px] text-amber-300 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={sub.runAsAdmin}
                        onChange={(e) => {
                          const next = [...multiItems];
                          next[idx] = { ...next[idx], runAsAdmin: e.target.checked };
                          setMultiItems(next);
                        }}
                        className="rounded border-white/10 bg-white/5"
                      />
                      {t("additem.elevate")}
                    </label>
                    <button
                      type="button"
                      onClick={() => setMultiItems(multiItems.filter((_, i) => i !== idx))}
                      className="p-1 text-slate-500 hover:text-red-400 rounded cursor-pointer"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Admin Run & Remarks */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 pt-2">
            <div className="flex items-center gap-2">
              <label className="flex items-center gap-2 cursor-pointer text-xs text-amber-400 font-medium">
                <input
                  type="checkbox"
                  checked={runAsAdmin}
                  onChange={(e) => setRunAsAdmin(e.target.checked)}
                  className="rounded border-amber-500/30 bg-amber-500/10 text-amber-500 focus:ring-0"
                />
                <Shield className="w-3.5 h-3.5" />
                <span>{t("additem.runAsAdmin")}</span>
              </label>
            </div>
            <div>
              <input
                type="text"
                autoComplete="off"
                spellCheck={false}
                value={remark}
                onChange={(e) => setRemark(e.target.value)}
                placeholder={t("additem.remarkPlaceholder")}
                className="w-full bg-white/5 border border-white/10 rounded-xl px-3 py-1.5 text-xs text-white focus:outline-none focus:border-purple-500 select-text"
              />
            </div>
          </div>

          {/* Actions */}
          <div className="flex items-center justify-end gap-2.5 pt-3 border-t border-white/10">
            <button
              type="button"
              onClick={onClose}
              className="px-4 py-2 rounded-xl text-xs font-medium text-slate-400 hover:text-white hover:bg-white/5 transition cursor-pointer"
            >
              {t("additem.cancel")}
            </button>
            <button
              type="submit"
              disabled={saving}
              className="px-5 py-2 rounded-xl text-xs font-semibold bg-purple-600 hover:bg-purple-500 text-white shadow-lg shadow-purple-600/30 transition cursor-pointer disabled:opacity-50 flex items-center gap-1.5"
            >
              <Check className="w-3.5 h-3.5" />
              {saving ? t("additem.saving") : t("additem.saveItem")}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
