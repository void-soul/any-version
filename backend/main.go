package main

import (
	"fmt"
	"io/ioutil"
	"os"
	"path/filepath"
	"strings"
)

var sdks = []SDK{
	&NodeJS{},
	&GoSDK{},
	&BunSDK{},
	&PythonSDK{},
	&JavaSDK{},
	&AndroidSDK{},
	&FlutterSDK{},
	&RustSDK{},
	&PHPSDK{},
	&RubySDK{},
	&NginxSDK{},
	&RedisSDK{},
	&MySQLSDK{},
	&MongoDB{},
	&PostgreSQL{},
	&Maven{},
	&Gradle{},
	&Pub{},
	&Vcpkg{},
	&Yarn{},
	&Pnpm{},
}

func findSDK(name string) SDK {
	for _, sdk := range sdks {
		if strings.ToLower(sdk.Name()) == strings.ToLower(name) {
			return sdk
		}
	}
	return nil
}

func getBaseDir() string {
	userProfile := os.Getenv("USERPROFILE")
	if userProfile == "" {
		userProfile = os.Getenv("HOMEDRIVE") + os.Getenv("HOMEPATH")
	}
	if userProfile == "" {
		userProfile = "C:\\any-version"
	}
	return filepath.Join(userProfile, ".any-version")
}

func main() {
	// Load configuration settings first
	LoadConfig()

	if len(os.Args) < 2 {
		printUsage()
		return
	}

	baseDir := getBaseDir()
	command := strings.ToLower(os.Args[1])

	switch command {
	case "init":
		initCmd(baseDir)
	case "list", "ls":
		listCmd(os.Args[2:], baseDir)
	case "list-remote", "lr":
		listRemoteCmd(os.Args[2:])
	case "install":
		installCmd(os.Args[2:], baseDir)
	case "uninstall":
		uninstallCmd(os.Args[2:], baseDir)
	case "use":
		useCmd(os.Args[2:], baseDir)
	case "add":
		addCmd(os.Args[2:], baseDir)
	case "sdk":
		sdkCmd(os.Args[2:])
	case "cache":
		cacheCmd(os.Args[2:])
	case "config":
		configCmd(os.Args[2:])
	case "service":
		serviceCmd(os.Args[2:])
	case "mirror":
		mirrorCmd(os.Args[2:])
	case "env":
		envCmd(os.Args[2:])
	case "port":
		portCmd(os.Args[2:])
	case "pkg":
		pkgCmd(os.Args[2:])
	case "hosts":
		hostsCmd(os.Args[2:])
	default:
		fmt.Printf("未知命令：%s\n", command)
		printUsage()
	}
}

func printUsage() {
	fmt.Println("Any-Version (av) - Windows 多语言开发环境版本管理器")
	fmt.Println("\n用法:")
	fmt.Println("  av init                              初始化环境变量与目录结构")
	fmt.Println("  av list [sdk]                        列出所有或特定 SDK 已安装的版本")
	fmt.Println("  av list-remote <sdk>                 列出可在线安装的远程版本")
	fmt.Println("  av install <sdk> <version>           下载并安装特定版本的 SDK")
	fmt.Println("  av uninstall <sdk> <version>         卸载特定版本的 SDK")
	fmt.Println("  av use <sdk> <version>               切换当前启用的 SDK 版本 (使用目录联接)")
	fmt.Println("  av add <sdk> <version> <local_path>  手动注册已存在的本地 SDK 目录")
	fmt.Println("\nGUI 辅助命令:")
	fmt.Println("  av sdk list                          列出所有 SDK 状态信息")
	fmt.Println("  av cache list                        列出开发包缓存路径及大小")
	fmt.Println("  av cache set <name> <path>           更新特定开发包缓存的目标路径")
	fmt.Println("  av config get                        获取全局配置目录")
	fmt.Println("  av config set <vdir> <ldir>          保存全局配置目录")
	fmt.Println("  av pkg list <nodejs|python>          列出全局依赖包及更新信息")
	fmt.Println("  av pkg upgrade <sdk> <name>          升级特定的全局依赖包")
	fmt.Println("\n支持的开发库/工具 (SDKs 与本地服务):")
	fmt.Println("  nodejs, java, python, flutter, go, rust, bun, nginx, redis, mysql")
}

