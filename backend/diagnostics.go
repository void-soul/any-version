package main

import (
	"bytes"
	"fmt"
	"io/ioutil"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// DiagnosticCheckItem represents a check category (like 360/腾讯电脑管家)
type DiagnosticCheckItem struct {
	CheckID   string          // unique check ID
	CheckName string          // display name of this check
	Passed    bool            // true if no problems found
	Problems  []DiagnosticProblem
}

// DiagnosticProblem represents a single detected issue
type DiagnosticProblem struct {
	ProblemType  string // "dead_path", "duplicate_path", "conflict", "missing_env", "wrong_env", "not_installed"
	ProblemDesc  string // human-readable description
	ProblemDetail string // detailed info (path, value, etc.)
	FixType      string // "clean_path", "remove_path", "set_env", "install", "uninstall", "manual"
	FixTarget    string // the target to fix (path, env var name, etc.)
}

// EnvVarCheck defines a developer-related environment variable to check
type EnvVarCheck struct {
	Name         string
	Description  string
	ExpectedFunc func() string // returns expected value, empty if no expectation
	Category     string        // "go", "java", "android", "nodejs", "flutter", "rust", "python", "general"
}

var developerEnvVars = []EnvVarCheck{
	// Go-related
	{Name: "GOROOT", Description: "Go 安装根目录", Category: "go", ExpectedFunc: func() string { return "" }},
	{Name: "GOPATH", Description: "Go 工作空间路径", Category: "go", ExpectedFunc: func() string { return "" }},
	{Name: "GOCACHE", Description: "Go 编译缓存目录", Category: "go", ExpectedFunc: func() string { return "" }},
	{Name: "GOMODCACHE", Description: "Go 模块缓存目录", Category: "go", ExpectedFunc: func() string { return "" }},
	{Name: "GOPROXY", Description: "Go 模块代理地址", Category: "go", ExpectedFunc: func() string { return "" }},
	{Name: "GO111MODULE", Description: "Go 模块模式开关", Category: "go", ExpectedFunc: func() string { return "" }},
	{Name: "GOPRIVATE", Description: "Go 私有模块配置", Category: "go", ExpectedFunc: func() string { return "" }},
	{Name: "GOBIN", Description: "Go 编译输出目录", Category: "go", ExpectedFunc: func() string { return "" }},

	// Java-related
	{Name: "JAVA_HOME", Description: "Java 安装根目录", Category: "java", ExpectedFunc: func() string { return "" }},
	{Name: "CLASSPATH", Description: "Java 类路径", Category: "java", ExpectedFunc: func() string { return "" }},
	{Name: "MAVEN_HOME", Description: "Maven 安装目录", Category: "java", ExpectedFunc: func() string { return "" }},
	{Name: "GRADLE_HOME", Description: "Gradle 安装目录", Category: "java", ExpectedFunc: func() string { return "" }},
	{Name: "GRADLE_USER_HOME", Description: "Gradle 用户缓存目录", Category: "java", ExpectedFunc: func() string { return "" }},

	// Android-related
	{Name: "ANDROID_HOME", Description: "Android SDK 安装目录", Category: "android", ExpectedFunc: func() string { return "" }},
	{Name: "ANDROID_SDK_ROOT", Description: "Android SDK 根目录", Category: "android", ExpectedFunc: func() string { return "" }},
	{Name: "ANDROID_NDK_HOME", Description: "Android NDK 安装目录", Category: "android", ExpectedFunc: func() string { return "" }},

	// Node.js-related
	{Name: "NODE_PATH", Description: "Node.js 全局模块路径", Category: "nodejs", ExpectedFunc: func() string { return "" }},
	{Name: "NPM_CONFIG_CACHE", Description: "NPM 缓存目录", Category: "nodejs", ExpectedFunc: func() string { return "" }},
	{Name: "NPM_CONFIG_PREFIX", Description: "NPM 全局安装前缀", Category: "nodejs", ExpectedFunc: func() string { return "" }},

	// Flutter/Dart-related
	{Name: "FLUTTER_HOME", Description: "Flutter SDK 目录", Category: "flutter", ExpectedFunc: func() string { return "" }},
	{Name: "PUB_CACHE", Description: "Dart Pub 缓存目录", Category: "flutter", ExpectedFunc: func() string { return "" }},

	// Rust-related
	{Name: "CARGO_HOME", Description: "Cargo 包管理器目录", Category: "rust", ExpectedFunc: func() string { return "" }},
	{Name: "RUSTUP_HOME", Description: "Rustup 工具链目录", Category: "rust", ExpectedFunc: func() string { return "" }},

	// Python-related
	{Name: "PYTHONPATH", Description: "Python 模块搜索路径", Category: "python", ExpectedFunc: func() string { return "" }},
	{Name: "PIP_CACHE_DIR", Description: "Pip 缓存目录", Category: "python", ExpectedFunc: func() string { return "" }},
	{Name: "PYTHON_HOME", Description: "Python 安装目录", Category: "python", ExpectedFunc: func() string { return "" }},

	// General dev tools
	{Name: "NUGET_PACKAGES", Description: "NuGet 包缓存目录", Category: "general", ExpectedFunc: func() string { return "" }},
}

// RunFullDiagnostics performs a comprehensive system check
func RunFullDiagnostics() ([]DiagnosticCheckItem, error) {
	var checks []DiagnosticCheckItem

	// === Check 1: Installed SDK Versions ===
	checks = append(checks, checkInstalledSDKs())

	// === Check 2: PATH Environment Variable ===
	checks = append(checks, checkPathEnvironment())

	// === Check 3: SDK-related Environment Variables ===
	checks = append(checks, checkDeveloperEnvVars())

	// === Check 4: External Software Conflicts ===
	checks = append(checks, checkExternalSoftware())

	return checks, nil
}

// checkInstalledSDKs verifies which SDKs have versions installed
func checkInstalledSDKs() DiagnosticCheckItem {
	item := DiagnosticCheckItem{
		CheckID:   "installed_sdks",
		CheckName: "本地 SDK 版本安装检测",
		Passed:    true,
	}

	for _, sdk := range sdks {
		sdkDir := filepath.Join(globalConfig.VersionsDir, sdk.Name())
		entries, err := ioutil.ReadDir(sdkDir)
		hasVersions := err == nil && len(entries) > 0

		var installedVersions []string
		var activeVersion string

		if hasVersions {
			for _, entry := range entries {
				if entry.IsDir() {
					installedVersions = append(installedVersions, entry.Name())
				}
			}
			junctionPath := filepath.Join(globalConfig.LinksDir, sdk.Name())
			if target, err := filepath.EvalSymlinks(junctionPath); err == nil {
				targetClean := strings.ToLower(filepath.Clean(target))
				for _, v := range installedVersions {
					vPath := strings.ToLower(filepath.Clean(filepath.Join(sdkDir, v)))
					if vPath == targetClean {
						activeVersion = v
						break
					}
				}
			}
		}

		if !hasVersions || len(installedVersions) == 0 {
			item.Passed = false
			item.Problems = append(item.Problems, DiagnosticProblem{
				ProblemType:   "not_installed",
				ProblemDesc:   fmt.Sprintf("%s：未安装任何版本", sdk.Name()),
				ProblemDetail: fmt.Sprintf("建议安装 %s 以便进行版本管理", sdk.Name()),
				FixType:       "install",
				FixTarget:     sdk.Name(),
			})
		} else {
			detail := fmt.Sprintf("已安装版本: %s", strings.Join(installedVersions, ", "))
			if activeVersion != "" {
				detail += fmt.Sprintf(" | 当前启用: %s", activeVersion)
			}
			// No problem for this SDK - it's fine
			_ = detail
		}
	}

	return item
}

// checkPathEnvironment examines the PATH for duplicates, dead links, and conflicts
func checkPathEnvironment() DiagnosticCheckItem {
	item := DiagnosticCheckItem{
		CheckID:   "path_check",
		CheckName: "PATH 环境变量健康检查",
		Passed:    true,
	}

	currentPath, err := GetUserEnv("PATH")
	if err != nil {
		item.Passed = false
		item.Problems = append(item.Problems, DiagnosticProblem{
			ProblemType:   "read_error",
			ProblemDesc:   "无法读取用户 PATH 环境变量",
			ProblemDetail: err.Error(),
			FixType:       "manual",
			FixTarget:     "PATH",
		})
		return item
	}

	systemPath, _ := GetSystemEnv("PATH")

	pathParts := filepath.SplitList(currentPath)
	seen := make(map[string]int)
	linksDir := strings.ToLower(filepath.Clean(globalConfig.LinksDir))
	avIndices := make(map[string]int)

	for i, part := range pathParts {
		partTrimmed := strings.TrimSpace(part)
		if partTrimmed == "" {
			continue
		}
		partClean := strings.ToLower(filepath.Clean(partTrimmed))

		// 1. Duplicate check
		seen[partClean]++
		if seen[partClean] > 1 {
			item.Passed = false
			item.Problems = append(item.Problems, DiagnosticProblem{
				ProblemType:   "duplicate_path",
				ProblemDesc:   fmt.Sprintf("PATH 中存在重复路径: %s", partTrimmed),
				ProblemDetail: partTrimmed,
				FixType:       "clean_path",
				FixTarget:     partTrimmed,
			})
		}

		// 2. Dead path check (directory doesn't exist)
			isAVPath := strings.Contains(partClean, linksDir)
			if !isAVPath {
				if _, err := os.Stat(partTrimmed); os.IsNotExist(err) {
					item.Passed = false
					item.Problems = append(item.Problems, DiagnosticProblem{
						ProblemType:   "dead_path",
						ProblemDesc:   fmt.Sprintf("PATH 中的路径不存在: %s", partTrimmed),
						ProblemDetail: partTrimmed,
						FixType:       "remove_path",
						FixTarget:     partTrimmed,
					})
				}
			}

		// Track Any-Version link paths
		if strings.Contains(partClean, linksDir) {
			avIndices[partClean] = i
		}
	}

	// 3. External conflicts check
	conflictTools := map[string]string{
		"node.exe":    "nodejs",
		"go.exe":      "go",
		"python.exe":  "python",
		"flutter.bat": "flutter",
		"rustc.exe":   "rust",
		"bun.exe":     "bun",
		"java.exe":    "java",
		"javac.exe":   "java",
	}

	for i, part := range pathParts {
		partTrimmed := strings.TrimSpace(part)
		if partTrimmed == "" {
			continue
		}
		partClean := strings.ToLower(filepath.Clean(partTrimmed))

		if strings.Contains(partClean, linksDir) {
			continue
		}

		for executable, toolName := range conflictTools {
			exePath := filepath.Join(partTrimmed, executable)
			if _, err := os.Stat(exePath); err == nil {
				linkPathKey := strings.ToLower(filepath.Clean(filepath.Join(globalConfig.LinksDir, toolName)))
				if toolName == "go" || toolName == "flutter" || toolName == "rust" || toolName == "java" {
					linkPathKey = strings.ToLower(filepath.Clean(filepath.Join(globalConfig.LinksDir, toolName, "bin")))
				}

				avIdx, avFound := avIndices[linkPathKey]
				precedes := !avFound || i < avIdx

				if precedes {
					item.Passed = false
					item.Problems = append(item.Problems, DiagnosticProblem{
						ProblemType:   "conflict",
						ProblemDesc:   fmt.Sprintf("检测到外部 %s 安装，优先级高于 Any-Version 管理的版本，可能导致版本切换不生效", toolName),
						ProblemDetail: fmt.Sprintf("外部路径: %s", partTrimmed),
						FixType:       "remove_path",
						FixTarget:     partTrimmed,
					})
				}
			}
		}
	}

	// 4. Check if Any-Version paths are NOT at top (optimization suggestion)
	if len(avIndices) > 0 {
		nonAVFirst := -1
		for i, part := range pathParts {
			partClean := strings.ToLower(filepath.Clean(strings.TrimSpace(part)))
			if !strings.Contains(partClean, linksDir) {
				nonAVFirst = i
				break
			}
		}
		if nonAVFirst >= 0 {
			for _, avIdx := range avIndices {
				if avIdx > nonAVFirst {
					item.Problems = append(item.Problems, DiagnosticProblem{
						ProblemType:   "priority",
						ProblemDesc:   "Any-Version 管理的路径未置顶，建议优化 PATH 顺序以保证版本切换优先",
						ProblemDetail: "点击修复将 Any-Version 路径移到 PATH 最前面",
						FixType:       "clean_path",
						FixTarget:     "REORDER_ALL",
					})
					break
				}
			}
		}
	}

	// 5. Check System PATH for potential interference
	if systemPath != "" {
		sysParts := filepath.SplitList(systemPath)
		for _, sp := range sysParts {
			sp = strings.TrimSpace(sp)
			for executable, toolName := range conflictTools {
				exePath := filepath.Join(sp, executable)
				if _, err := os.Stat(exePath); err == nil {
					item.Passed = false
					item.Problems = append(item.Problems, DiagnosticProblem{
						ProblemType:   "system_conflict",
						ProblemDesc:   fmt.Sprintf("系统 PATH 中存在 %s 可执行文件: %s（可能与 Any-Version 冲突）", toolName, sp),
						ProblemDetail: fmt.Sprintf("系统路径: %s", sp),
						FixType:       "manual",
						FixTarget:     sp,
					})
				}
			}
		}
	}

	return item
}

// checkDeveloperEnvVars checks all SDK-related environment variables
func checkDeveloperEnvVars() DiagnosticCheckItem {
	item := DiagnosticCheckItem{
		CheckID:   "dev_env_vars",
		CheckName: "开发环境变量检测",
		Passed:    true,
	}

	linksDir := strings.ToLower(filepath.Clean(globalConfig.LinksDir))

	// Check for dead paths in env var values
	for _, ev := range developerEnvVars {
		val, _ := GetUserEnv(ev.Name)
		if val == "" {
			val, _ = GetSystemEnv(ev.Name)
		}

		if val == "" {
			continue // Not set is fine for optional vars
		}

		// Check if this env var points to a directory that exists
		if isPathLikeEnvVar(ev.Name) {
			valClean := strings.ToLower(filepath.Clean(val))
			isAVVal := strings.Contains(valClean, linksDir)
			if !isAVVal {
				if _, err := os.Stat(val); os.IsNotExist(err) {
					item.Passed = false
					item.Problems = append(item.Problems, DiagnosticProblem{
						ProblemType:   "dead_env_path",
						ProblemDesc:   fmt.Sprintf("环境变量 %s (%s) 指向的路径不存在: %s", ev.Name, ev.Description, val),
						ProblemDetail: fmt.Sprintf("%s=%s", ev.Name, val),
						FixType:       "set_env",
						FixTarget:     ev.Name,
					})
				}
			}
		}
	}

	// Check if there are any SDK-related vars in the user environment
	// that are NOT managed by Any-Version
	allUserEnv := getAllUserEnvVars()
	for envName, envVal := range allUserEnv {
		envUpper := strings.ToUpper(envName)
		if isDevEnvVar(envUpper) && envVal != "" {
			// Check if this env var points to old/non-existent path
			if isPathLikeEnvVar(envName) {
				valClean := strings.ToLower(filepath.Clean(envVal))
				isAVVal := strings.Contains(valClean, linksDir)
				if !isAVVal {
					if _, err := os.Stat(envVal); os.IsNotExist(err) {
						item.Passed = false
						item.Problems = append(item.Problems, DiagnosticProblem{
							ProblemType:   "dead_env_path",
							ProblemDesc:   fmt.Sprintf("环境变量 %s 指向不存在的路径: %s", envName, envVal),
							ProblemDetail: fmt.Sprintf("%s=%s", envName, envVal),
							FixType:       "set_env",
							FixTarget:     envName,
						})
					}
				}
			}
		}
	}

	// Check Java: JAVA_HOME consistency with Any-Version
	javaHome, _ := GetUserEnv("JAVA_HOME")
	if javaHome != "" {
		linksDir := strings.ToLower(filepath.Clean(globalConfig.LinksDir))
		javaLinkPath := strings.ToLower(filepath.Clean(filepath.Join(globalConfig.LinksDir, "java")))
		javaHomeClean := strings.ToLower(filepath.Clean(javaHome))
		if !strings.Contains(javaHomeClean, linksDir) {
			item.Passed = false
			item.Problems = append(item.Problems, DiagnosticProblem{
				ProblemType:   "wrong_env",
				ProblemDesc:   "JAVA_HOME 未指向 Any-Version 管理的 Java 版本",
				ProblemDetail: fmt.Sprintf("当前 JAVA_HOME=%s，建议设为 %s", javaHome, javaLinkPath),
				FixType:       "set_env",
				FixTarget:     "JAVA_HOME",
			})
		}
	}

	// Check Go: GOROOT consistency
	goRoot, _ := GetUserEnv("GOROOT")
	if goRoot != "" {
		linksDir := strings.ToLower(filepath.Clean(globalConfig.LinksDir))
		goLinkPath := strings.ToLower(filepath.Clean(filepath.Join(globalConfig.LinksDir, "go")))
		goRootClean := strings.ToLower(filepath.Clean(goRoot))
		if !strings.Contains(goRootClean, linksDir) {
			item.Passed = false
			item.Problems = append(item.Problems, DiagnosticProblem{
				ProblemType:   "wrong_env",
				ProblemDesc:   "GOROOT 未指向 Any-Version 管理的 Go 版本",
				ProblemDetail: fmt.Sprintf("当前 GOROOT=%s，建议设为 %s", goRoot, goLinkPath),
				FixType:       "set_env",
				FixTarget:     "GOROOT",
			})
		}
	}

	return item
}

// checkExternalSoftware checks Windows registry for conflicting software
func checkExternalSoftware() DiagnosticCheckItem {
	item := DiagnosticCheckItem{
		CheckID:   "external_software",
		CheckName: "外部开发软件冲突检测",
		Passed:    true,
	}

	// Use the existing ScanExternalSoftware from registry
	externalSW := scanExternalSoftware()
	for _, sw := range externalSW {
		item.Passed = false
		item.Problems = append(item.Problems, DiagnosticProblem{
			ProblemType:   "conflict_software",
			ProblemDesc:   fmt.Sprintf("检测到已安装的外部开发软件: %s (版本: %s)，可能与 Any-Version 冲突", sw.DisplayName, sw.DisplayVersion),
			ProblemDetail: fmt.Sprintf("卸载命令: %s | 安装位置: %s", sw.UninstallString, sw.InstallLocation),
			FixType:       "uninstall",
			FixTarget:     sw.UninstallString,
		})
	}

	return item
}

// ExternalSoftwareInfo represents a conflicting software entry from registry
type ExternalSoftwareInfo struct {
	DisplayName     string
	DisplayVersion  string
	UninstallString string
	InstallLocation string
	RegistryKey     string
}

// scanExternalSoftware scans Windows registry for conflicting dev software
func scanExternalSoftware() []ExternalSoftwareInfo {
	var result []ExternalSoftwareInfo

	// Use PowerShell to scan registry
	// Build a clean script without backtick conflicts
	script := "-Command \"$results=@(); " +
		"$keywords=@('node.js','openjdk','adoptium','zulu','java(tm)','jdk','nvm for windows','android studio','android sdk'); " +
		"$paths=@('HKLM:\\\\SOFTWARE\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Uninstall'," +
		"'HKLM:\\\\SOFTWARE\\\\WOW6432Node\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Uninstall'," +
		"'HKCU:\\\\SOFTWARE\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Uninstall'," +
		"'HKCU:\\\\SOFTWARE\\\\WOW6432Node\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Uninstall'); " +
		"foreach($p in $paths){try{Get-ChildItem -Path $p -ErrorAction Stop|ForEach-Object{" +
		"$props=Get-ItemProperty $_.PSPath; $n=$props.DisplayName; if($n){$ln=$n.ToLower(); " +
		"foreach($kw in $keywords){if($ln.Contains($kw)){$results+=[PSCustomObject]@{Name=$n;" +
		"Version=if($props.DisplayVersion){$props.DisplayVersion}else{''};" +
		"Uninstall=if($props.UninstallString){$props.UninstallString}else{''};" +
		"Location=if($props.InstallLocation){$props.InstallLocation}else{''};" +
		"RegKey=$_.PSPath};break}}}}}}catch{}}; " +
		"if($results.Count -gt 0){$results|ConvertTo-Json -Compress}\""

	psCmd := exec.Command("powershell", "-NoProfile", "-NonInteractive", script)
	var stdout, stderr bytes.Buffer
	psCmd.Stdout = &stdout
	psCmd.Stderr = &stderr
	if err := psCmd.Run(); err != nil {
		return result
	}

	output := strings.TrimSpace(stdout.String())
	if output == "" {
		return result
	}

	// Parse JSON array output
	// Format: [{"Name":"...","Version":"...","Uninstall":"...","Location":"...","RegKey":"..."},...]
	lines := strings.Split(output, "},{")
	for _, line := range lines {
		info := ExternalSoftwareInfo{}
		info.DisplayName = extractJSONField(line, "Name")
		info.DisplayVersion = extractJSONField(line, "Version")
		info.UninstallString = extractJSONField(line, "Uninstall")
		info.InstallLocation = extractJSONField(line, "Location")
		info.RegistryKey = extractJSONField(line, "RegKey")
		if info.DisplayName != "" && info.UninstallString != "" {
			result = append(result, info)
		}
	}

	return result
}

// extractJSONField extracts a simple JSON string field value from a JSON line
func extractJSONField(line, fieldName string) string {
	searchKey := "\"" + fieldName + "\":\""
	idx := strings.Index(line, searchKey)
	if idx == -1 {
		return ""
	}
	start := idx + len(searchKey)
	end := strings.Index(line[start:], "\"")
	if end == -1 {
		return ""
	}
	return line[start : start+end]
}

// FixDiagnosticIssue applies a fix for a specific problem
func FixDiagnosticIssue(fixType, fixTarget string) error {
	switch fixType {
	case "clean_path":
		return OptimizeEnvironment()
	case "remove_path":
		return removeSinglePathEntry(fixTarget)
	case "set_env":
		return fixSetEnvVar(fixTarget)
	default:
		return fmt.Errorf("unsupported fix type: %s", fixType)
	}
}

// removeSinglePathEntry removes a specific entry from the user PATH
func removeSinglePathEntry(target string) error {
	currentPath, err := GetUserEnv("PATH")
	if err != nil {
		return err
	}
	targetClean := strings.ToLower(filepath.Clean(target))
	pathParts := filepath.SplitList(currentPath)
	var newParts []string
	for _, p := range pathParts {
		pClean := strings.ToLower(filepath.Clean(strings.TrimSpace(p)))
		if pClean != targetClean {
			newParts = append(newParts, p)
		}
	}
	newPath := strings.Join(newParts, string(filepath.ListSeparator))
	return SetUserEnv("PATH", newPath)
}

// fixSetEnvVar fixes a specific environment variable to point to the right location
func fixSetEnvVar(varName string) error {
	varNameUpper := strings.ToUpper(varName)
	linksDir := globalConfig.LinksDir

	switch varNameUpper {
	case "JAVA_HOME":
		javaHome := filepath.Join(linksDir, "java")
		return SetUserEnv("JAVA_HOME", javaHome)
	case "GOROOT":
		goRoot := filepath.Join(linksDir, "go")
		return SetUserEnv("GOROOT", goRoot)
	case "ANDROID_HOME", "ANDROID_SDK_ROOT":
		androidHome := filepath.Join(linksDir, "android")
		if err := SetUserEnv("ANDROID_HOME", androidHome); err != nil {
			return err
		}
		return SetUserEnv("ANDROID_SDK_ROOT", androidHome)
	default:
		return fmt.Errorf("unsupported env var for auto-fix: %s (请手动修改)", varName)
	}
}

// isPathLikeEnvVar returns true if the env var typically holds a file path
func isPathLikeEnvVar(name string) bool {
	pathLikeVars := map[string]bool{
		"GOROOT": true, "GOPATH": true, "GOCACHE": true, "GOMODCACHE": true, "GOBIN": true,
		"JAVA_HOME": true, "MAVEN_HOME": true, "GRADLE_HOME": true, "GRADLE_USER_HOME": true,
		"ANDROID_HOME": true, "ANDROID_SDK_ROOT": true, "ANDROID_NDK_HOME": true,
		"NODE_PATH": true, "NPM_CONFIG_CACHE": true, "NPM_CONFIG_PREFIX": true,
		"FLUTTER_HOME": true, "PUB_CACHE": true,
		"CARGO_HOME": true, "RUSTUP_HOME": true,
		"PYTHONPATH": true, "PIP_CACHE_DIR": true, "PYTHON_HOME": true,
		"NUGET_PACKAGES": true,
	}
	return pathLikeVars[strings.ToUpper(name)]
}

// isDevEnvVar returns true if the env var is development-related
func isDevEnvVar(name string) bool {
	devPrefixes := []string{"GO", "JAVA", "MAVEN", "GRADLE", "ANDROID", "NODE", "NPM", "FLUTTER", "DART", "CARGO", "RUST", "PYTHON", "PIP", "NUGET"}
	for _, prefix := range devPrefixes {
		if strings.HasPrefix(name, prefix) {
			return true
		}
	}
	// Also check specific vars
	devVars := map[string]bool{
		"CLASSPATH": true, "PUB_CACHE": true, "CHROME_EXECUTABLE": true,
	}
	return devVars[name]
}

// getAllUserEnvVars retrieves all User environment variables via PowerShell
func getAllUserEnvVars() map[string]string {
	result := make(map[string]string)
	cmd := exec.Command("powershell", "-NoProfile", "-NonInteractive", "-Command",
		"[Environment]::GetEnvironmentVariables('User') | ConvertTo-Json -Compress")
	var out bytes.Buffer
	cmd.Stdout = &out
	if err := cmd.Run(); err != nil {
		return result
	}
	output := strings.TrimSpace(out.String())
	if output == "" {
		return result
	}
	// Parse simple JSON object
	output = strings.Trim(output, "{}")
	if output == "" {
		return result
	}
	// Split on comma between key-value pairs, but be careful with quoted values
	parts := strings.Split(output, `","`)
	for _, pair := range parts {
		kv := strings.SplitN(pair, `":"`, 2)
		if len(kv) == 2 {
			key := strings.Trim(kv[0], `"`)
			val := strings.Trim(kv[1], `"`)
			result[key] = val
		}
	}
	return result
}

// GetSystemEnv reads a System-level environment variable
func GetSystemEnv(name string) (string, error) {
	cmdStr := fmt.Sprintf("[Environment]::GetEnvironmentVariable('%s', 'Machine')", name)
	cmd := exec.Command("powershell", "-NoProfile", "-NonInteractive", "-Command", cmdStr)
	var out bytes.Buffer
	cmd.Stdout = &out
	if err := cmd.Run(); err != nil {
		return "", err
	}
	return strings.TrimSpace(out.String()), nil
}

// OptimizeEnvironment cleans PATH by removing duplicate and dead paths, and moves Any-Version link directories to the top
func OptimizeEnvironment() error {
	currentPath, err := GetUserEnv("PATH")
	if err != nil {
		return fmt.Errorf("failed to read User PATH: %v", err)
	}

	pathParts := filepath.SplitList(currentPath)
	linksDir := strings.ToLower(filepath.Clean(globalConfig.LinksDir))

	var cleanParts []string
	var avParts []string
	seen := make(map[string]bool)

	for _, part := range pathParts {
		partTrimmed := strings.TrimSpace(part)
		if partTrimmed == "" {
			continue
		}
		partClean := strings.ToLower(filepath.Clean(partTrimmed))

		// Skip duplicates
		if seen[partClean] {
			continue
		}
		seen[partClean] = true

		// Skip dead paths (keep Any-Version links even if the actual directory is not created yet)
		isAVPath := strings.Contains(partClean, linksDir)
		if !isAVPath {
			if _, err := os.Stat(partTrimmed); os.IsNotExist(err) {
				continue
			}
		}

		// Group Any-Version links separately so they can be placed at the top
		if strings.Contains(partClean, linksDir) {
			avParts = append(avParts, partTrimmed)
		} else {
			cleanParts = append(cleanParts, partTrimmed)
		}
	}

	// Reassemble with Any-Version link paths at the very top
	optimizedParts := append(avParts, cleanParts...)
	newPath := strings.Join(optimizedParts, string(filepath.ListSeparator))

	if err := SetUserEnv("PATH", newPath); err != nil {
		return fmt.Errorf("failed to update User PATH: %v", err)
	}

	// Force environment broadcast
	BroadcastSettingChange()

	return nil
}

