using System;
using System.IO;
using System.Diagnostics;
using System.Windows.Forms;
using System.Drawing;
using System.Threading;
using System.Collections.Generic;
using System.Text;
using Microsoft.Win32;

namespace AnyVersion
{
    public class Program
    {
        [STAThread]
        public static void Main()
        {
            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);
            Application.Run(new AnyVersionForm());
        }
    }

    public class AnyVersionForm : Form
    {
        private Panel sidebar;
        private Panel contentPanel;
        private Label logoLabel;
        
        // 导航按钮
        private Button btnSdks;
        private Button btnServices;
        private Button btnCaches;
        private Button btnMirrors;
        private Button btnEnv;
        private Button btnSystemTools;
        private Button btnSettings;

        // 视图面板
        private Panel panelSdks;
        private Panel panelServices;
        private Panel panelCaches;
        private Panel panelMirrors;
        private Panel panelEnv;
        private Panel panelSystemTools;
        private Panel panelSettings;

        // SDK 视图控件
        private TreeView tvSdks;
        private Label lblSdkName;
        private Label lblActiveVer;
        private Label lblLinkPath;
        private Label lblTargetPath;
        private ListBox lbInstalledVersions;
        private ComboBox cbRemoteVersions;
        private Button btnSetActive;
        private Button btnInstall;
        private Button btnUninstall;
        private Button btnAddFolder;
        private ProgressBar prgInstall;
        private Label lblProgressStatus;

        // 服务视图控件
        private ListView lvServices;
        private Button btnStartService;
        private Button btnStopService;
        private Button btnEditServiceConfig;
        private Button btnOpenServiceDir;
        private System.Windows.Forms.Timer timerServices;

        // 缓存视图控件
        private ListView lvCaches;
        private Button btnEditCache;
        private Label lblCacheLoading;

        // 全局包管理控件
        private Panel panelPkgs;
        private Button btnPkgs;
        private ComboBox cbPkgSdks;
        private ListView lvPkgs;
        private Button btnPkgRefresh;
        private Button btnPkgUpgrade;
        private Button btnPkgHomepage;
        private Label lblPkgLoading;

        // 镜像源视图控件
        private ListView lvMirrors;
        private ComboBox cbMirrorPresets;
        private Button btnApplyMirror;

        // 环境变量与诊断控件
        private ListView lvEnv;
        private ListView lvDiagnostics;
        private TextBox txtDiagDetail;
        private Button btnScanEnv;
        private Button btnOptimizePath;

        // 系统工具控件
        private TextBox txtPortToCheck;
        private Button btnCheckPort;
        private Label lblPortStatus;
        private Button btnKillPortProcess;
        private TextBox txtHostsContent;
        private TabControl tabHosts;
        private TabPage tabPageVisual;
        private TabPage tabPageRaw;
        private ListView lvHostsVisual;
        private TextBox txtNewHostIp;
        private TextBox txtNewHostName;
        private Button btnNewHostAdd;
        private Button btnHostToggle;
        private Button btnHostDelete;

        public class HostEntry
        {
            public string Ip;
            public string Hostname;
            public bool Enabled;
            public string TrailingComment;
        }

        public class HostsLine
        {
            public string Raw;
            public bool IsHostEntry;
            public HostEntry Entry;
        }

        private List<HostsLine> hostsLines = new List<HostsLine>();

        public class RegistrySoftware
        {
            public string DisplayName;
            public string DisplayVersion;
            public string UninstallString;
            public string InstallLocation;
            public string RegistryKey;
        }

        private List<RegistrySoftware> detectedSoftware = new List<RegistrySoftware>();
        private Button btnFixSelected;
        private Button btnSaveHosts;
        private Button btnApplyHostsTemplate;

        // 全局设置控件
        private TextBox txtVersionsDir;
        private TextBox txtLinksDir;
        private Button btnSaveSettings;

        // 主题配色
        private Color colorBg = Color.FromArgb(20, 20, 26);
        private Color colorPanelBg = Color.FromArgb(30, 30, 40);
        private Color colorSidebarBg = Color.FromArgb(17, 17, 25);
        private Color colorTextMain = Color.FromArgb(243, 244, 246);
        private Color colorTextMuted = Color.FromArgb(156, 163, 175);
        private Color colorAccent = Color.FromArgb(99, 102, 241);
        private Color colorSuccess = Color.FromArgb(16, 185, 129);
        private Color colorDanger = Color.FromArgb(239, 68, 68);
        private Color colorControlBg = Color.FromArgb(21, 28, 38);

        public AnyVersionForm()
        {
            this.Text = "Any-Version 本地开发集成工作站";
            this.Size = new Size(1200, 600);
            this.FormBorderStyle = FormBorderStyle.FixedSingle;
            this.MaximizeBox = false;
            this.StartPosition = FormStartPosition.CenterScreen;
            this.BackColor = colorBg;
            this.ForeColor = colorTextMain;
            this.Font = new Font("Segoe UI", 9.5F, FontStyle.Regular);

            InitializeComponents();
            StyleForm();
            LoadConfigData();
            LoadSdkList();
            ShowPanel(panelSdks, btnSdks);

            // 定时器：每 3 秒后台刷新一次本地服务状态
            timerServices = new System.Windows.Forms.Timer();
            timerServices.Interval = 3000;
            timerServices.Tick += (s, e) => { if (panelServices.Visible) LoadServicesList(true); };
            timerServices.Start();
        }

        private void InitializeComponents()
        {
            // --- 布局面板 ---
            sidebar = new Panel { Width = 220, Dock = DockStyle.Left };
            contentPanel = new Panel { Dock = DockStyle.Fill };
            this.Controls.Add(contentPanel);
            this.Controls.Add(sidebar);

            // --- 侧边栏 Logo ---
            logoLabel = new Label
            {
                Text = "Any-Version",
                Font = new Font("Segoe UI", 16F, FontStyle.Bold),
                ForeColor = Color.White,
                Location = new Point(20, 25),
                Size = new Size(180, 40),
                TextAlign = ContentAlignment.MiddleLeft
            };
            sidebar.Controls.Add(logoLabel);

            // --- 侧边栏导航按钮 ---
            btnSdks = CreateNavButton("SDK 版本管理", 85);
            btnServices = CreateNavButton("本地服务管理", 135);
            btnCaches = CreateNavButton("开发缓存清理", 185);
            btnMirrors = CreateNavButton("国内镜像配置", 235);
            btnEnv = CreateNavButton("集成环境体检", 285);
            btnSystemTools = CreateNavButton("系统实用工具", 335);
            btnPkgs = CreateNavButton("全局包管理", 385);
            btnSettings = CreateNavButton("全局路径设置", 435);

            btnSdks.Click += (s, e) => { ShowPanel(panelSdks, btnSdks); LoadSdkList(); };
            btnServices.Click += (s, e) => { ShowPanel(panelServices, btnServices); LoadServicesList(false); };
            btnCaches.Click += (s, e) => { ShowPanel(panelCaches, btnCaches); LoadCachesList(); };
            btnMirrors.Click += (s, e) => { ShowPanel(panelMirrors, btnMirrors); LoadMirrorsList(); };
            btnEnv.Click += (s, e) => { ShowPanel(panelEnv, btnEnv); RunHealthCheck(); };
            btnSystemTools.Click += (s, e) => { ShowPanel(panelSystemTools, btnSystemTools); LoadHostsFile(); };
            btnPkgs.Click += (s, e) => { ShowPanel(panelPkgs, btnPkgs); LoadPkgsList(); };
            btnSettings.Click += (s, e) => { ShowPanel(panelSettings, btnSettings); LoadConfigData(); };

            // --- 视图面板初始化 ---
            panelSdks = new Panel { Dock = DockStyle.Fill, Visible = false };
            panelServices = new Panel { Dock = DockStyle.Fill, Visible = false };
            panelCaches = new Panel { Dock = DockStyle.Fill, Visible = false };
            panelMirrors = new Panel { Dock = DockStyle.Fill, Visible = false };
            panelEnv = new Panel { Dock = DockStyle.Fill, Visible = false };
            panelPkgs = new Panel { Dock = DockStyle.Fill, Visible = false };
            panelSystemTools = new Panel { Dock = DockStyle.Fill, Visible = false };
            panelSettings = new Panel { Dock = DockStyle.Fill, Visible = false };

            contentPanel.Controls.Add(panelSdks);
            contentPanel.Controls.Add(panelServices);
            contentPanel.Controls.Add(panelCaches);
            contentPanel.Controls.Add(panelMirrors);
            contentPanel.Controls.Add(panelEnv);
            contentPanel.Controls.Add(panelPkgs);
            contentPanel.Controls.Add(panelSystemTools);
            contentPanel.Controls.Add(panelSettings);

            InitializeSdkPanel();
            InitializeServicesPanel();
            InitializeCachePanel();
            InitializeMirrorsPanel();
            InitializeEnvPanel();
            InitializePkgsPanel();
            InitializeSystemToolsPanel();
            InitializeSettingsPanel();
        }

        private Button CreateNavButton(string text, int top)
        {
            var btn = new Button
            {
                Text = text,
                Location = new Point(15, top),
                Size = new Size(190, 42),
                FlatStyle = FlatStyle.Flat,
                TextAlign = ContentAlignment.MiddleLeft,
                Padding = new Padding(12, 0, 0, 0),
                Cursor = Cursors.Hand,
                Font = new Font("Microsoft YaHei", 9.5F, FontStyle.Bold)
            };
            btn.FlatAppearance.BorderSize = 0;
            sidebar.Controls.Add(btn);
            return btn;
        }

        private void InitializeSdkPanel()
        {
            var lblTitle = new Label { Text = "SDK 版本管理", Font = new Font("Microsoft YaHei", 15F, FontStyle.Bold), Location = new Point(25, 20), Size = new Size(300, 35) };
            var lblSubtitle = new Label { Text = "在本地轻松切换和管理各种编程语言/开发库的多个版本。", ForeColor = colorTextMuted, Location = new Point(25, 55), Size = new Size(500, 20) };
            panelSdks.Controls.Add(lblTitle);
            panelSdks.Controls.Add(lblSubtitle);

            tvSdks = new TreeView { Location = new Point(25, 90), Size = new Size(220, 460), Cursor = Cursors.Hand, BorderStyle = BorderStyle.FixedSingle, ShowLines = true, ShowPlusMinus = true };
            tvSdks.AfterSelect += (s, e) => LoadSdkDetails();
            panelSdks.Controls.Add(tvSdks);

            var detailsPanel = new Panel { Location = new Point(260, 90), Size = new Size(695, 460), BackColor = colorPanelBg };
            panelSdks.Controls.Add(detailsPanel);

            lblSdkName = new Label { Text = "选择一个 SDK", Font = new Font("Microsoft YaHei", 12F, FontStyle.Bold), Location = new Point(20, 15), Size = new Size(300, 25) };
            detailsPanel.Controls.Add(lblSdkName);

            var lblActiveTag = new Label { Text = "当前启用版本:", ForeColor = colorTextMuted, Location = new Point(20, 50), Size = new Size(110, 20) };
            lblActiveVer = new Label { Text = "(无)", Font = new Font("Microsoft YaHei", 9.5F, FontStyle.Bold), Location = new Point(140, 50), Size = new Size(530, 20) };
            detailsPanel.Controls.Add(lblActiveTag);
            detailsPanel.Controls.Add(lblActiveVer);

            var lblLinkTag = new Label { Text = "虚拟 Link 路径:", ForeColor = colorTextMuted, Location = new Point(20, 75), Size = new Size(110, 20) };
            lblLinkPath = new Label { Text = "-", Location = new Point(140, 75), Size = new Size(530, 20) };
            detailsPanel.Controls.Add(lblLinkTag);
            detailsPanel.Controls.Add(lblLinkPath);

            var lblTargetTag = new Label { Text = "实际安装目录:", ForeColor = colorTextMuted, Location = new Point(20, 100), Size = new Size(110, 20) };
            lblTargetPath = new Label { Text = "-", Location = new Point(140, 100), Size = new Size(530, 20) };
            detailsPanel.Controls.Add(lblTargetTag);
            detailsPanel.Controls.Add(lblTargetPath);

            var lblInstalledTag = new Label { Text = "本地已安装版本:", ForeColor = colorTextMuted, Location = new Point(20, 130), Size = new Size(150, 20) };
            lbInstalledVersions = new ListBox { Location = new Point(20, 155), Size = new Size(250, 170), BorderStyle = BorderStyle.FixedSingle };
            detailsPanel.Controls.Add(lblInstalledTag);
            detailsPanel.Controls.Add(lbInstalledVersions);

            btnSetActive = CreateThemeButton("切换为此版本", 290, 155, 130, 32, true);
            btnUninstall = CreateThemeButton("卸载此版本", 290, 195, 130, 32, false, true);
            btnAddFolder = CreateThemeButton("导入本地版本", 290, 235, 130, 32, false);

            btnSetActive.Click += btnSetActive_Click;
            btnUninstall.Click += btnUninstall_Click;
            btnAddFolder.Click += btnAddFolder_Click;

            detailsPanel.Controls.Add(btnSetActive);
            detailsPanel.Controls.Add(btnUninstall);
            detailsPanel.Controls.Add(btnAddFolder);

            var lblRemoteTag = new Label { Text = "从远程下载并安装:", ForeColor = colorTextMuted, Location = new Point(20, 340), Size = new Size(200, 20) };
            cbRemoteVersions = new ComboBox { Location = new Point(20, 365), Size = new Size(250, 28), DropDownStyle = ComboBoxStyle.DropDownList };
            btnInstall = CreateThemeButton("开始下载安装", 290, 364, 130, 28, true);
            btnInstall.Click += btnInstall_Click;

            detailsPanel.Controls.Add(lblRemoteTag);
            detailsPanel.Controls.Add(cbRemoteVersions);
            detailsPanel.Controls.Add(btnInstall);

            prgInstall = new ProgressBar { Location = new Point(20, 405), Size = new Size(655, 12), Visible = false };
            lblProgressStatus = new Label { Text = "正在准备下载...", ForeColor = colorAccent, Location = new Point(20, 420), Size = new Size(655, 20), Visible = false };
            detailsPanel.Controls.Add(prgInstall);
            detailsPanel.Controls.Add(lblProgressStatus);
        }

        private void InitializeServicesPanel()
        {
            var lblTitle = new Label { Text = "本地服务管理", Font = new Font("Microsoft YaHei", 15F, FontStyle.Bold), Location = new Point(25, 20), Size = new Size(350, 35) };
            var lblSubtitle = new Label { Text = "在当前用户会话中启动和停止 Nginx、Redis 和 MySQL，极其节省系统资源。", ForeColor = colorTextMuted, Location = new Point(25, 55), Size = new Size(500, 20) };
            panelServices.Controls.Add(lblTitle);
            panelServices.Controls.Add(lblSubtitle);

            lvServices = new ListView { Location = new Point(25, 90), Size = new Size(930, 240), View = View.Details, FullRowSelect = true, BorderStyle = BorderStyle.FixedSingle, HeaderStyle = ColumnHeaderStyle.Nonclickable };
            lvServices.Columns.Add("服务名称", 180);
            lvServices.Columns.Add("运行状态", 120);
            lvServices.Columns.Add("当前版本", 180);
            lvServices.Columns.Add("端口号", 120);
            lvServices.Columns.Add("进程 PID", 120);
            lvServices.SelectedIndexChanged += lvServices_SelectedIndexChanged;
            panelServices.Controls.Add(lvServices);

            var controlGroup = new Panel { Location = new Point(25, 350), Size = new Size(930, 200), BackColor = colorPanelBg };
            panelServices.Controls.Add(controlGroup);

            btnStartService = CreateThemeButton("启动服务", 20, 25, 180, 38, true);
            btnStopService = CreateThemeButton("停止服务", 220, 25, 180, 38, false, true);
            btnEditServiceConfig = CreateThemeButton("修改配置文件", 420, 25, 180, 38, false);
            btnOpenServiceDir = CreateThemeButton("打开安装目录", 620, 25, 180, 38, false);

            btnStartService.Click += btnStartService_Click;
            btnStopService.Click += btnStopService_Click;
            btnEditServiceConfig.Click += btnEditServiceConfig_Click;
            btnOpenServiceDir.Click += btnOpenServiceDir_Click;

            controlGroup.Controls.Add(btnStartService);
            controlGroup.Controls.Add(btnStopService);
            controlGroup.Controls.Add(btnEditServiceConfig);
            controlGroup.Controls.Add(btnOpenServiceDir);

            var lblTips = new Label
            {
                Text = "提示：这里启动的服务运行在您当前的用户会话下（不占多余后台资源），而不是作为隐藏的 Windows 系统服务运行。默认情况下，Nginx 占用 80 端口，Redis 占用 6379 端口，MySQL 占用 3306 端口。若提示端口被占用，请在“系统实用工具”标签页中检测并释放端口。",
                Location = new Point(20, 85),
                Size = new Size(890, 80),
                ForeColor = colorTextMuted,
                Font = new Font("Microsoft YaHei", 9F, FontStyle.Italic)
            };
            controlGroup.Controls.Add(lblTips);
        }

        private void InitializeCachePanel()
        {
            var lblTitle = new Label { Text = "开发缓存清理", Font = new Font("Microsoft YaHei", 15F, FontStyle.Bold), Location = new Point(25, 20), Size = new Size(350, 35) };
            var lblSubtitle = new Label { Text = "检测并重定向开发包管理器的本地缓存目录，迁移至其他盘以节省 C 盘空间。", ForeColor = colorTextMuted, Location = new Point(25, 55), Size = new Size(500, 20) };
            panelCaches.Controls.Add(lblTitle);
            panelCaches.Controls.Add(lblSubtitle);

            lvCaches = new ListView { Location = new Point(25, 90), Size = new Size(930, 400), View = View.Details, FullRowSelect = true, BorderStyle = BorderStyle.FixedSingle, HeaderStyle = ColumnHeaderStyle.Nonclickable };
            lvCaches.Columns.Add("开发工具", 120);
            lvCaches.Columns.Add("本地安装", 120);
            lvCaches.Columns.Add("当前缓存目录", 550);
            lvCaches.Columns.Add("已用空间", 120);
            lvCaches.SelectedIndexChanged += (s, e) => btnEditCache.Enabled = lvCaches.SelectedItems.Count > 0;
            panelCaches.Controls.Add(lvCaches);

            btnEditCache = CreateThemeButton("重定向并迁移缓存路径", 25, 505, 280, 36, true);
            btnEditCache.Enabled = false;
            btnEditCache.Click += btnEditCache_Click;
            panelCaches.Controls.Add(btnEditCache);

            lblCacheLoading = new Label
            {
                Text = "正在扫描开发缓存文件夹大小... 请稍候...",
                Font = new Font("Microsoft YaHei", 10.5F, FontStyle.Bold),
                ForeColor = colorAccent,
                BackColor = colorControlBg,
                TextAlign = ContentAlignment.MiddleCenter,
                Visible = false
            };
            panelCaches.Controls.Add(lblCacheLoading);
        }

        private void InitializeMirrorsPanel()
        {
            var lblTitle = new Label { Text = "国内镜像源快速配置", Font = new Font("Microsoft YaHei", 15F, FontStyle.Bold), Location = new Point(25, 20), Size = new Size(450, 35) };
            var lblSubtitle = new Label { Text = "一键切换各开发语言包管理器到国内高速镜像源，彻底解决依赖下载慢的问题。", ForeColor = colorTextMuted, Location = new Point(25, 55), Size = new Size(500, 20) };
            panelMirrors.Controls.Add(lblTitle);
            panelMirrors.Controls.Add(lblSubtitle);

            lvMirrors = new ListView { Location = new Point(25, 90), Size = new Size(930, 240), View = View.Details, FullRowSelect = true, BorderStyle = BorderStyle.FixedSingle, HeaderStyle = ColumnHeaderStyle.Nonclickable };
            lvMirrors.Columns.Add("包管理器", 180);
            lvMirrors.Columns.Add("当前下载源 URL", 550);
            lvMirrors.Columns.Add("已应用预设", 180);
            lvMirrors.SelectedIndexChanged += lvMirrors_SelectedIndexChanged;
            panelMirrors.Controls.Add(lvMirrors);

            var controlGroup = new Panel { Location = new Point(25, 350), Size = new Size(930, 200), BackColor = colorPanelBg };
            panelMirrors.Controls.Add(controlGroup);

            var lblSelectPreset = new Label { Text = "选择国内镜像源预设：", Font = new Font("Microsoft YaHei", 10F, FontStyle.Bold), Location = new Point(20, 20), Size = new Size(250, 20) };
            cbMirrorPresets = new ComboBox { Location = new Point(20, 45), Size = new Size(200, 28), DropDownStyle = ComboBoxStyle.DropDownList };
            btnApplyMirror = CreateThemeButton("一键应用此镜像", 240, 44, 150, 28, true);
            btnApplyMirror.Click += btnApplyMirror_Click;

            controlGroup.Controls.Add(lblSelectPreset);
            controlGroup.Controls.Add(cbMirrorPresets);
            controlGroup.Controls.Add(btnApplyMirror);

            var lblDetails = new Label
            {
                Text = "说明：NPM 镜像配置会同时应用到 npm、yarn 和 pnpm。Go 镜像会写入 go env 中的 GOPROXY。Pip 镜像写入 pip.ini。Maven 镜像写入 settings.xml。Rust 镜像写入您用户目录下的 .cargo/config.toml。",
                Location = new Point(20, 95),
                Size = new Size(890, 80),
                ForeColor = colorTextMuted
            };
            controlGroup.Controls.Add(lblDetails);
        }

        private void InitializeEnvPanel()
        {
            var lblTitle = new Label { Text = "集成环境健康体检", Font = new Font("Microsoft YaHei", 15F, FontStyle.Bold), Location = new Point(25, 20), Size = new Size(400, 35) };
            var lblSubtitle = new Label { Text = "一键扫描系统 PATH 冲突与重复死链，以及检测和卸载系统自带的冲突开发语言/环境安装包。", ForeColor = colorTextMuted, Location = new Point(25, 55), Size = new Size(800, 20) };
            panelEnv.Controls.Add(lblTitle);
            panelEnv.Controls.Add(lblSubtitle);

            var lblEnvTitle = new Label { Text = "当前用户环境变量列表：", Font = new Font("Microsoft YaHei", 10F, FontStyle.Bold), Location = new Point(25, 90), Size = new Size(250, 20) };
            panelEnv.Controls.Add(lblEnvTitle);

            lvEnv = new ListView { Location = new Point(25, 115), Size = new Size(300, 360), View = View.Details, FullRowSelect = true, BorderStyle = BorderStyle.FixedSingle, HeaderStyle = ColumnHeaderStyle.Nonclickable };
            lvEnv.Columns.Add("变量名", 90);
            lvEnv.Columns.Add("对应数值", 200);
            panelEnv.Controls.Add(lvEnv);

            var lblDiagTitle = new Label { Text = "环境健康体检与清理诊断结果：", Font = new Font("Microsoft YaHei", 10F, FontStyle.Bold), Location = new Point(345, 90), Size = new Size(350, 20) };
            panelEnv.Controls.Add(lblDiagTitle);

            lvDiagnostics = new ListView { Location = new Point(345, 115), Size = new Size(610, 240), View = View.Details, FullRowSelect = true, BorderStyle = BorderStyle.FixedSingle, HeaderStyle = ColumnHeaderStyle.Nonclickable };
            lvDiagnostics.Columns.Add("体检大类", 90);
            lvDiagnostics.Columns.Add("体检诊断与建议", 300);
            lvDiagnostics.Columns.Add("受影响路径/卸载命令行", 200);
            lvDiagnostics.SelectedIndexChanged += lvDiagnostics_SelectedIndexChanged;
            panelEnv.Controls.Add(lvDiagnostics);

            GroupBox grpDiagDetail = new GroupBox
            {
                Text = "诊断详细信息与修复建议",
                Location = new Point(345, 360),
                Size = new Size(610, 115),
                ForeColor = colorTextMain
            };
            txtDiagDetail = new TextBox
            {
                Location = new Point(10, 20),
                Size = new Size(590, 85),
                Multiline = true,
                ReadOnly = true,
                ScrollBars = ScrollBars.Vertical,
                BorderStyle = BorderStyle.None,
                BackColor = colorControlBg,
                ForeColor = colorTextMain
            };
            grpDiagDetail.Controls.Add(txtDiagDetail);
            panelEnv.Controls.Add(grpDiagDetail);


            btnScanEnv = CreateThemeButton("一键扫描体检", 345, 490, 150, 40, false);
            btnFixSelected = CreateThemeButton("一键修复/卸载选中项", 510, 490, 250, 40, true);
            btnOptimizePath = CreateThemeButton("一键清理环境变量", 775, 490, 180, 40, false);

            btnScanEnv.Click += (s, e) => RunHealthCheck();
            btnFixSelected.Click += btnFixSelected_Click;
            btnOptimizePath.Click += btnOptimizePath_Click;

            btnFixSelected.Enabled = false;
            panelEnv.Controls.Add(btnScanEnv);
            panelEnv.Controls.Add(btnFixSelected);
            panelEnv.Controls.Add(btnOptimizePath);
            
            var lblDiagTip = new Label
            {
                Text = "“一键健康体检”可自动识别系统 PATH 中的健康问题，以及已安装的可能会影响 Any-Version 切换的冲突软件（如 Node.js、Java 安装版等），并提供一键安全修复或彻底卸载。",
                Location = new Point(25, 540),
                Size = new Size(930, 25),
                ForeColor = colorTextMuted,
                Font = new Font("Microsoft YaHei", 8.5F, FontStyle.Italic)
            };
            panelEnv.Controls.Add(lblDiagTip);
        }

        private void InitializeSystemToolsPanel()
        {
            var lblTitle = new Label { Text = "系统实用工具", Font = new Font("Microsoft YaHei", 15F, FontStyle.Bold), Location = new Point(25, 20), Size = new Size(300, 35) };
            var lblSubtitle = new Label { Text = "快速诊断本地开发端口占用、强制释放占用，以及快捷编辑 Hosts 文件。", ForeColor = colorTextMuted, Location = new Point(25, 55), Size = new Size(500, 20) };
            panelSystemTools.Controls.Add(lblTitle);
            panelSystemTools.Controls.Add(lblSubtitle);

            // 左：端口扫描器
            var portGroup = new Panel { Location = new Point(25, 90), Size = new Size(380, 460), BackColor = colorPanelBg };
            panelSystemTools.Controls.Add(portGroup);

            var lblPortTitle = new Label { Text = "端口占用检测与一键释放", Font = new Font("Microsoft YaHei", 11F, FontStyle.Bold), Location = new Point(15, 15), Size = new Size(300, 25) };
            var lblPortPrompt = new Label { Text = "输入 TCP 端口号 (例如 8080 或 3306):", ForeColor = colorTextMuted, Location = new Point(15, 50), Size = new Size(350, 20) };
            txtPortToCheck = new TextBox { Location = new Point(15, 75), Size = new Size(200, 28), BorderStyle = BorderStyle.FixedSingle };
            btnCheckPort = CreateThemeButton("检测端口", 230, 74, 130, 26, true);
            lblPortStatus = new Label { Text = "状态: 请输入端口号并点击“检测端口”。", ForeColor = colorTextMuted, Location = new Point(15, 120), Size = new Size(350, 60) };
            btnKillPortProcess = CreateThemeButton("一键强杀进程 (释放端口)", 15, 190, 350, 36, false, true);

            btnCheckPort.Click += btnCheckPort_Click;
            btnKillPortProcess.Click += btnKillPortProcess_Click;
            btnKillPortProcess.Enabled = false;

            portGroup.Controls.Add(lblPortTitle);
            portGroup.Controls.Add(lblPortPrompt);
            portGroup.Controls.Add(txtPortToCheck);
            portGroup.Controls.Add(btnCheckPort);
            portGroup.Controls.Add(lblPortStatus);
            portGroup.Controls.Add(btnKillPortProcess);

            // 右：Hosts 编辑器
            var hostsGroup = new Panel { Location = new Point(430, 90), Size = new Size(525, 460), BackColor = colorPanelBg };
            panelSystemTools.Controls.Add(hostsGroup);

            var lblHostsTitle = new Label { Text = "系统 Hosts 文件管理", Font = new Font("Microsoft YaHei", 11F, FontStyle.Bold), Location = new Point(15, 15), Size = new Size(250, 25) };

            tabHosts = new TabControl { Location = new Point(15, 45), Size = new Size(495, 350) };
            tabPageVisual = new TabPage { Text = "可视列表管理" };
            tabPageRaw = new TabPage { Text = "源文件编辑" };
            tabHosts.TabPages.Add(tabPageVisual);
            tabHosts.TabPages.Add(tabPageRaw);

            // tabPageVisual
            lvHostsVisual = new ListView { Location = new Point(5, 5), Size = new Size(477, 200), View = View.Details, FullRowSelect = true, BorderStyle = BorderStyle.FixedSingle, HeaderStyle = ColumnHeaderStyle.Nonclickable };
            lvHostsVisual.Columns.Add("状态", 80);
            lvHostsVisual.Columns.Add("IP地址", 150);
            lvHostsVisual.Columns.Add("主机名/域名", 230);
            tabPageVisual.Controls.Add(lvHostsVisual);

            var lblNewIp = new Label { Text = "IP:", ForeColor = colorTextMuted, Location = new Point(5, 215), Size = new Size(30, 20) };
            txtNewHostIp = new TextBox { Location = new Point(35, 212), Size = new Size(120, 25), BorderStyle = BorderStyle.FixedSingle };
            var lblNewHost = new Label { Text = "域名:", ForeColor = colorTextMuted, Location = new Point(165, 215), Size = new Size(35, 20) };
            txtNewHostName = new TextBox { Location = new Point(205, 212), Size = new Size(277, 25), BorderStyle = BorderStyle.FixedSingle };
            tabPageVisual.Controls.Add(lblNewIp);
            tabPageVisual.Controls.Add(txtNewHostIp);
            tabPageVisual.Controls.Add(lblNewHost);
            tabPageVisual.Controls.Add(txtNewHostName);

            btnNewHostAdd = CreateThemeButton("添加新域名映射", 5, 245, 477, 28, true);
            btnNewHostAdd.Click += btnNewHostAdd_Click;
            tabPageVisual.Controls.Add(btnNewHostAdd);

            btnHostToggle = CreateThemeButton("启用/禁用选中映射", 5, 280, 230, 28, false);
            btnHostToggle.Click += btnHostToggle_Click;
            tabPageVisual.Controls.Add(btnHostToggle);

            btnHostDelete = CreateThemeButton("删除选中映射", 252, 280, 230, 28, false, true);
            btnHostDelete.Click += btnHostDelete_Click;
            tabPageVisual.Controls.Add(btnHostDelete);

            // tabPageRaw
            txtHostsContent = new TextBox { Location = new Point(5, 5), Size = new Size(477, 310), Multiline = true, ScrollBars = ScrollBars.Vertical, BorderStyle = BorderStyle.FixedSingle };
            tabPageRaw.Controls.Add(txtHostsContent);

            btnSaveHosts = CreateThemeButton("保存 Hosts 修改", 15, 410, 230, 36, true);
            btnApplyHostsTemplate = CreateThemeButton("应用 GitHub 下载加速模板", 260, 410, 250, 36, false);

            btnSaveHosts.Click += btnSaveHosts_Click;
            btnApplyHostsTemplate.Click += btnApplyHostsTemplate_Click;

            hostsGroup.Controls.Add(lblHostsTitle);
            hostsGroup.Controls.Add(tabHosts);
            hostsGroup.Controls.Add(btnSaveHosts);
            hostsGroup.Controls.Add(btnApplyHostsTemplate);
        }

        private void InitializeSettingsPanel()
        {
            var lblTitle = new Label { Text = "全局目录设置", Font = new Font("Microsoft YaHei", 15F, FontStyle.Bold), Location = new Point(25, 20), Size = new Size(300, 35) };
            var lblSubtitle = new Label { Text = "自定义 Any-Version 存放安装的 SDK 真实版本以及生成虚拟链接的全局目录。", ForeColor = colorTextMuted, Location = new Point(25, 55), Size = new Size(500, 20) };
            panelSettings.Controls.Add(lblTitle);
            panelSettings.Controls.Add(lblSubtitle);

            var configPanel = new Panel { Location = new Point(25, 90), Size = new Size(930, 460), BackColor = colorPanelBg };
            panelSettings.Controls.Add(configPanel);

            var lblVDir = new Label { Text = "SDK 版本存放安装路径 (Versions)：", Font = new Font("Microsoft YaHei", 10F, FontStyle.Bold), Location = new Point(25, 25), Size = new Size(300, 20) };
            txtVersionsDir = new TextBox { Location = new Point(25, 50), Size = new Size(730, 28), BorderStyle = BorderStyle.FixedSingle };
            var btnBrowseVDir = CreateThemeButton("浏览目录...", 770, 49, 130, 25, false);
            btnBrowseVDir.Click += (s, e) => BrowseFolder(txtVersionsDir);

            configPanel.Controls.Add(lblVDir);
            configPanel.Controls.Add(txtVersionsDir);
            configPanel.Controls.Add(btnBrowseVDir);

            var lblLDir = new Label { Text = "SDK 虚拟链接生成路径 (Links)：", Font = new Font("Microsoft YaHei", 10F, FontStyle.Bold), Location = new Point(25, 95), Size = new Size(300, 20) };
            txtLinksDir = new TextBox { Location = new Point(25, 120), Size = new Size(730, 28), BorderStyle = BorderStyle.FixedSingle };
            var btnBrowseLDir = CreateThemeButton("浏览目录...", 770, 119, 130, 25, false);
            btnBrowseLDir.Click += (s, e) => BrowseFolder(txtLinksDir);

            configPanel.Controls.Add(lblLDir);
            configPanel.Controls.Add(txtLinksDir);
            configPanel.Controls.Add(btnBrowseLDir);

            btnSaveSettings = CreateThemeButton("保存设置", 25, 175, 150, 36, true);
            btnSaveSettings.Click += btnSaveSettings_Click;
            configPanel.Controls.Add(btnSaveSettings);
        }

        private Button CreateThemeButton(string text, int left, int top, int width, int height, bool isPrimary = false, bool isDanger = false)
        {
            var btn = new Button
            {
                Text = text,
                Location = new Point(left, top),
                Size = new Size(width, height),
                FlatStyle = FlatStyle.Flat,
                Cursor = Cursors.Hand,
                Font = new Font("Microsoft YaHei", 9F, FontStyle.Bold)
            };
            btn.FlatAppearance.BorderSize = 1;
            
            if (isPrimary)
            {
                btn.BackColor = colorAccent;
                btn.ForeColor = Color.White;
                btn.FlatAppearance.BorderColor = colorAccent;
            }
            else if (isDanger)
            {
                btn.BackColor = Color.FromArgb(40, 20, 20);
                btn.ForeColor = colorDanger;
                btn.FlatAppearance.BorderColor = Color.FromArgb(80, 40, 40);
            }
            else
            {
                btn.BackColor = Color.FromArgb(45, 45, 55);
                btn.ForeColor = colorTextMain;
                btn.FlatAppearance.BorderColor = Color.FromArgb(70, 70, 85);
            }

            return btn;
        }

        private void StyleForm()
        {
            sidebar.BackColor = colorSidebarBg;
            contentPanel.BackColor = colorBg;

            StyleControl(tvSdks);
            StyleControl(lbInstalledVersions);
            StyleControl(cbRemoteVersions);
            StyleControl(lvServices);
            StyleControl(lvCaches);
            StyleControl(lvMirrors);
            StyleControl(cbMirrorPresets);
            StyleControl(lvEnv);
            StyleControl(lvDiagnostics);
            StyleControl(txtDiagDetail);
            StyleControl(cbPkgSdks);
            StyleControl(lvPkgs);
            StyleControl(txtPortToCheck);
            StyleControl(txtHostsContent);
            StyleControl(tabHosts);
            StyleControl(tabPageVisual);
            StyleControl(tabPageRaw);
            StyleControl(lvHostsVisual);
            StyleControl(txtNewHostIp);
            StyleControl(txtNewHostName);
            StyleControl(txtVersionsDir);
            StyleControl(txtLinksDir);
        }

        private void StyleControl(Control ctrl)
        {
            if (ctrl == null) return;
            ctrl.BackColor = colorControlBg;
            ctrl.ForeColor = colorTextMain;
            
            if (ctrl is ListBox || ctrl is TextBox || ctrl is ComboBox || ctrl is TreeView)
            {
                ctrl.Font = new Font("Microsoft YaHei", 9F, FontStyle.Regular);
            }
            if (ctrl is ListView)
            {
                ListView lv = (ListView)ctrl;
                lv.BackColor = colorControlBg;
                lv.ForeColor = colorTextMain;
                lv.HeaderStyle = ColumnHeaderStyle.Clickable;
                lv.Font = new Font("Microsoft YaHei", 9F, FontStyle.Regular);
            }
        }

        private void ShowPanel(Panel targetPanel, Button navBtn)
        {
            panelSdks.Visible = false;
            panelServices.Visible = false;
            panelCaches.Visible = false;
            panelMirrors.Visible = false;
            panelEnv.Visible = false;
            panelPkgs.Visible = false;
            panelSystemTools.Visible = false;
            panelSettings.Visible = false;

            targetPanel.Visible = true;

            btnSdks.BackColor = Color.Transparent;
            btnSdks.ForeColor = colorTextMuted;
            btnServices.BackColor = Color.Transparent;
            btnServices.ForeColor = colorTextMuted;
            btnCaches.BackColor = Color.Transparent;
            btnCaches.ForeColor = colorTextMuted;
            btnMirrors.BackColor = Color.Transparent;
            btnMirrors.ForeColor = colorTextMuted;
            btnEnv.BackColor = Color.Transparent;
            btnEnv.ForeColor = colorTextMuted;
            btnPkgs.BackColor = Color.Transparent;
            btnPkgs.ForeColor = colorTextMuted;
            btnSystemTools.BackColor = Color.Transparent;
            btnSystemTools.ForeColor = colorTextMuted;
            btnSettings.BackColor = Color.Transparent;
            btnSettings.ForeColor = colorTextMuted;

            navBtn.BackColor = colorAccent;
            navBtn.ForeColor = Color.White;
        }

        // --- CLI 通信封装辅助函数 ---
        private string RunCliCommand(string arguments)
        {
            return RunCliCommandWithStdin(arguments, null);
        }

        private string RunCliCommandWithStdin(string arguments, string stdinContent)
        {
            try
            {
                var startInfo = new ProcessStartInfo
                {
                    FileName = "av.exe",
                    Arguments = arguments,
                    RedirectStandardOutput = true,
                    RedirectStandardError = true,
                    RedirectStandardInput = stdinContent != null,
                    StandardOutputEncoding = Encoding.UTF8,
                    StandardErrorEncoding = Encoding.UTF8,
                    UseShellExecute = false,
                    CreateNoWindow = true
                };

                using (var process = Process.Start(startInfo))
                {
                    if (stdinContent != null)
                    {
                        using (var writer = process.StandardInput)
                        {
                            writer.Write(stdinContent);
                        }
                    }

                    string output = process.StandardOutput.ReadToEnd();
                    string error = process.StandardError.ReadToEnd();
                    process.WaitForExit();
                    if (process.ExitCode != 0)
                    {
                        return "ERROR: " + error + "\n" + output;
                    }
                    return output.Trim();
                }
            }
            catch (Exception ex)
            {
                return "ERROR: " + ex.Message;
            }
        }

        // --- 窗口交互动作操作 ---
        private void LoadConfigData()
        {
            string output = RunCliCommand("config get");
            if (output.StartsWith("ERROR")) return;

            string[] lines = output.Split('\n');
            foreach (var line in lines)
            {
                var parts = line.Split('|');
                if (parts.Length == 2)
                {
                    if (parts[0] == "versions_dir") txtVersionsDir.Text = parts[1].Trim();
                    if (parts[0] == "links_dir") txtLinksDir.Text = parts[1].Trim();
                }
            }
        }

        private void btnSaveSettings_Click(object sender, EventArgs e)
        {
            this.Cursor = Cursors.WaitCursor;
            try
            {
                string output = RunCliCommand(string.Format("config set \"{0}\" \"{1}\"", txtVersionsDir.Text, txtLinksDir.Text));
                if (output.Contains("SUCCESS"))
                {
                    MessageBox.Show("全局目录配置成功保存！", "成功", MessageBoxButtons.OK, MessageBoxIcon.Information);
                }
                else
                {
                    MessageBox.Show("保存设置失败：" + output, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                }
            }
            finally
            {
                this.Cursor = Cursors.Default;
            }
        }

        private void BrowseFolder(TextBox target)
        {
            using (var dialog = new FolderBrowserDialog())
            {
                if (dialog.ShowDialog() == DialogResult.OK)
                {
                    target.Text = dialog.SelectedPath;
                }
            }
        }

        private void LoadSdkList()
        {
            string output = RunCliCommand("sdk list");
            if (output.StartsWith("ERROR"))
            {
                MessageBox.Show("无法从引擎加载 SDK 状态。请确保已经运行过 av init 并且 av.exe 在同目录下。", "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                return;
            }

            string prevSelectedName = tvSdks.SelectedNode != null ? tvSdks.SelectedNode.Text : "";
            tvSdks.Nodes.Clear();

            TreeNode nodeLanguages = new TreeNode("开发语言") { Name = "language" };
            TreeNode nodeServices = new TreeNode("本地服务") { Name = "service" };
            TreeNode nodeBuildTools = new TreeNode("构建工具") { Name = "build_tool" };

            nodeLanguages.NodeFont = new Font(tvSdks.Font, FontStyle.Bold);
            nodeServices.NodeFont = new Font(tvSdks.Font, FontStyle.Bold);
            nodeBuildTools.NodeFont = new Font(tvSdks.Font, FontStyle.Bold);

            string[] lines = output.Split('\n');
            foreach (var line in lines)
            {
                var parts = line.Split('|');
                if (parts.Length >= 2 && !string.IsNullOrEmpty(parts[0]))
                {
                    string name = parts[0].Trim();
                    string category = parts[1].Trim();
                    
                    TreeNode subNode = new TreeNode(name);
                    if (category == "language")
                    {
                        nodeLanguages.Nodes.Add(subNode);
                    }
                    else if (category == "service")
                    {
                        nodeServices.Nodes.Add(subNode);
                    }
                    else if (category == "build_tool")
                    {
                        nodeBuildTools.Nodes.Add(subNode);
                    }
                }
            }

            tvSdks.Nodes.Add(nodeLanguages);
            tvSdks.Nodes.Add(nodeServices);
            tvSdks.Nodes.Add(nodeBuildTools);

            tvSdks.ExpandAll();

            if (!string.IsNullOrEmpty(prevSelectedName))
            {
                TreeNode foundNode = FindNodeByName(tvSdks.Nodes, prevSelectedName);
                if (foundNode != null)
                {
                    tvSdks.SelectedNode = foundNode;
                    return;
                }
            }

            if (nodeLanguages.Nodes.Count > 0)
            {
                tvSdks.SelectedNode = nodeLanguages.Nodes[0];
            }
        }

        private TreeNode FindNodeByName(TreeNodeCollection nodes, string name)
        {
            foreach (TreeNode node in nodes)
            {
                if (node.Text.Equals(name, StringComparison.OrdinalIgnoreCase)) return node;
                TreeNode child = FindNodeByName(node.Nodes, name);
                if (child != null) return child;
            }
            return null;
        }

        private void LoadSdkDetails()
        {
            if (tvSdks.SelectedNode == null || tvSdks.SelectedNode.Parent == null)
            {
                lblSdkName.Text = "选择一个 SDK/服务";
                lblActiveVer.Text = "-";
                lblLinkPath.Text = "-";
                lblTargetPath.Text = "-";
                lbInstalledVersions.Items.Clear();
                cbRemoteVersions.Items.Clear();
                cbRemoteVersions.Enabled = false;
                btnInstall.Enabled = false;
                btnSetActive.Enabled = false;
                btnUninstall.Enabled = false;
                btnAddFolder.Enabled = false;
                return;
            }

            btnSetActive.Enabled = true;
            btnUninstall.Enabled = true;
            btnAddFolder.Enabled = true;

            string sdkName = tvSdks.SelectedNode.Text;
            lblSdkName.Text = sdkName.ToUpper();

            string output = RunCliCommand("sdk list");
            if (output.StartsWith("ERROR")) return;

            string[] lines = output.Split('\n');
            foreach (var line in lines)
            {
                var parts = line.Split('|');
                if (parts.Length >= 4 && parts[0].Trim() == sdkName)
                {
                    string active = parts[2].Trim();
                    string installedRaw = parts[3].Trim();

                    if (!string.IsNullOrEmpty(active))
                    {
                        lblActiveVer.Text = active;
                        lblActiveVer.ForeColor = colorSuccess;
                    }
                    else
                    {
                        lblActiveVer.Text = "(无)";
                        lblActiveVer.ForeColor = colorTextMuted;
                    }

                    lblLinkPath.Text = Path.Combine(txtLinksDir.Text, sdkName);
                    lblTargetPath.Text = string.IsNullOrEmpty(active) ? "-" : Path.Combine(txtVersionsDir.Text, sdkName, active);

                    lbInstalledVersions.Items.Clear();
                    if (!string.IsNullOrEmpty(installedRaw))
                    {
                        foreach (var v in installedRaw.Split(','))
                        {
                            lbInstalledVersions.Items.Add(v);
                        }
                    }

                    // 异步载入远程版本
                    cbRemoteVersions.Items.Clear();
                    cbRemoteVersions.Items.Add("正在拉取远程版本列表...");
                    cbRemoteVersions.SelectedIndex = 0;
                    cbRemoteVersions.Enabled = false;
                    btnInstall.Enabled = false;

                    Thread thread = new Thread(() => FetchRemoteList(sdkName));
                    thread.Start();
                    break;
                }
            }
        }

        private void FetchRemoteList(string sdkName)
        {
            string output = RunCliCommand("list-remote " + sdkName);
            this.Invoke((MethodInvoker)delegate
            {
                cbRemoteVersions.Items.Clear();
                if (output.StartsWith("ERROR"))
                {
                    cbRemoteVersions.Items.Add("拉取版本列表失败");
                    cbRemoteVersions.Enabled = false;
                    btnInstall.Enabled = false;
                    return;
                }

                string[] lines = output.Split('\n');
                bool started = false;
                foreach (var line in lines)
                {
                    var clean = line.Trim();
                    if (clean.Contains("Available remote versions") || clean.Contains("可用远程版本"))
                    {
                        started = true;
                        continue;
                    }
                    if (started && !string.IsNullOrEmpty(clean))
                    {
                        cbRemoteVersions.Items.Add(clean);
                    }
                }
                if (cbRemoteVersions.Items.Count > 0)
                {
                    cbRemoteVersions.SelectedIndex = 0;
                    cbRemoteVersions.Enabled = true;
                    btnInstall.Enabled = true;
                }
                else
                {
                    cbRemoteVersions.Items.Add("无可用线上版本");
                    cbRemoteVersions.Enabled = false;
                    btnInstall.Enabled = false;
                }
            });
        }

        private void btnSetActive_Click(object sender, EventArgs e)
        {
            if (tvSdks.SelectedNode == null || tvSdks.SelectedNode.Parent == null || lbInstalledVersions.SelectedItem == null)
            {
                MessageBox.Show("请在左侧选择一个 SDK，并在下方选择一个已安装的本地版本进行激活。", "选择缺失", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                return;
            }

            string sdkName = tvSdks.SelectedNode.Text;
            string version = lbInstalledVersions.SelectedItem.ToString();

            this.Cursor = Cursors.WaitCursor;
            try
            {
                string output = RunCliCommand(string.Format("use {0} {1}", sdkName, version));
                if (!output.StartsWith("ERROR"))
                {
                    MessageBox.Show(string.Format("成功激活 {0} 版本为 {1}！", sdkName.ToUpper(), version), "切换成功", MessageBoxButtons.OK, MessageBoxIcon.Information);
                    LoadSdkDetails();
                }
                else
                {
                    MessageBox.Show("激活失败：" + output, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                }
            }
            finally
            {
                this.Cursor = Cursors.Default;
            }
        }

        private void btnUninstall_Click(object sender, EventArgs e)
        {
            if (tvSdks.SelectedNode == null || tvSdks.SelectedNode.Parent == null || lbInstalledVersions.SelectedItem == null)
            {
                MessageBox.Show("请选择一个已安装的版本进行卸载。", "选择缺失", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                return;
            }

            string sdkName = tvSdks.SelectedNode.Text;
            string version = lbInstalledVersions.SelectedItem.ToString();

            var result = MessageBox.Show(string.Format("您确定要彻底删除并卸载 {0} 版本 {1} 吗？", sdkName.ToUpper(), version), "确认卸载", MessageBoxButtons.YesNo, MessageBoxIcon.Warning);
            if (result == DialogResult.Yes)
            {
                this.Cursor = Cursors.WaitCursor;
                try
                {
                    string output = RunCliCommand(string.Format("uninstall {0} {1}", sdkName, version));
                    MessageBox.Show(output, "卸载结果", MessageBoxButtons.OK, MessageBoxIcon.Information);
                    LoadSdkDetails();
                }
                finally
                {
                    this.Cursor = Cursors.Default;
                }
            }
        }

        private void btnAddFolder_Click(object sender, EventArgs e)
        {
            if (tvSdks.SelectedNode == null || tvSdks.SelectedNode.Parent == null) return;
            string sdkName = tvSdks.SelectedNode.Text;

            using (var dialog = new FolderBrowserDialog())
            {
                dialog.Description = "选择要导入的本地 " + sdkName.ToUpper() + " 文件夹目录：";
                if (dialog.ShowDialog() == DialogResult.OK)
                {
                    string folderPath = dialog.SelectedPath;
                    string version = PromptDialog.Show("输入导入版本号", "请输入此文件夹对应的版本号 (例如 1.2.3):");
                    if (string.IsNullOrEmpty(version)) return;

                    string output = RunCliCommand(string.Format("add {0} {1} \"{2}\"", sdkName, version, folderPath));
                    MessageBox.Show(output, "导入结果", MessageBoxButtons.OK, MessageBoxIcon.Information);
                    LoadSdkDetails();
                }
            }
        }

        private void btnInstall_Click(object sender, EventArgs e)
        {
            if (tvSdks.SelectedNode == null || tvSdks.SelectedNode.Parent == null || cbRemoteVersions.SelectedItem == null) return;
            string sdkName = tvSdks.SelectedNode.Text;
            string rawVersion = cbRemoteVersions.SelectedItem.ToString();

            string version = rawVersion.Split(' ')[0].Trim();

            btnInstall.Enabled = false;
            prgInstall.Value = 0;
            prgInstall.Visible = true;
            lblProgressStatus.Text = "正在下载版本 " + version + "...";
            lblProgressStatus.Visible = true;

            Thread thread = new Thread(() =>
            {
                try
                {
                    var startInfo = new ProcessStartInfo
                    {
                        FileName = "av.exe",
                        Arguments = string.Format("install {0} {1}", sdkName, version),
                        RedirectStandardOutput = true,
                        RedirectStandardError = true,
                        StandardOutputEncoding = Encoding.UTF8,
                        StandardErrorEncoding = Encoding.UTF8,
                        UseShellExecute = false,
                        CreateNoWindow = true
                    };

                    using (var process = Process.Start(startInfo))
                    {
                        var reader = process.StandardOutput;
                        var buffer = new char[1024];
                        int count;
                        var sb = new StringBuilder();

                        while ((count = reader.Read(buffer, 0, buffer.Length)) > 0)
                        {
                            for (int i = 0; i < count; i++)
                            {
                                char c = buffer[i];
                                if (c == '\r' || c == '\n')
                                {
                                    string line = sb.ToString().Trim();
                                    sb.Clear();
                                    if (line.Contains("Downloading...") || line.Contains("正在下载..."))
                                    {
                                        int start = line.LastIndexOf('(');
                                        int end = line.LastIndexOf('%');
                                        if (start != -1 && end != -1 && end > start)
                                        {
                                            string pctStr = line.Substring(start + 1, end - start - 1);
                                            int pct;
                                            if (int.TryParse(pctStr, out pct))
                                            {
                                                this.Invoke((MethodInvoker)delegate
                                                {
                                                    prgInstall.Value = pct;
                                                    lblProgressStatus.Text = line.Replace("Downloading...", "正在下载...");
                                                });
                                            }
                                        }
                                    }
                                }
                                else
                                {
                                    sb.Append(c);
                                }
                            }
                        }

                        process.WaitForExit();
                        
                        this.Invoke((MethodInvoker)delegate
                        {
                            btnInstall.Enabled = true;
                            prgInstall.Visible = false;
                            lblProgressStatus.Visible = false;

                            if (process.ExitCode == 0)
                            {
                                MessageBox.Show(string.Format("成功下载并安装 {0} v{1}！", sdkName.ToUpper(), version), "安装成功", MessageBoxButtons.OK, MessageBoxIcon.Information);
                            }
                            else
                            {
                                string err = process.StandardError.ReadToEnd();
                                MessageBox.Show("安装失败：\n" + err, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                            }
                            LoadSdkDetails();
                        });
                    }
                }
                catch (Exception ex)
                {
                    this.Invoke((MethodInvoker)delegate
                    {
                        btnInstall.Enabled = true;
                        prgInstall.Visible = false;
                        lblProgressStatus.Visible = false;
                        MessageBox.Show("运行安装程序出错：" + ex.Message, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                    });
                }
            });
            thread.Start();
        }

        // --- 本地服务管理相关动作 ---
        private void LoadServicesList(bool quiet)
        {
            string output = RunCliCommand("service list");
            if (output.StartsWith("ERROR")) return;

            string selectedName = "";
            if (lvServices.SelectedItems.Count > 0)
            {
                selectedName = lvServices.SelectedItems[0].Text;
            }

            lvServices.Items.Clear();
            string[] lines = output.Split('\n');
            foreach (var line in lines)
            {
                var parts = line.Split('|');
                if (parts.Length >= 5)
                {
                    var item = new ListViewItem(parts[0].Trim().ToUpper());
                    string status = parts[1].Trim();
                    if (status == "running")
                    {
                        item.SubItems.Add("运行中");
                        item.ForeColor = colorSuccess;
                    }
                    else if (status == "not_installed")
                    {
                        item.SubItems.Add("未安装");
                        item.ForeColor = colorTextMuted;
                    }
                    else
                    {
                        item.SubItems.Add("已停止");
                        item.ForeColor = colorDanger;
                    }
                    item.SubItems.Add(string.IsNullOrEmpty(parts[2].Trim()) ? "(无)" : parts[2].Trim());
                    item.SubItems.Add(parts[3].Trim());
                    item.SubItems.Add(parts[4].Trim() == "0" ? "-" : parts[4].Trim());

                    lvServices.Items.Add(item);

                    if (item.Text.ToLower() == selectedName.ToLower())
                    {
                        item.Selected = true;
                    }
                }
            }

            if (!quiet && lvServices.SelectedItems.Count == 0 && lvServices.Items.Count > 0)
            {
                lvServices.Items[0].Selected = true;
            }
        }

        private void lvServices_SelectedIndexChanged(object sender, EventArgs e)
        {
            if (lvServices.SelectedItems.Count == 0)
            {
                btnStartService.Enabled = false;
                btnStopService.Enabled = false;
                btnEditServiceConfig.Enabled = false;
                btnOpenServiceDir.Enabled = false;
                return;
            }

            var item = lvServices.SelectedItems[0];
            string status = item.SubItems[1].Text;
            string activeVersion = item.SubItems[2].Text;

            btnStartService.Enabled = (status == "已停止" && activeVersion != "(无)");
            btnStopService.Enabled = (status == "运行中");
            btnEditServiceConfig.Enabled = (activeVersion != "(无)");
            btnOpenServiceDir.Enabled = (activeVersion != "(无)");
        }

        private void btnStartService_Click(object sender, EventArgs e)
        {
            if (lvServices.SelectedItems.Count == 0) return;
            var item = lvServices.SelectedItems[0];
            string name = item.Text.ToLower();
            string version = item.SubItems[2].Text;

            this.Cursor = Cursors.WaitCursor;
            try
            {
                string output = RunCliCommand(string.Format("service start {0} {1}", name, version));
                if (output.Contains("SUCCESS"))
                {
                    MessageBox.Show(string.Format("已成功启动本地服务：{0}！", item.Text), "启动成功", MessageBoxButtons.OK, MessageBoxIcon.Information);
                    LoadServicesList(false);
                }
                else
                {
                    MessageBox.Show("启动服务失败：\n" + output, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                }
            }
            finally
            {
                this.Cursor = Cursors.Default;
            }
        }

        private void btnStopService_Click(object sender, EventArgs e)
        {
            if (lvServices.SelectedItems.Count == 0) return;
            var item = lvServices.SelectedItems[0];
            string name = item.Text.ToLower();

            this.Cursor = Cursors.WaitCursor;
            try
            {
                string output = RunCliCommand(string.Format("service stop {0}", name));
                if (output.Contains("SUCCESS"))
                {
                    MessageBox.Show(string.Format("已成功停止本地服务：{0}！", item.Text), "停止成功", MessageBoxButtons.OK, MessageBoxIcon.Information);
                    LoadServicesList(false);
                }
                else
                {
                    MessageBox.Show("停止服务失败：\n" + output, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                }
            }
            finally
            {
                this.Cursor = Cursors.Default;
            }
        }

        private void btnEditServiceConfig_Click(object sender, EventArgs e)
        {
            if (lvServices.SelectedItems.Count == 0) return;
            var item = lvServices.SelectedItems[0];
            string name = item.Text.ToLower();
            string version = item.SubItems[2].Text;

            string activeDir = Path.Combine(txtVersionsDir.Text, name, version);
            string confFile = "";

            if (name == "mysql") confFile = Path.Combine(activeDir, "my.ini");
            else if (name == "redis") confFile = Path.Combine(activeDir, "redis.windows.conf");
            else if (name == "nginx") confFile = Path.Combine(activeDir, "conf", "nginx.conf");

            if (!string.IsNullOrEmpty(confFile) && File.Exists(confFile))
            {
                Process.Start("notepad.exe", string.Format("\"{0}\"", confFile));
            }
            else
            {
                MessageBox.Show("未找到配置文件，路径：" + confFile, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
            }
        }

        private void btnOpenServiceDir_Click(object sender, EventArgs e)
        {
            if (lvServices.SelectedItems.Count == 0) return;
            var item = lvServices.SelectedItems[0];
            string name = item.Text.ToLower();
            string version = item.SubItems[2].Text;

            string activeDir = Path.Combine(txtVersionsDir.Text, name, version);
            if (Directory.Exists(activeDir))
            {
                Process.Start("explorer.exe", string.Format("\"{0}\"", activeDir));
            }
        }

        // --- 包管理器缓存清理相关动作 ---
        private void LoadCachesList()
        {
            lvCaches.Items.Clear();
            btnEditCache.Enabled = false;

            lblCacheLoading.Bounds = lvCaches.Bounds;
            lblCacheLoading.Visible = true;
            lblCacheLoading.BringToFront();

            Thread thread = new Thread(() =>
            {
                string output = RunCliCommand("cache list");
                this.Invoke((MethodInvoker)delegate
                {
                    lblCacheLoading.Visible = false;
                    if (output.StartsWith("ERROR"))
                    {
                        MessageBox.Show("获取依赖缓存列表失败：" + output, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                        return;
                    }

                    string[] lines = output.Split('\n');
                    foreach (var line in lines)
                    {
                        var parts = line.Split('|');
                        if (parts.Length >= 4 && !string.IsNullOrEmpty(parts[0].Trim()))
                        {
                            var item = new ListViewItem(parts[0].Trim());
                            item.SubItems.Add(parts[1].Trim() == "true" ? "已安装" : "未检测到");
                            
                            string origPath = parts[2].Trim();
                            string size = parts[3].Trim();
                            bool isLink = parts.Length >= 6 && parts[4].Trim() == "true";
                            string realTarget = parts.Length >= 6 ? parts[5].Trim() : "";

                            if (isLink)
                            {
                                item.SubItems.Add(string.Format("{0} -> {1} [已创建链接]", origPath, realTarget));
                                item.ForeColor = colorSuccess;
                            }
                            else
                            {
                                item.SubItems.Add(origPath);
                            }
                            item.SubItems.Add(size);
                            lvCaches.Items.Add(item);
                        }
                    }
                });
            });
            thread.Start();
        }

        private void btnEditCache_Click(object sender, EventArgs e)
        {
            if (lvCaches.SelectedItems.Count == 0) return;
            var item = lvCaches.SelectedItems[0];
            string toolName = item.Text;

            using (var dialog = new FolderBrowserDialog())
            {
                dialog.Description = "为 " + toolName.ToUpper() + " 选择并迁移到新的自定义缓存路径：";
                if (dialog.ShowDialog() == DialogResult.OK)
                {
                    string newPath = dialog.SelectedPath;
                    this.Cursor = Cursors.WaitCursor;
                    try
                    {
                        string output = RunCliCommand(string.Format("cache set {0} \"{1}\"", toolName, newPath));
                        if (output.Contains("SUCCESS"))
                        {
                            MessageBox.Show(string.Format("{0} 的缓存路径已成功重定向并迁移到 {1}！", toolName.ToUpper(), newPath), "成功", MessageBoxButtons.OK, MessageBoxIcon.Information);
                            LoadCachesList();
                        }
                        else
                        {
                            MessageBox.Show("修改缓存路径失败：" + output, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                        }
                    }
                    finally
                    {
                        this.Cursor = Cursors.Default;
                    }
                }
            }
        }

        // --- 国内镜像配置相关动作 ---
        private void LoadMirrorsList()
        {
            string output = RunCliCommand("mirror list");
            if (output.StartsWith("ERROR")) return;

            lvMirrors.Items.Clear();
            string[] lines = output.Split('\n');
            foreach (var line in lines)
            {
                var parts = line.Split('|');
                if (parts.Length >= 3)
                {
                    var item = new ListViewItem(parts[0].Trim().ToUpper());
                    item.SubItems.Add(parts[1].Trim());
                    item.SubItems.Add(parts[2].Trim());
                    lvMirrors.Items.Add(item);
                }
            }
            if (lvMirrors.Items.Count > 0)
            {
                lvMirrors.Items[0].Selected = true;
            }
        }

        private void lvMirrors_SelectedIndexChanged(object sender, EventArgs e)
        {
            if (lvMirrors.SelectedItems.Count == 0)
            {
                cbMirrorPresets.Items.Clear();
                btnApplyMirror.Enabled = false;
                return;
            }

            var item = lvMirrors.SelectedItems[0];
            string tool = item.Text.ToLower();

            cbMirrorPresets.Items.Clear();
            if (tool == "npm")
            {
                cbMirrorPresets.Items.Add("阿里云镜像源 (npmmirror.com)");
                cbMirrorPresets.Items.Add("腾讯云镜像源 (mirrors.cloud.tencent.com)");
                cbMirrorPresets.Items.Add("官方默认源 (registry.npmjs.org)");
            }
            else if (tool == "pip")
            {
                cbMirrorPresets.Items.Add("清华大学源 (pypi.tuna.tsinghua.edu.cn)");
                cbMirrorPresets.Items.Add("阿里云源 (mirrors.aliyun.com)");
                cbMirrorPresets.Items.Add("官方默认源 (pypi.org)");
            }
            else if (tool == "maven")
            {
                cbMirrorPresets.Items.Add("阿里云公共仓库 (maven.aliyun.com)");
                cbMirrorPresets.Items.Add("官方默认源 (repo.maven.apache.org)");
            }
            else if (tool == "go")
            {
                cbMirrorPresets.Items.Add("七牛云 GoProxy.cn (goproxy.cn)");
                cbMirrorPresets.Items.Add("阿里云源 (mirrors.aliyun.com)");
                cbMirrorPresets.Items.Add("官方默认源 (proxy.golang.org)");
            }
            else if (tool == "rust")
            {
                cbMirrorPresets.Items.Add("字节跳动 Rsproxy (rsproxy.cn)");
                cbMirrorPresets.Items.Add("清华大学源 (mirrors.tuna.tsinghua.edu.cn)");
                cbMirrorPresets.Items.Add("官方默认源 (crates.io)");
            }

            if (cbMirrorPresets.Items.Count > 0)
            {
                cbMirrorPresets.SelectedIndex = 0;
                btnApplyMirror.Enabled = true;
            }
        }

        private void btnApplyMirror_Click(object sender, EventArgs e)
        {
            if (lvMirrors.SelectedItems.Count == 0 || cbMirrorPresets.SelectedItem == null) return;

            string tool = lvMirrors.SelectedItems[0].Text.ToLower();
            string selectedPreset = cbMirrorPresets.SelectedItem.ToString();
            
            string presetKey = "official";
            if (selectedPreset.Contains("阿里云") || selectedPreset.Contains("Taobao")) presetKey = "aliyun";
            else if (selectedPreset.Contains("清华大学")) presetKey = "tsinghua";
            else if (selectedPreset.Contains("腾讯云")) presetKey = "tencent";
            else if (selectedPreset.Contains("GoProxy.cn")) presetKey = "goproxy";
            else if (selectedPreset.Contains("Rsproxy")) presetKey = "rsproxy";

            this.Cursor = Cursors.WaitCursor;
            try
            {
                string output = RunCliCommand(string.Format("mirror set {0} {1}", tool, presetKey));
                if (output.Contains("SUCCESS"))
                {
                    MessageBox.Show(string.Format("成功为 {0} 应用国内镜像源！", tool.ToUpper()), "成功", MessageBoxButtons.OK, MessageBoxIcon.Information);
                    LoadMirrorsList();
                }
                else
                {
                    MessageBox.Show("应用镜像源失败：" + output, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                }
            }
            finally
            {
                this.Cursor = Cursors.Default;
            }
        }

        // --- 环境变量诊断与优化相关动作 ---
        private void LoadEnvList()
        {
            lvEnv.Items.Clear();
            string[] keys = new string[] { "PATH", "JAVA_HOME", "GOROOT", "USERPROFILE", "LOCALAPPDATA", "NUGET_PACKAGES", "PIP_CACHE_DIR" };
            foreach (var key in keys)
            {
                string val = Environment.GetEnvironmentVariable(key, EnvironmentVariableTarget.Process);
                if (string.IsNullOrEmpty(val))
                {
                    val = Environment.GetEnvironmentVariable(key, EnvironmentVariableTarget.User);
                }
                if (string.IsNullOrEmpty(val))
                {
                    val = "(未设置)";
                }

                var item = new ListViewItem(key);
                item.SubItems.Add(val);
                lvEnv.Items.Add(item);
            }
        }

        private Dictionary<string, string> checkIdToName = new Dictionary<string, string>
        {
            { "installed_sdks", "SDK 安装状态" },
            { "path_check", "PATH 路径健康" },
            { "dev_env_vars", "开发环境变量" },
            { "external_software", "外部软件冲突" },
        };

        private string GetCheckCategory(string checkId)
        {
            string name;
            if (checkIdToName.TryGetValue(checkId, out name))
                return name;
            return checkId;
        }

        private string GetProblemTypeName(string problemType)
        {
            switch (problemType)
            {
                case "not_installed": return "未安装";
                case "duplicate_path": return "重复路径";
                case "dead_path": return "失效死链";
                case "conflict": return "环境冲突";
                case "priority": return "路径优先级";
                case "read_error": return "读取错误";
                case "system_conflict": return "系统冲突";
                case "dead_env_path": return "无效路径";
                case "wrong_env": return "变量异常";
                case "conflict_software": return "软件冲突";
                case "missing_env": return "缺失变量";
                default: return problemType;
            }
        }

        private void RunHealthCheck()
        {
            this.Cursor = Cursors.WaitCursor;
            try
            {
                LoadEnvList();
                lvDiagnostics.Items.Clear();

                // 从 CLI 运行完整诊断
                string cliOutput = RunCliCommand("env check");
                int issuesCount = 0;
                if (!cliOutput.StartsWith("ERROR"))
                {
                    string currentCheckId = "";
                    string[] lines = cliOutput.Split('\n');
                    foreach (var line in lines)
                    {
                        var parts = line.Split('|');
                        if (parts.Length < 2) continue;

                        string recordType = parts[0].Trim();

                        if (recordType == "CHECK" && parts.Length >= 3)
                        {
                            currentCheckId = parts[1].Trim();
                        }
                        else if (recordType == "PROBLEM" && parts.Length >= 6)
                        {
                            string checkId = parts[1].Trim();
                            string problemType = parts[2].Trim();
                            string problemDesc = parts[3].Trim();
                            string fixType = parts[4].Trim();
                            string fixTarget = parts[5].Trim();

                            string category = GetCheckCategory(checkId);
                            string typeName = GetProblemTypeName(problemType);

                            var item = new ListViewItem(category);
                            item.SubItems.Add(problemDesc);
                            item.SubItems.Add(fixTarget);
                            item.ToolTipText = string.Format("{0} | 修复类型: {1}", problemDesc, fixType);

                            // 存储修复信息
                            item.Tag = new DiagnosticFixInfo
                            {
                                CheckID = checkId,
                                ProblemType = problemType,
                                FixType = fixType,
                                FixTarget = fixTarget
                            };

                            // 颜色标记
                            if (problemType == "duplicate_path") item.ForeColor = Color.Orange;
                            else if (problemType == "dead_path" || problemType == "dead_env_path") item.ForeColor = colorDanger;
                            else if (problemType == "conflict" || problemType == "system_conflict") item.ForeColor = Color.Yellow;
                            else if (problemType == "conflict_software") item.ForeColor = Color.FromArgb(244, 63, 94);
                            else if (problemType == "not_installed") item.ForeColor = Color.FromArgb(100, 150, 255);
                            else if (problemType == "priority") item.ForeColor = Color.Cyan;
                            else if (problemType == "wrong_env") item.ForeColor = Color.Orange;

                            lvDiagnostics.Items.Add(item);
                            issuesCount++;
                        }
                    }
                }

                btnOptimizePath.Enabled = (issuesCount > 0);
                btnFixSelected.Enabled = false;
                btnFixSelected.Text = "一键修复/卸载选中项";
            }
            finally
            {
                this.Cursor = Cursors.Default;
            }
        }

        // 诊断修复信息载体
        public class DiagnosticFixInfo
        {
            public string CheckID;
            public string ProblemType;
            public string FixType;
            public string FixTarget;
        }

        private void ScanExternalSoftware()
        {
            detectedSoftware.Clear();
            var hives = new RegistryHive[] { RegistryHive.LocalMachine, RegistryHive.CurrentUser };
            var views = new RegistryView[] { RegistryView.Registry64, RegistryView.Registry32 };
            var visitedUninstalls = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

            foreach (var hive in hives)
            {
                foreach (var view in views)
                {
                    try
                    {
                        using (var baseKey = RegistryKey.OpenBaseKey(hive, view))
                        using (var uninstallKey = baseKey.OpenSubKey(@"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"))
                        {
                            if (uninstallKey == null) continue;

                            foreach (var subkeyName in uninstallKey.GetSubKeyNames())
                            {
                                try
                                {
                                    using (var subkey = uninstallKey.OpenSubKey(subkeyName))
                                    {
                                        if (subkey == null) continue;

                                        string displayName = subkey.GetValue("DisplayName") as string;
                                        string displayVersion = subkey.GetValue("DisplayVersion") as string;
                                        string uninstallString = subkey.GetValue("UninstallString") as string;
                                        string installLocation = subkey.GetValue("InstallLocation") as string;

                                        if (string.IsNullOrEmpty(displayName) || string.IsNullOrEmpty(uninstallString))
                                            continue;

                                        string dnLower = displayName.ToLower();
                                        bool isMatch = dnLower.Contains("node.js") ||
                                                       dnLower.Contains("openjdk") ||
                                                       dnLower.Contains("adoptium") ||
                                                       dnLower.Contains("zulu") ||
                                                       dnLower.Contains("java(tm)") ||
                                                       dnLower.Contains("jdk") ||
                                                       dnLower.Contains("nvm for windows");

                                        if (isMatch)
                                        {
                                            string cleanUninstall = uninstallString.Trim();
                                            if (visitedUninstalls.Contains(cleanUninstall))
                                                continue;

                                            visitedUninstalls.Add(cleanUninstall);

                                            var sw = new RegistrySoftware
                                            {
                                                DisplayName = displayName,
                                                DisplayVersion = displayVersion ?? "",
                                                UninstallString = cleanUninstall,
                                                InstallLocation = installLocation ?? "",
                                                RegistryKey = string.Format(@"{0}\{1}\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{2}", hive, view, subkeyName)
                                            };
                                            detectedSoftware.Add(sw);
                                        }
                                    }
                                }
                                catch
                                {
                                    // 忽略单个注册表项读取异常
                                }
                            }
                        }
                    }
                    catch
                    {
                        // 忽略整个配置单元/视图读取异常
                    }
                }
            }
        }

        private void UninstallSoftware(RegistrySoftware sw)
        {
            this.Cursor = Cursors.WaitCursor;
            try
            {
                string cmd = sw.UninstallString.Trim();
                string exePath = "";
                string args = "";

                if (cmd.StartsWith("\""))
                {
                    int nextQuote = cmd.IndexOf("\"", 1);
                    if (nextQuote != -1)
                    {
                        exePath = cmd.Substring(1, nextQuote - 1).Trim();
                        args = cmd.Substring(nextQuote + 1).Trim();
                    }
                    else
                    {
                        exePath = cmd.Replace("\"", "").Trim();
                    }
                }
                else
                {
                    int spaceIdx = cmd.IndexOf(' ');
                    if (spaceIdx == -1)
                    {
                        exePath = cmd;
                    }
                    else
                    {
                        string lowerCmd = cmd.ToLower();
                        if (lowerCmd.StartsWith("msiexec"))
                        {
                            exePath = "msiexec.exe";
                            args = cmd.Substring(spaceIdx + 1).Trim();
                        }
                        else
                        {
                            int exeIdx = lowerCmd.IndexOf(".exe");
                            if (exeIdx != -1)
                            {
                                exePath = cmd.Substring(0, exeIdx + 4).Trim();
                                args = cmd.Substring(exeIdx + 4).Trim();
                            }
                            else
                            {
                                exePath = cmd.Substring(0, spaceIdx).Trim();
                                args = cmd.Substring(spaceIdx + 1).Trim();
                            }
                        }
                    }
                }

                if (exePath.ToLower().Contains("msiexec"))
                {
                    string argsLower = args.ToLower();
                    if (argsLower.Contains("/i"))
                    {
                        int iIdx = argsLower.IndexOf("/i");
                        args = args.Substring(0, iIdx) + "/x" + args.Substring(iIdx + 2);
                    }
                    else if (!argsLower.Contains("/x"))
                    {
                        args = "/x " + args;
                    }

                    if (!argsLower.Contains("/q"))
                    {
                        args += " /qb /norestart";
                    }
                }
                else
                {
                    string exeLower = exePath.ToLower();
                    if (exeLower.Contains("unins") && !args.ToLower().Contains("/silent"))
                    {
                        args += " /VERYSILENT /SUPPRESSMSGBOXES /NORESTART";
                    }
                }

                var startInfo = new ProcessStartInfo
                {
                    FileName = exePath,
                    Arguments = args,
                    UseShellExecute = true,
                    Verb = "runas"
                };

                using (var process = Process.Start(startInfo))
                {
                    if (process != null)
                    {
                        process.WaitForExit();
                    }
                }

                MessageBox.Show(string.Format("外部软件 {0} 的卸载程序执行完毕，即将重新扫描环境状态！", sw.DisplayName), "卸载完成", MessageBoxButtons.OK, MessageBoxIcon.Information);
                RunHealthCheck();
            }
            catch (Exception ex)
            {
                MessageBox.Show("启动卸载程序失败：" + ex.Message, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
            }
            finally
            {
                this.Cursor = Cursors.Default;
            }
        }

        private void lvDiagnostics_SelectedIndexChanged(object sender, EventArgs e)
        {
            if (lvDiagnostics.SelectedItems.Count == 0)
            {
                btnFixSelected.Enabled = false;
                btnFixSelected.Text = "一键修复/卸载选中项";
                if (txtDiagDetail != null) txtDiagDetail.Text = "请从上方列表选择一个诊断项以查看详细信息。";
                return;
            }

            var item = lvDiagnostics.SelectedItems[0];
            var fixInfo = item.Tag as DiagnosticFixInfo;
            if (fixInfo != null)
            {
                StringBuilder sb = new StringBuilder();
                sb.AppendLine("【诊断分类】 " + item.Text);
                sb.AppendLine("【诊断说明】 " + item.SubItems[1].Text);
                sb.AppendLine("【影响路径】 " + fixInfo.FixTarget);
                
                string actionDesc = "";
                switch (fixInfo.FixType)
                {
                    case "clean_path":
                        actionDesc = "优化环境变量 (清理死链和重复项，将虚拟链接置顶)";
                        break;
                    case "remove_path":
                        actionDesc = "从 PATH 环境变量中移除该失效路径";
                        break;
                    case "set_env":
                        actionDesc = "修复或设置该环境变量的值";
                        break;
                    case "install":
                        actionDesc = "前往 [SDK 版本管理] 标签页安装此 SDK";
                        break;
                    case "uninstall":
                        actionDesc = "一键运行卸载命令行以彻底移除冲突的外部软件";
                        break;
                    default:
                        actionDesc = fixInfo.FixType;
                        break;
                }
                sb.AppendLine("【修复动作】 " + actionDesc);
                if (txtDiagDetail != null) txtDiagDetail.Text = sb.ToString();

                btnFixSelected.Enabled = true;
                switch (fixInfo.FixType)
                {
                    case "clean_path":
                        btnFixSelected.Text = "优化环境变量";
                        break;
                    case "remove_path":
                        btnFixSelected.Text = "移除该路径";
                        break;
                    case "set_env":
                        btnFixSelected.Text = "修正环境变量";
                        break;
                    case "install":
                        btnFixSelected.Text = "前往安装";
                        break;
                    case "uninstall":
                        btnFixSelected.Text = "一键卸载外部软件";
                        break;
                    default:
                        btnFixSelected.Text = "一键修复/卸载选中项";
                        break;
                }
            }
            else
            {
                btnFixSelected.Enabled = false;
                btnFixSelected.Text = "一键修复/卸载选中项";
                if (txtDiagDetail != null) txtDiagDetail.Text = "该诊断项无关联修复建议。";
            }
        }

        private void btnFixSelected_Click(object sender, EventArgs e)
        {
            if (lvDiagnostics.SelectedItems.Count == 0) return;

            var item = lvDiagnostics.SelectedItems[0];
            var fixInfo = item.Tag as DiagnosticFixInfo;
            if (fixInfo != null)
            {
                if (fixInfo.FixType == "uninstall")
                {
                    // 外部软件卸载：fixTarget 是卸载命令行
                    string uninstallCmd = fixInfo.FixTarget;
                    var result = MessageBox.Show(
                        string.Format("您确定要启动卸载程序以移除该软件吗？\n\n卸载命令行：{0}", uninstallCmd),
                        "确认卸载外部软件", MessageBoxButtons.YesNo, MessageBoxIcon.Warning);
                    if (result == DialogResult.Yes)
                    {
                        UninstallExternalSoftware(uninstallCmd);
                    }
                }
                else if (fixInfo.FixType == "install")
                {
                    // 转到 SDK 管理页面去安装
                    MessageBox.Show(
                        string.Format("请前往 [SDK 版本管理] 标签页选择并安装 {0}。", fixInfo.FixTarget),
                        "提示", MessageBoxButtons.OK, MessageBoxIcon.Information);
                }
                else
                {
                    // 通过 CLI 执行修复
                    this.Cursor = Cursors.WaitCursor;
                    try
                    {
                        string output = RunCliCommand(
                            string.Format("env fix {0} \"{1}\"", fixInfo.FixType, fixInfo.FixTarget));
                        if (output.Contains("SUCCESS") || output.Contains("SUCCESS"))
                        {
                            MessageBox.Show("修复操作执行成功！", "成功", MessageBoxButtons.OK, MessageBoxIcon.Information);
                            // 如果是 PATH 相关，传播环境变量变更
                            RunHealthCheck();
                        }
                        else
                        {
                            // 检查是否为可处理的结果
                            if (!output.StartsWith("ERROR"))
                            {
                                MessageBox.Show("修复操作已完成。\n" + output, "结果", MessageBoxButtons.OK, MessageBoxIcon.Information);
                                RunHealthCheck();
                            }
                            else
                            {
                                MessageBox.Show("修复失败：" + output, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                            }
                        }
                    }
                    finally
                    {
                        this.Cursor = Cursors.Default;
                    }
                }
            }
        }

        // 通过卸载命令行卸载外部软件
        private void UninstallExternalSoftware(string uninstallCmd)
        {
            try
            {
                string exePath = "";
                string args = "";

                if (uninstallCmd.StartsWith("\""))
                {
                    int nextQuote = uninstallCmd.IndexOf("\"", 1);
                    if (nextQuote != -1)
                    {
                        exePath = uninstallCmd.Substring(1, nextQuote - 1).Trim();
                        args = uninstallCmd.Substring(nextQuote + 1).Trim();
                    }
                    else
                    {
                        exePath = uninstallCmd.Replace("\"", "").Trim();
                    }
                }
                else
                {
                    int spaceIdx = uninstallCmd.IndexOf(' ');
                    string lowerCmd = uninstallCmd.ToLower();
                    if (spaceIdx == -1)
                    {
                        exePath = uninstallCmd;
                    }
                    else if (lowerCmd.StartsWith("msiexec"))
                    {
                        exePath = "msiexec.exe";
                        args = uninstallCmd.Substring(spaceIdx + 1).Trim();
                    }
                    else
                    {
                        int exeIdx = lowerCmd.IndexOf(".exe");
                        if (exeIdx != -1)
                        {
                            exePath = uninstallCmd.Substring(0, exeIdx + 4).Trim();
                            args = uninstallCmd.Substring(exeIdx + 4).Trim();
                        }
                        else
                        {
                            exePath = uninstallCmd.Substring(0, spaceIdx).Trim();
                            args = uninstallCmd.Substring(spaceIdx + 1).Trim();
                        }
                    }
                }

                // 处理 MSI 卸载
                if (exePath.ToLower().Contains("msiexec"))
                {
                    string argsLower = args.ToLower();
                    if (argsLower.Contains("/i") && !argsLower.Contains("/x"))
                    {
                        int iIdx = argsLower.IndexOf("/i");
                        args = args.Substring(0, iIdx) + "/x" + args.Substring(iIdx + 2);
                    }
                    else if (!argsLower.Contains("/x"))
                    {
                        args = "/x " + args;
                    }
                    if (!argsLower.Contains("/q"))
                    {
                        args += " /qb /norestart";
                    }
                }
                else
                {
                    string exeLower = exePath.ToLower();
                    if (exeLower.Contains("unins") && !args.ToLower().Contains("/silent"))
                    {
                        args += " /VERYSILENT /SUPPRESSMSGBOXES /NORESTART";
                    }
                }

                var startInfo = new ProcessStartInfo
                {
                    FileName = exePath,
                    Arguments = args,
                    UseShellExecute = true,
                    Verb = "runas"
                };

                using (var process = Process.Start(startInfo))
                {
                    if (process != null)
                    {
                        process.WaitForExit();
                    }
                }

                MessageBox.Show("外部软件的卸载程序执行完毕，即将重新扫描环境状态！", "卸载完成", MessageBoxButtons.OK, MessageBoxIcon.Information);
                RunHealthCheck();
            }
            catch (Exception ex)
            {
                MessageBox.Show("启动卸载程序失败：" + ex.Message, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
            }
        }

        private void btnOptimizePath_Click(object sender, EventArgs e)
        {
            var result = MessageBox.Show("您确定要优化 Windows PATH 环境变量吗？\n\n这将自动执行：\n1. 清理所有不存在的死链目录和重复项\n2. 将 Any-Version 的虚拟链接路径移至最前（置顶）以保证版本切换完美优先。", "确认优化环境变量", MessageBoxButtons.YesNo, MessageBoxIcon.Warning);
            if (result == DialogResult.Yes)
            {
                this.Cursor = Cursors.WaitCursor;
                try
                {
                    string output = RunCliCommand("env clean");
                    if (output.Contains("SUCCESS"))
                    {
                        MessageBox.Show("Windows PATH 环境变量优化成功！请重启您的终端或 IDE (如 VS Code) 使变更生效。", "优化成功", MessageBoxButtons.OK, MessageBoxIcon.Information);
                        RunHealthCheck();
                    }
                    else
                    {
                        MessageBox.Show("环境变量清理失败：" + output, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                    }
                }
                finally
                {
                    this.Cursor = Cursors.Default;
                }
            }
        }

        // --- 系统实用工具相关动作 ---
        private void btnCheckPort_Click(object sender, EventArgs e)
        {
            string port = txtPortToCheck.Text.Trim();
            if (string.IsNullOrEmpty(port))
            {
                MessageBox.Show("请输入需要检测的端口号。", "警告", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                return;
            }

            this.Cursor = Cursors.WaitCursor;
            try
            {
                string output = RunCliCommand("port check " + port);
                if (output.StartsWith("ERROR"))
                {
                    lblPortStatus.Text = "检测失败：" + output.Substring(6);
                    lblPortStatus.ForeColor = colorDanger;
                    btnKillPortProcess.Enabled = false;
                }
                else
                {
                    var parts = output.Split('|');
                    if (parts.Length >= 6)
                    {
                        bool isFree = parts[0].Trim() == "true";
                        bool isReserved = parts[1].Trim() == "true";
                        bool isOccupied = parts[2].Trim() == "true";
                        string pidStr = parts[4].Trim();
                        string procName = parts[5].Trim();

                        if (isOccupied)
                        {
                            lblPortStatus.Text = string.Format("状态: 该端口当前被占用\n进程 PID: {0}\n进程名称: {1}", pidStr, procName);
                            lblPortStatus.ForeColor = colorDanger;
                            btnKillPortProcess.Enabled = true;
                        }
                        else if (isReserved)
                        {
                            lblPortStatus.Text = string.Format("状态: 端口被系统排除/保留 (Reserved Range)\n虽然当前未被进程占用，但可能无法正常绑定。");
                            lblPortStatus.ForeColor = Color.Orange;
                            btnKillPortProcess.Enabled = false;
                        }
                        else
                        {
                            lblPortStatus.Text = string.Format("状态: TCP 端口 {0} 当前完全空闲。", port);
                            lblPortStatus.ForeColor = colorSuccess;
                            btnKillPortProcess.Enabled = false;
                        }
                    }
                    else
                    {
                        lblPortStatus.Text = "异常的响应格式: " + output;
                        lblPortStatus.ForeColor = colorDanger;
                        btnKillPortProcess.Enabled = false;
                    }
                }
            }
            finally
            {
                this.Cursor = Cursors.Default;
            }
        }

        private void btnKillPortProcess_Click(object sender, EventArgs e)
        {
            string port = txtPortToCheck.Text.Trim();
            if (string.IsNullOrEmpty(port)) return;

            var result = MessageBox.Show(string.Format("您确定要强行结束占用端口 {0} 的进程吗？", port), "确认释放端口", MessageBoxButtons.YesNo, MessageBoxIcon.Warning);
            if (result == DialogResult.Yes)
            {
                this.Cursor = Cursors.WaitCursor;
                try
                {
                    string output = RunCliCommand("port kill " + port);
                    if (output.Contains("SUCCESS"))
                    {
                        MessageBox.Show(string.Format("已成功杀死占用进程，释放端口 {0}！", port), "成功", MessageBoxButtons.OK, MessageBoxIcon.Information);
                        btnCheckPort_Click(null, null);
                    }
                    else
                    {
                        MessageBox.Show("释放端口失败：" + output, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                    }
                }
                finally
                {
                    this.Cursor = Cursors.Default;
                }
            }
        }

        private void LoadHostsFile()
        {
            string output = RunCliCommand("hosts read");
            if (output.StartsWith("ERROR"))
            {
                txtHostsContent.Text = "读取 Hosts 文件失败：" + output;
                btnSaveHosts.Enabled = false;
                return;
            }
            txtHostsContent.Text = output;
            btnSaveHosts.Enabled = true;

            // 同步可视列表
            ParseHosts(output);
            RenderHostsVisual();
        }

        private void btnSaveHosts_Click(object sender, EventArgs e)
        {
            this.Cursor = Cursors.WaitCursor;
            try
            {
                string output = RunCliCommandWithStdin("hosts write", txtHostsContent.Text);
                if (output.Contains("SUCCESS"))
                {
                    MessageBox.Show("系统 Hosts 文件保存修改成功！", "成功", MessageBoxButtons.OK, MessageBoxIcon.Information);
                    ParseHosts(txtHostsContent.Text);
                    RenderHostsVisual();
                }
                else
                {
                    MessageBox.Show("保存 Hosts 失败 (可能需要管理员身份运行本程序)：\n" + output, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                }
            }
            finally
            {
                this.Cursor = Cursors.Default;
            }
        }

        private void btnApplyHostsTemplate_Click(object sender, EventArgs e)
        {
            var sb = new StringBuilder(txtHostsContent.Text);
            sb.AppendLine();
            sb.AppendLine("# --- GitHub 下载加速 Host (由 Any-Version 追加) ---");
            sb.AppendLine("140.82.113.3      github.com");
            sb.AppendLine("185.199.108.153   github.githubassets.com");
            sb.AppendLine("199.232.69.194    github.global.ssl.fastly.net");
            sb.AppendLine("185.199.108.133   raw.githubusercontent.com");
            sb.AppendLine("185.199.108.133   objects.githubusercontent.com");
            
            txtHostsContent.Text = sb.ToString();
            
            ParseHosts(txtHostsContent.Text);
            RenderHostsVisual();
            MessageBox.Show("加速模板已成功追加至文本框，请点击底部的“保存 Hosts 修改”写入系统。", "追加成功", MessageBoxButtons.OK, MessageBoxIcon.Information);
        }

        private void ParseHosts(string content)
        {
            hostsLines.Clear();
            if (string.IsNullOrEmpty(content)) return;
            
            string[] lines = content.Split(new[] { "\r\n", "\n" }, StringSplitOptions.None);
            foreach (var line in lines)
            {
                var hl = new HostsLine { Raw = line, IsHostEntry = false };
                string trimmed = line.Trim();
                if (string.IsNullOrEmpty(trimmed))
                {
                    hostsLines.Add(hl);
                    continue;
                }

                if (trimmed.StartsWith("#"))
                {
                    string afterHash = trimmed.Substring(1).Trim();
                    var parts = SplitByWhitespace(afterHash);
                    if (parts.Count >= 2)
                    {
                        System.Net.IPAddress ip;
                        if (System.Net.IPAddress.TryParse(parts[0], out ip))
                        {
                            hl.IsHostEntry = true;
                            hl.Entry = new HostEntry
                            {
                                Ip = parts[0],
                                Hostname = parts[1],
                                Enabled = false,
                                TrailingComment = getTrailingComment(afterHash, parts[0], parts[1])
                            };
                        }
                    }
                }
                else
                {
                    var parts = SplitByWhitespace(trimmed);
                    if (parts.Count >= 2)
                    {
                        System.Net.IPAddress ip;
                        if (System.Net.IPAddress.TryParse(parts[0], out ip))
                        {
                            hl.IsHostEntry = true;
                            hl.Entry = new HostEntry
                            {
                                Ip = parts[0],
                                Hostname = parts[1],
                                Enabled = true,
                                TrailingComment = getTrailingComment(trimmed, parts[0], parts[1])
                            };
                        }
                    }
                }
                hostsLines.Add(hl);
            }
        }

        private string SerializeHosts()
        {
            var sb = new StringBuilder();
            for (int i = 0; i < hostsLines.Count; i++)
            {
                var hl = hostsLines[i];
                if (hl.IsHostEntry && hl.Entry != null)
                {
                    string entryStr = string.Format("{0,-16} {1}", hl.Entry.Ip, hl.Entry.Hostname);
                    if (!string.IsNullOrEmpty(hl.Entry.TrailingComment))
                    {
                        entryStr += " " + hl.Entry.TrailingComment;
                    }
                    if (!hl.Entry.Enabled)
                    {
                        entryStr = "# " + entryStr;
                    }
                    sb.Append(entryStr);
                }
                else
                {
                    sb.Append(hl.Raw);
                }
                if (i < hostsLines.Count - 1)
                {
                    sb.AppendLine();
                }
            }
            return sb.ToString();
        }

        private List<string> SplitByWhitespace(string input)
        {
            var result = new List<string>();
            var parts = input.Split(new[] { ' ', '\t' }, StringSplitOptions.RemoveEmptyEntries);
            foreach (var part in parts)
            {
                result.Add(part);
            }
            return result;
        }

        private string getTrailingComment(string original, string ip, string host)
        {
            int ipIdx = original.IndexOf(ip);
            if (ipIdx == -1) return "";
            int hostIdx = original.IndexOf(host, ipIdx + ip.Length);
            if (hostIdx == -1) return "";
            
            string remainder = original.Substring(hostIdx + host.Length).Trim();
            if (remainder.StartsWith("#"))
            {
                return remainder;
            }
            return "";
        }

        private void RenderHostsVisual()
        {
            lvHostsVisual.Items.Clear();
            foreach (var hl in hostsLines)
            {
                if (hl.IsHostEntry && hl.Entry != null)
                {
                    var item = new ListViewItem(hl.Entry.Enabled ? "已启用" : "已禁用");
                    item.SubItems.Add(hl.Entry.Ip);
                    item.SubItems.Add(hl.Entry.Hostname);
                    item.ForeColor = hl.Entry.Enabled ? colorSuccess : colorTextMuted;
                    item.Tag = hl;
                    lvHostsVisual.Items.Add(item);
                }
            }
        }

        private void SaveAndRefreshHosts()
        {
            string newContent = SerializeHosts();
            txtHostsContent.Text = newContent;

            this.Cursor = Cursors.WaitCursor;
            try
            {
                string output = RunCliCommandWithStdin("hosts write", newContent);
                if (output.Contains("SUCCESS"))
                {
                    ParseHosts(newContent);
                    RenderHostsVisual();
                }
                else
                {
                    MessageBox.Show("保存 Hosts 失败 (可能需要管理员身份运行本程序)：\n" + output, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                }
            }
            finally
            {
                this.Cursor = Cursors.Default;
            }
        }

        private void btnNewHostAdd_Click(object sender, EventArgs e)
        {
            string ipStr = txtNewHostIp.Text.Trim();
            string hostStr = txtNewHostName.Text.Trim();

            if (string.IsNullOrEmpty(ipStr) || string.IsNullOrEmpty(hostStr))
            {
                MessageBox.Show("IP 地址和域名均不能为空！", "警告", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                return;
            }

            System.Net.IPAddress ip;
            if (!System.Net.IPAddress.TryParse(ipStr, out ip))
            {
                MessageBox.Show("请输入合法的 IP 地址！", "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                return;
            }

            var hl = new HostsLine
            {
                IsHostEntry = true,
                Entry = new HostEntry
                {
                    Ip = ipStr,
                    Hostname = hostStr,
                    Enabled = true,
                    TrailingComment = ""
                }
            };

            hostsLines.Add(hl);
            SaveAndRefreshHosts();

            txtNewHostIp.Clear();
            txtNewHostName.Clear();
        }

        private void btnHostToggle_Click(object sender, EventArgs e)
        {
            if (lvHostsVisual.SelectedItems.Count == 0)
            {
                MessageBox.Show("请先在列表中选中一个映射项以切换状态。", "提示", MessageBoxButtons.OK, MessageBoxIcon.Information);
                return;
            }

            var item = lvHostsVisual.SelectedItems[0];
            var hl = item.Tag as HostsLine;
            if (hl != null && hl.Entry != null)
            {
                hl.Entry.Enabled = !hl.Entry.Enabled;
                SaveAndRefreshHosts();
            }
        }

        private void btnHostDelete_Click(object sender, EventArgs e)
        {
            if (lvHostsVisual.SelectedItems.Count == 0)
            {
                MessageBox.Show("请先在列表中选中一个映射项以进行删除。", "提示", MessageBoxButtons.OK, MessageBoxIcon.Information);
                return;
            }

            var item = lvHostsVisual.SelectedItems[0];
            var hl = item.Tag as HostsLine;
            if (hl != null)
            {
                var result = MessageBox.Show(string.Format("您确认要删除映射 {0} -> {1} 吗？", hl.Entry.Ip, hl.Entry.Hostname), "确认删除", MessageBoxButtons.YesNo, MessageBoxIcon.Warning);
                if (result == DialogResult.Yes)
                {
                    hostsLines.Remove(hl);
                    SaveAndRefreshHosts();
                }
            }
        }

        private void InitializePkgsPanel()
        {
            var lblTitle = new Label { Text = "全局开发包管理", Font = new Font("Microsoft YaHei", 15F, FontStyle.Bold), Location = new Point(25, 20), Size = new Size(400, 35) };
            var lblSubtitle = new Label { Text = "一键列出并升级全局安装的开发依赖包，保持您的开发环境为最新版本。", ForeColor = colorTextMuted, Location = new Point(25, 55), Size = new Size(800, 20) };
            panelPkgs.Controls.Add(lblTitle);
            panelPkgs.Controls.Add(lblSubtitle);

            cbPkgSdks = new ComboBox { Location = new Point(25, 90), Size = new Size(200, 28), DropDownStyle = ComboBoxStyle.DropDownList };
            cbPkgSdks.Items.Add("Node.js (NPM)");
            cbPkgSdks.Items.Add("Python (PIP)");
            cbPkgSdks.SelectedIndex = 0;
            cbPkgSdks.SelectedIndexChanged += (s, e) => LoadPkgsList();
            panelPkgs.Controls.Add(cbPkgSdks);

            btnPkgRefresh = CreateThemeButton("刷新列表", 240, 89, 120, 28, false);
            btnPkgRefresh.Click += (s, e) => LoadPkgsList();
            panelPkgs.Controls.Add(btnPkgRefresh);

            btnPkgUpgrade = CreateThemeButton("升级选中包", 375, 89, 150, 28, true);
            btnPkgUpgrade.Click += btnPkgUpgrade_Click;
            btnPkgUpgrade.Enabled = false;
            panelPkgs.Controls.Add(btnPkgUpgrade);

            btnPkgHomepage = CreateThemeButton("浏览项目首页", 540, 89, 150, 28, false);
            btnPkgHomepage.Click += btnPkgHomepage_Click;
            btnPkgHomepage.Enabled = false;
            panelPkgs.Controls.Add(btnPkgHomepage);

            lvPkgs = new ListView { Location = new Point(25, 130), Size = new Size(930, 360), View = View.Details, FullRowSelect = true, BorderStyle = BorderStyle.FixedSingle, HeaderStyle = ColumnHeaderStyle.Clickable };
            lvPkgs.Columns.Add("包名称", 280);
            lvPkgs.Columns.Add("当前版本", 180);
            lvPkgs.Columns.Add("最新版本", 180);
            lvPkgs.Columns.Add("状态", 180);
            lvPkgs.SelectedIndexChanged += (s, e) => {
                if (lvPkgs.SelectedItems.Count > 0) {
                    var status = lvPkgs.SelectedItems[0].SubItems[3].Text;
                    btnPkgUpgrade.Enabled = (status == "可升级" || status == "Outdated");
                    btnPkgHomepage.Enabled = true;
                } else {
                    btnPkgUpgrade.Enabled = false;
                    btnPkgHomepage.Enabled = false;
                }
            };
            panelPkgs.Controls.Add(lvPkgs);

            lblPkgLoading = new Label
            {
                Text = "正在读取全局包列表... 请稍候...",
                Font = new Font("Microsoft YaHei", 10.5F, FontStyle.Bold),
                ForeColor = colorAccent,
                BackColor = colorControlBg,
                TextAlign = ContentAlignment.MiddleCenter,
                Visible = false
            };
            panelPkgs.Controls.Add(lblPkgLoading);

            var lblPkgTips = new Label
            {
                Text = "提示：全局包的列表获取与升级需要向远程源查询（可能需要十几秒钟）。升级过程中程序会异步等待执行。NPM 升级使用 npm install -g <name>@latest，PIP 升级使用 pip install --upgrade <name>。",
                Location = new Point(25, 505),
                Size = new Size(930, 45),
                ForeColor = colorTextMuted,
                Font = new Font("Microsoft YaHei", 8.5F, FontStyle.Italic)
            };
            panelPkgs.Controls.Add(lblPkgTips);
        }

        private void LoadPkgsList()
        {
            if (cbPkgSdks.SelectedItem == null) return;
            string sdkText = cbPkgSdks.SelectedItem.ToString();
            string sdkName = sdkText.Contains("NPM") ? "nodejs" : "python";

            lvPkgs.Items.Clear();
            btnPkgUpgrade.Enabled = false;
            btnPkgHomepage.Enabled = false;

            lblPkgLoading.Bounds = lvPkgs.Bounds;
            lblPkgLoading.Visible = true;
            lblPkgLoading.BringToFront();

            Thread thread = new Thread(() =>
            {
                string output = RunCliCommand("pkg list " + sdkName);
                this.Invoke((MethodInvoker)delegate
                {
                    lblPkgLoading.Visible = false;
                    if (output.StartsWith("ERROR"))
                    {
                        MessageBox.Show("获取全局包列表失败：" + output.Substring(6), "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                        return;
                    }

                    string[] lines = output.Split('\n');
                    foreach (var line in lines)
                    {
                        var parts = line.Split('|');
                        if (parts.Length >= 4 && !string.IsNullOrEmpty(parts[0].Trim()))
                        {
                            var item = new ListViewItem(parts[0].Trim());
                            item.SubItems.Add(parts[1].Trim());
                            item.SubItems.Add(parts[2].Trim());
                            
                            string status = parts[3].Trim();
                            if (status == "up_to_date" || status == "latest")
                            {
                                item.SubItems.Add("已是最新");
                                item.ForeColor = colorSuccess;
                            }
                            else if (status == "outdated")
                            {
                                item.SubItems.Add("可升级");
                                item.ForeColor = Color.Orange;
                            }
                            else
                            {
                                item.SubItems.Add(status);
                                item.ForeColor = colorTextMain;
                            }

                            lvPkgs.Items.Add(item);
                        }
                    }
                });
            });
            thread.Start();
        }

        private void btnPkgUpgrade_Click(object sender, EventArgs e)
        {
            if (lvPkgs.SelectedItems.Count == 0 || cbPkgSdks.SelectedItem == null) return;
            var item = lvPkgs.SelectedItems[0];
            string pkgName = item.Text;
            string sdkText = cbPkgSdks.SelectedItem.ToString();
            string sdkName = sdkText.Contains("NPM") ? "nodejs" : "python";

            btnPkgUpgrade.Enabled = false;
            btnPkgHomepage.Enabled = false;
            btnPkgRefresh.Enabled = false;
            cbPkgSdks.Enabled = false;

            lblPkgLoading.Text = "正在升级包 " + pkgName + "... 请稍候...";
            lblPkgLoading.Bounds = lvPkgs.Bounds;
            lblPkgLoading.Visible = true;
            lblPkgLoading.BringToFront();

            Thread thread = new Thread(() =>
            {
                string output = RunCliCommand(string.Format("pkg upgrade {0} {1}", sdkName, pkgName));
                this.Invoke((MethodInvoker)delegate
                {
                    lblPkgLoading.Visible = false;
                    lblPkgLoading.Text = "正在读取全局包列表... 请稍候...";
                    btnPkgRefresh.Enabled = true;
                    cbPkgSdks.Enabled = true;

                    if (output.Contains("SUCCESS"))
                    {
                        MessageBox.Show(string.Format("全局包 {0} 升级成功！", pkgName), "升级成功", MessageBoxButtons.OK, MessageBoxIcon.Information);
                        LoadPkgsList();
                    }
                    else
                    {
                        MessageBox.Show("升级包失败：\n" + output, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
                        LoadPkgsList();
                    }
                });
            });
            thread.Start();
        }

        private void btnPkgHomepage_Click(object sender, EventArgs e)
        {
            if (lvPkgs.SelectedItems.Count == 0 || cbPkgSdks.SelectedItem == null) return;
            var item = lvPkgs.SelectedItems[0];
            string pkgName = item.Text;
            string sdkText = cbPkgSdks.SelectedItem.ToString();
            
            string url = "";
            if (sdkText.Contains("NPM"))
            {
                url = "https://www.npmjs.com/package/" + pkgName;
            }
            else
            {
                url = "https://pypi.org/project/" + pkgName + "/";
            }

            try
            {
                Process.Start(url);
            }
            catch (Exception ex)
            {
                MessageBox.Show("无法打开网页：" + ex.Message, "错误", MessageBoxButtons.OK, MessageBoxIcon.Error);
            }
        }
    }

    public static class PromptDialog
    {
        public static string Show(string title, string text)
        {
            Form prompt = new Form()
            {
                Width = 400,
                Height = 160,
                FormBorderStyle = FormBorderStyle.FixedDialog,
                Text = title,
                StartPosition = FormStartPosition.CenterParent,
                BackColor = Color.FromArgb(20, 20, 26),
                ForeColor = Color.White
            };
            Label textLabel = new Label() { Left = 20, Top = 15, Width = 350, Height = 20, Text = text };
            TextBox textBox = new TextBox() { Left = 20, Top = 40, Width = 350, BorderStyle = BorderStyle.FixedSingle, BackColor = Color.FromArgb(30, 30, 40), ForeColor = Color.White };
            Button confirmation = new Button() { Text = "确定", Left = 270, Width = 100, Top = 80, DialogResult = DialogResult.OK, FlatStyle = FlatStyle.Flat };
            
            confirmation.FlatAppearance.BorderSize = 1;
            confirmation.FlatAppearance.BorderColor = Color.FromArgb(99, 102, 241);
            confirmation.BackColor = Color.FromArgb(99, 102, 241);
            
            prompt.Controls.Add(textBox);
            prompt.Controls.Add(confirmation);
            prompt.Controls.Add(textLabel);
            prompt.AcceptButton = confirmation;

            return prompt.ShowDialog() == DialogResult.OK ? textBox.Text : "";
        }
    }
}