func initCmd(baseDir string) {
	fmt.Println("正在初始化 Any-Version...")
	// Create required directories
	versionsDir := globalConfig.VersionsDir
	linksDir := globalConfig.LinksDir
	tmpDir := filepath.Join(baseDir, ".tmp")

	for _, dir := range []string{versionsDir, linksDir, tmpDir} {
		if err := os.MkdirAll(dir, 0755); err != nil {
			fmt.Printf("创建目录 %s 失败: %v\n", dir, err)
			return
		}
	}

	// Configure path and environment variables
	if err := InitEnvironment(baseDir); err != nil {
		fmt.Printf("配置环境变量失败: %v\n", err)
		return
	}

	fmt.Println("Any-Version 初始化成功！")
}

func listCmd(args []string, baseDir string) {
	if len(args) > 0 {
		sdkName := args[0]
		sdk := findSDK(sdkName)
		if sdk == nil {
			fmt.Printf("未知的 SDK: %s。支持的工具: nodejs, java, python, flutter, go, rust, bun, nginx, redis, mysql\n", sdkName)
			return
		}
		fmt.Printf("[%s]\n", sdk.Name())
		listSDKVersions(sdk, baseDir)
		return
	}

	fmt.Println("已安装版本 (带有 * 的为当前启用版本):")
	for _, sdk := range sdks {
		fmt.Printf("\n[%s]\n", sdk.Name())
		listSDKVersions(sdk, baseDir)
	}
}

func listSDKVersions(sdk SDK, baseDir string) {
	sdkDir := filepath.Join(globalConfig.VersionsDir, sdk.Name())
	junctionPath := filepath.Join(globalConfig.LinksDir, sdk.Name())

	// Check where the link currently points
	activeDir, _ := filepath.EvalSymlinks(junctionPath)
	activeDir = strings.ToLower(filepath.Clean(activeDir))

	entries, err := ioutil.ReadDir(sdkDir)
	if err != nil || len(entries) == 0 {
		fmt.Println("  (未安装任何版本。请运行 'av install' 或 'av add' 进行注册。)")
		return
	}

	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		vName := entry.Name()
		fullPath := strings.ToLower(filepath.Clean(filepath.Join(sdkDir, vName)))

		if fullPath == activeDir {
			fmt.Printf("  * %s (当前启用)\n", vName)
		} else {
			fmt.Printf("    %s\n", vName)
		}
	}
}

func listRemoteCmd(args []string) {
	if len(args) < 1 {
		fmt.Println("错误: 缺失 SDK 名称。用法: av list-remote <sdk>")
		return
	}

	sdkName := args[0]
	sdk := findSDK(sdkName)
	if sdk == nil {
		fmt.Printf("未知的 SDK: %s。支持的工具: nodejs, java, python, flutter, go, rust, bun, nginx, redis, mysql\n", sdkName)
		return
	}

	fmt.Printf("正在拉取 %s 的远程版本列表...\n", sdk.Name())
	versions, err := sdk.ListRemote()
	if err != nil {
		fmt.Printf("拉取远程版本失败: %v\n", err)
		return
	}

	fmt.Printf("可用远程版本 %s:\n", sdk.Name())
	for _, v := range versions {
		fmt.Printf("  %s\n", v)
	}
}

func installCmd(args []string, baseDir string) {
	if len(args) < 2 {
		fmt.Println("错误: 参数缺失。用法: av install <sdk> <version>")
		return
	}

	sdkName := args[0]
	version := args[1]

	sdk := findSDK(sdkName)
	if sdk == nil {
		fmt.Printf("未知的 SDK: %s。支持的工具: nodejs, java, python, flutter, go, rust, bun, nginx, redis, mysql\n", sdkName)
		return
	}

	destDir := filepath.Join(globalConfig.VersionsDir, sdk.Name(), version)
	if _, err := os.Stat(destDir); err == nil {
		fmt.Printf("版本 %s 的 %s 已经安装。\n", version, sdk.Name())
		return
	}

	// Pass parent of versions folder to install implementations as baseDir parameter
	sdkBaseDir := filepath.Dir(globalConfig.VersionsDir)
	if err := sdk.Install(version, sdkBaseDir); err != nil {
		fmt.Printf("安装失败: %v\n", err)
		// Clean up partial downloads/installs
		os.RemoveAll(destDir)
		return
	}

	// Auto-use if it's the first version installed for this SDK
	junctionPath := filepath.Join(globalConfig.LinksDir, sdk.Name())
	if _, err := os.Lstat(junctionPath); os.IsNotExist(err) {
		fmt.Printf("当前没有启用的版本。自动切换到 %s...\n", version)
		if err := CreateJunction(junctionPath, destDir); err != nil {
			fmt.Printf("警告: 切换启用版本失败: %v\n", err)
		}
	}
}

