import { Item, Classification } from "./types";
export interface AddItemModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSave: (item: Item) => Promise<void>;
  editingItem: Item | null;
  classificationId: number;
  classifications: Classification[];
}


// 预设 Windows 核心系统工具
export const PRESET_SYSTEM_TOOLS = [
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