func uninstallCmd(args []string, baseDir string) {
	if len(args) < 2 {
		fmt.Println("错误: 参数缺失。用法: av uninstall <sdk> <version>")
		return
	}

	sdkName := args[0]
	version := args[1]

	sdk := findSDK(sdkName)
	if sdk == nil {
		fmt.Printf("未知的 SDK: %s。支持的工具: nodejs, java, python, flutter, go, rust, bun, nginx, redis, mysql\n", sdkName)
		return
	}

	destDir := filepath.Join(globalConfig.VersionsDir, sdk.Name(), version)
	if _, err := os.Stat(destDir); os.IsNotExist(err) {
		fmt.Printf("版本 %s 的 %s 未安装。\n", version, sdk.Name())
		return
	}

	// If active, remove the junction first
	junctionPath := filepath.Join(globalConfig.LinksDir, sdk.Name())
	activeDir, _ := filepath.EvalSymlinks(junctionPath)
	if strings.ToLower(filepath.Clean(activeDir)) == strings.ToLower(filepath.Clean(destDir)) {
		fmt.Printf("正在移除 %s 的当前启用链接...\n", sdk.Name())
		RemoveJunction(junctionPath)
	}

	fmt.Printf("正在卸载 %s 版本 %s...\n", sdk.Name(), version)
	if err := os.RemoveAll(destDir); err != nil {
		fmt.Printf("删除文件出错: %v\n", err)
		return
	}

	fmt.Printf("成功卸载 %s v%s。\n", sdk.Name(), version)
}

func useCmd(args []string, baseDir string) {
	if len(args) < 2 {
		fmt.Println("错误: 参数缺失。用法: av use <sdk> <version>")
		return
	}

	sdkName := args[0]
	version := args[1]

	sdk := findSDK(sdkName)
	if sdk == nil {
		fmt.Printf("未知的 SDK: %s。支持的工具: nodejs, java, python, flutter, go, rust, bun, nginx, redis, mysql\n", sdkName)
		return
	}

	targetDir := filepath.Join(globalConfig.VersionsDir, sdk.Name(), version)
	if _, err := os.Stat(targetDir); os.IsNotExist(err) {
		fmt.Printf("版本 %s 的 %s 未安装。请先运行 'av install %s %s'。\n", version, sdk.Name(), sdkName, version)
		return
	}

	junctionPath := filepath.Join(globalConfig.LinksDir, sdk.Name())
	fmt.Printf("正在将 %s 切换到版本 %s...\n", sdk.Name(), version)
	if err := CreateJunction(junctionPath, targetDir); err != nil {
		fmt.Printf("错误: %v\n", err)
		return
	}

	fmt.Printf("当前正在使用 %s 版本 %s！\n", sdk.Name(), version)
}

func addCmd(args []string, baseDir string) {
	if len(args) < 3 {
		fmt.Println("错误: 参数缺失。用法: av add <sdk> <version> <local_path>")
		return
	}

	sdkName := args[0]
	version := args[1]
	localPath := args[2]

	sdk := findSDK(sdkName)
	if sdk == nil {
		fmt.Printf("未知的 SDK: %s。支持的工具: nodejs, java, python, flutter, go, rust, bun, nginx, redis, mysql\n", sdkName)
		return
	}

	srcInfo, err := os.Stat(localPath)
	if err != nil {
		fmt.Printf("错误: 本地路径 %s 不存在或无法访问\n", localPath)
		return
	}

	if !srcInfo.IsDir() {
		fmt.Printf("错误: 本地路径 %s 不是一个目录\n", localPath)
		return
	}

	destDir := filepath.Join(globalConfig.VersionsDir, sdk.Name(), version)
	if _, err := os.Stat(destDir); err == nil {
		fmt.Printf("错误: 版本 %s 的 %s 已经注册。\n", version, sdk.Name())
		return
	}

	fmt.Printf("正在从 %s 注册并复制 %s v%s...\n", localPath, sdk.Name(), version)
	if err := CopyDir(localPath, destDir); err != nil {
		fmt.Printf("复制文件失败: %v\n", err)
		os.RemoveAll(destDir)
		return
	}

	// Auto-use if it's the first version installed for this SDK
	junctionPath := filepath.Join(globalConfig.LinksDir, sdk.Name())
	if _, err := os.Lstat(junctionPath); os.IsNotExist(err) {
		fmt.Printf("当前没有启用的版本。自动切换到 %s...\n", version)
		if err := CreateJunction(junctionPath, destDir); err != nil {
			fmt.Printf("警告: 切换启用版本失败: %v\n", err)
		}
	}

	fmt.Printf("成功添加并注册 %s 版本 %s！\n", sdk.Name(), version)
}

func sdkCmd(args []string) {
	if len(args) > 0 && args[0] == "list" {
		for _, sdk := range sdks {
			junctionPath := filepath.Join(globalConfig.LinksDir, sdk.Name())
			sdkDir := filepath.Join(globalConfig.VersionsDir, sdk.Name())

			activeDir, _ := filepath.EvalSymlinks(junctionPath)
			activeDirClean := strings.ToLower(filepath.Clean(activeDir))

			var installed []string
			entries, err := ioutil.ReadDir(sdkDir)
			if err == nil {
				for _, entry := range entries {
					if entry.IsDir() {
						installed = append(installed, entry.Name())
					}
				}
			}

			activeVersion := ""
			for _, v := range installed {
				vPath := strings.ToLower(filepath.Clean(filepath.Join(sdkDir, v)))
				if vPath == activeDirClean {
					activeVersion = v
					break
				}
			}

			fmt.Printf("%s|%s|%s|%s\n", sdk.Name(), sdk.Category(), activeVersion, strings.Join(installed, ","))
		}
		return
	}
	fmt.Println("Usage: av sdk list")
}

func cacheCmd(args []string) {
	if len(args) > 0 {
		subCmd := args[0]
		if subCmd == "list" {
			list := GetCachesList()
			for _, cache := range list {
				installedStr := "false"
				if cache.Installed {
					installedStr = "true"
				}
				isLinkStr := "false"
				if cache.IsLink {
					isLinkStr = "true"
				}
				fmt.Printf("%s|%s|%s|%s|%s|%s\n", cache.Name, installedStr, cache.Path, cache.Size, isLinkStr, cache.RealTarget)
			}
			return
		} else if subCmd == "set" && len(args) >= 3 {
			name := args[1]
			path := args[2]
			if err := UpdateCachePath(name, path); err != nil {
				fmt.Printf("ERROR: %v\n", err)
				os.Exit(1)
			}
			fmt.Println("SUCCESS")
			return
		}
	}
	fmt.Println("Usage:")
	fmt.Println("  av cache list")
	fmt.Println("  av cache set <name> <path>")
}

func configCmd(args []string) {
	if len(args) > 0 {
		subCmd := args[0]
		if subCmd == "get" {
			fmt.Printf("versions_dir|%s\n", globalConfig.VersionsDir)
			fmt.Printf("links_dir|%s\n", globalConfig.LinksDir)
			return
		} else if subCmd == "set" && len(args) >= 3 {
			vDir := args[1]
			lDir := args[2]

			oldVDir := globalConfig.VersionsDir
			oldLDir := globalConfig.LinksDir

			globalConfig.VersionsDir = vDir
			globalConfig.LinksDir = lDir
			if err := SaveConfig(); err != nil {
				fmt.Printf("ERROR: %v\n", err)
				os.Exit(1)
			}
			os.MkdirAll(globalConfig.VersionsDir, 0755)
			os.MkdirAll(globalConfig.LinksDir, 0755)

			// Remove old links paths from PATH
			if oldLDir != "" && oldLDir != lDir {
				removePathsFromEnv(oldLDir)
			}

			// Re-register new links paths in PATH
			if err := InitEnvironment(getBaseDir()); err != nil {
				fmt.Printf("WARNING: failed to update PATH: %v\n", err)
			}

			// Re-create junctions for already-installed SDKs
			if oldVDir != "" && oldVDir != vDir {
				fmt.Println("迁移已有 SDK 版本...")
				migrateJunctions(oldVDir, oldLDir)
			}

			fmt.Println("SUCCESS")
			return
		}
	}
	fmt.Println("Usage:")
	fmt.Println("  av config get")
	fmt.Println("  av config set <versions_dir> <links_dir>")
}

// removePathsFromEnv removes any PATH entries that contain the given directory prefix
func removePathsFromEnv(oldDir string) {
	currentPath, err := GetUserEnv("PATH")
	if err != nil {
		return
	}
	oldDirClean := strings.ToLower(filepath.Clean(oldDir))
	pathParts := filepath.SplitList(currentPath)
	var newParts []string
	for _, p := range pathParts {
		pClean := strings.ToLower(filepath.Clean(strings.TrimSpace(p)))
		if pClean != "" && !strings.HasPrefix(pClean, oldDirClean) {
			newParts = append(newParts, p)
		}
	}
	newPath := strings.Join(newParts, string(filepath.ListSeparator))
	if newPath != currentPath {
		SetUserEnv("PATH", newPath)
		BroadcastSettingChange()
	}
}

// migrateJunctions re-creates all junctions from old dirs to new dirs
func migrateJunctions(oldVDir, oldLDir string) {
	entries, err := ioutil.ReadDir(oldLDir)
	if err != nil {
		return
	}
	for _, entry := range entries {
		// Check if it's actually a junction (reparse point)
		junctionPath := filepath.Join(oldLDir, entry.Name())
		target, err := filepath.EvalSymlinks(junctionPath)
		if err != nil {
			continue
		}
		// Map old version path to new
		oldVDirClean := strings.ToLower(filepath.Clean(oldVDir))
		targetClean := strings.ToLower(filepath.Clean(target))
		if strings.HasPrefix(targetClean, oldVDirClean) {
			relPath := target[len(oldVDir):]
			if relPath != "" && relPath[0] == filepath.Separator {
				relPath = relPath[1:]
			}
			newTarget := filepath.Join(globalConfig.VersionsDir, relPath)
			newJunctionPath := filepath.Join(globalConfig.LinksDir, entry.Name())
			// Remove old junction, create new one pointing to migrated target
			RemoveJunction(junctionPath)
			CreateJunction(newJunctionPath, newTarget)
		}
	}
}

func serviceCmd(args []string) {
	if len(args) > 0 {
		subCmd := args[0]
		if subCmd == "list" {
			svcs, err := GetRunningServices()
			if err != nil {
				fmt.Printf("ERROR: %v\n", err)
				return
			}
			for _, svc := range svcs {
				fmt.Printf("%s|%s|%s|%s|%d\n", svc.Name, svc.Status, svc.ActiveVersion, svc.Port, svc.Pid)
			}
			return
		} else if subCmd == "start" && len(args) >= 3 {
			name := args[1]
			version := args[2]
			if err := StartService(name, version); err != nil {
				fmt.Printf("ERROR: %v\n", err)
				return
			}
			fmt.Println("SUCCESS")
			return
		} else if subCmd == "stop" && len(args) >= 2 {
			name := args[1]
			if err := StopService(name); err != nil {
				fmt.Printf("ERROR: %v\n", err)
				return
			}
			fmt.Println("SUCCESS")
			return
		}
	}
	fmt.Println("Usage:")
	fmt.Println("  av service list")
	fmt.Println("  av service start <name> <version>")
	fmt.Println("  av service stop <name>")
}

func mirrorCmd(args []string) {
	if len(args) > 0 {
		subCmd := args[0]
		if subCmd == "list" {
			list := GetMirrorsList()
			for _, m := range list {
				fmt.Printf("%s|%s|%s\n", m.Tool, m.Current, m.MirrorName)
			}
			return
		} else if subCmd == "set" && len(args) >= 3 {
			tool := args[1]
			mirror := args[2]
			if err := SetMirror(tool, mirror); err != nil {
				fmt.Printf("ERROR: %v\n", err)
				return
			}
			fmt.Println("SUCCESS")
			return
		}
	}
	fmt.Println("Usage:")
	fmt.Println("  av mirror list")
	fmt.Println("  av mirror set <tool> <mirror>")
}

func envCmd(args []string) {
	if len(args) > 0 {
		subCmd := args[0]
		if subCmd == "check" {
			checks, err := RunFullDiagnostics()
			if err != nil {
				fmt.Printf("ERROR: %v\n", err)
				return
			}
			for _, check := range checks {
				passedStr := "false"
				if check.Passed {
					passedStr = "true"
				}
				fmt.Printf("CHECK|%s|%s|%s\n", check.CheckID, check.CheckName, passedStr)
				for _, prob := range check.Problems {
					fmt.Printf("PROBLEM|%s|%s|%s|%s|%s\n",
						check.CheckID, prob.ProblemType, prob.ProblemDesc, prob.FixType, prob.FixTarget)
				}
				fmt.Printf("ENDCHECK|%s\n", check.CheckID)
			}
			return
		} else if subCmd == "fix" && len(args) >= 3 {
			fixType := args[1]
			fixTarget := args[2]
			if err := FixDiagnosticIssue(fixType, fixTarget); err != nil {
				fmt.Printf("ERROR: %v\n", err)
				return
			}
			fmt.Println("SUCCESS")
			return
		} else if subCmd == "clean" {
			if err := OptimizeEnvironment(); err != nil {
				fmt.Printf("ERROR: %v\n", err)
				return
			}
			fmt.Println("SUCCESS")
			return
		}
	}
	fmt.Println("Usage:")
	fmt.Println("  av env check")
	fmt.Println("  av env fix <fixType> <fixTarget>")
	fmt.Println("  av env clean")
}

func portCmd(args []string) {
	if len(args) > 0 {
		subCmd := args[0]
		if subCmd == "check" && len(args) >= 2 {
			port := args[1]
			status, err := CheckPortStatus(port)
			if err != nil {
				fmt.Printf("ERROR: %v\n", err)
				return
			}
			// Format: free|reserved|occupied|port|pid|processName
			freeStr := "false"
			reservedStr := "false"
			occupiedStr := "false"
			pidStr := "0"
			procStr := ""
			if status.Free {
				freeStr = "true"
			}
			if status.Reserved {
				reservedStr = "true"
			}
			if status.Occupied && status.Owner != nil {
				occupiedStr = "true"
				pidStr = status.Owner.Pid
				procStr = status.Owner.ProcessName
			}
			fmt.Printf("%s|%s|%s|%s|%s|%s\n", freeStr, reservedStr, occupiedStr, port, pidStr, procStr)
			return
		} else if subCmd == "kill" && len(args) >= 2 {
			port := args[1]
			if err := KillPortOwner(port); err != nil {
				fmt.Printf("ERROR: %v\n", err)
				return
			}
			fmt.Println("SUCCESS")
			return
		}
	}
	fmt.Println("Usage:")
	fmt.Println("  av port check <port>")
	fmt.Println("  av port kill <port>")
}

func hostsCmd(args []string) {
	if len(args) > 0 {
		subCmd := args[0]
		if subCmd == "read" {
			content, err := ReadHosts()
			if err != nil {
				fmt.Printf("ERROR: %v\n", err)
				return
			}
			fmt.Print(content)
			return
		} else if subCmd == "write" {
			data, err := ioutil.ReadAll(os.Stdin)
			if err != nil {
				fmt.Printf("ERROR: %v\n", err)
				return
			}
			err = WriteHosts(string(data))
			if err != nil {
				if err == os.ErrPermission {
					fmt.Println("ERROR: Access Denied. Please run as Administrator.")
				} else {
					fmt.Printf("ERROR: %v\n", err)
				}
				return
			}
			fmt.Println("SUCCESS")
			return
		}
	}
	fmt.Println("Usage:")
	fmt.Println("  av hosts read")
	fmt.Println("  av hosts write")
}

func pkgCmd(args []string) {
	if len(args) > 0 {
		subCmd := args[0]
		if subCmd == "list" && len(args) >= 2 {
			sdkName := args[1]
			list, err := GetGlobalPackages(sdkName)
			if err != nil {
				fmt.Printf("ERROR: %v\n", err)
				return
			}
			for _, pkg := range list {
				fmt.Printf("%s|%s|%s|%s\n", pkg.Name, pkg.CurrentVersion, pkg.LatestVersion, pkg.Status)
			}
			return
		} else if subCmd == "upgrade" && len(args) >= 3 {
			sdkName := args[1]
			pkgName := args[2]
			if err := UpgradeGlobalPackage(sdkName, pkgName); err != nil {
				fmt.Printf("ERROR: %v\n", err)
				return
			}
			fmt.Println("SUCCESS")
			return
		}
	}
	fmt.Println("Usage:")
	fmt.Println("  av pkg list <nodejs|python>")
	fmt.Println("  av pkg upgrade <nodejs|python> <packageName>")
}
