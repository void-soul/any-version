package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type PackageInfo struct {
	Name           string
	CurrentVersion string
	LatestVersion  string
	Status         string // "latest" or "outdated"
}

// GetGlobalPackages lists all globally installed packages and checks for updates
func GetGlobalPackages(sdkName string) ([]PackageInfo, error) {
	sdkName = strings.ToLower(strings.TrimSpace(sdkName))
	if sdkName == "nodejs" || sdkName == "npm" {
		return getGlobalNpmPackages()
	} else if sdkName == "python" || sdkName == "pip" {
		return getGlobalPipPackages()
	}
	return nil, fmt.Errorf("不支持的包管理器：%s。目前仅支持 nodejs(npm) 和 python(pip)", sdkName)
}

// UpgradeGlobalPackage upgrades a specific global package to its latest version
func UpgradeGlobalPackage(sdkName, pkgName string) error {
	sdkName = strings.ToLower(strings.TrimSpace(sdkName))
	pkgName = strings.TrimSpace(pkgName)
	if pkgName == "" {
		return fmt.Errorf("包名称不能为空")
	}

	var cmd *exec.Cmd
	if sdkName == "nodejs" || sdkName == "npm" {
		npmPath := "npm"
		activeNpm := filepath.Join(globalConfig.LinksDir, "nodejs", "npm.cmd")
		if _, err := os.Stat(activeNpm); err == nil {
			npmPath = activeNpm
		}
		cmd = exec.Command(npmPath, "install", "-g", pkgName+"@latest")
	} else if sdkName == "python" || sdkName == "pip" {
		pythonPath := "python"
		activePython := filepath.Join(globalConfig.LinksDir, "python", "python.exe")
		if _, err := os.Stat(activePython); err == nil {
			pythonPath = activePython
		}
		cmd = exec.Command(pythonPath, "-m", "pip", "install", "--upgrade", pkgName)
	} else {
		return fmt.Errorf("不支持的包管理器：%s", sdkName)
	}

	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	if output, err := cmd.Output(); err != nil {
		return fmt.Errorf("升级失败: %v (错误信息: %s, 输出: %s)", err, strings.TrimSpace(stderr.String()), string(output))
	}
	return nil
}

// --- NPM NPM packages querying ---

type NpmList struct {
	Dependencies map[string]struct {
		Version string `json:"version"`
	} `json:"dependencies"`
}

type NpmOutdated map[string]struct {
	Current string `json:"current"`
	Latest  string `json:"latest"`
}

func getGlobalNpmPackages() ([]PackageInfo, error) {
	npmPath := "npm"
	activeNpm := filepath.Join(globalConfig.LinksDir, "nodejs", "npm.cmd")
	if _, err := os.Stat(activeNpm); err == nil {
		npmPath = activeNpm
	}

	// 1. Run npm list -g --depth=0 --json
	listCmd := exec.Command(npmPath, "list", "-g", "--depth=0", "--json")
	var listOut bytes.Buffer
	listCmd.Stdout = &listOut
	// npm list may exit with non-zero code if there are issues, but it still writes valid JSON
	_ = listCmd.Run()

	var listData NpmList
	if err := json.Unmarshal(listOut.Bytes(), &listData); err != nil {
		// Fallback to plain command detection or return empty list
		if listOut.Len() == 0 {
			return nil, fmt.Errorf("无法获取 npm 全局包列表，请确认 Node.js 已安装且 npm 可用")
		}
		// Try parsing from raw text if json was slightly malformed due to warnings
		jsonStart := bytes.Index(listOut.Bytes(), []byte("{"))
		if jsonStart != -1 {
			if err := json.Unmarshal(listOut.Bytes()[jsonStart:], &listData); err != nil {
				return nil, fmt.Errorf("解析 npm 列表 JSON 失败: %v", err)
			}
		} else {
			return nil, fmt.Errorf("未找到 npm 列表 JSON 输出: %v", err)
		}
	}

	// 2. Run npm outdated -g --json
	outdatedCmd := exec.Command(npmPath, "outdated", "-g", "--json")
	var outdatedOut bytes.Buffer
	outdatedCmd.Stdout = &outdatedOut
	// npm outdated returns exit code 1 if packages are outdated, which is expected
	_ = outdatedCmd.Run()

	var outdatedData NpmOutdated
	if outdatedOut.Len() > 0 {
		jsonStart := bytes.Index(outdatedOut.Bytes(), []byte("{"))
		if jsonStart != -1 {
			_ = json.Unmarshal(outdatedOut.Bytes()[jsonStart:], &outdatedData)
		}
	}

	var list []PackageInfo
	for name, dep := range listData.Dependencies {
		current := dep.Version
		latest := current
		status := "latest"

		if outInfo, isOutdated := outdatedData[name]; isOutdated {
			latest = outInfo.Latest
			status = "outdated"
		}

		list = append(list, PackageInfo{
			Name:           name,
			CurrentVersion: current,
			LatestVersion:  latest,
			Status:         status,
		})
	}

	return list, nil
}

// --- Python PIP packages querying ---

type PipPackage struct {
	Name    string `json:"name"`
	Version string `json:"version"`
}

type PipOutdated struct {
	Name          string `json:"name"`
	Version       string `json:"version"`
	LatestVersion string `json:"latest_version"`
}

func getGlobalPipPackages() ([]PackageInfo, error) {
	pythonPath := "python"
	activePython := filepath.Join(globalConfig.LinksDir, "python", "python.exe")
	if _, err := os.Stat(activePython); err == nil {
		pythonPath = activePython
	}

	// 1. Run python -m pip list --format=json
	listCmd := exec.Command(pythonPath, "-m", "pip", "list", "--format=json")
	var listOut bytes.Buffer
	listCmd.Stdout = &listOut
	if err := listCmd.Run(); err != nil {
		return nil, fmt.Errorf("运行 pip list 失败: %v。请确认 Python 已安装且 pip 可用", err)
	}

	var pkgs []PipPackage
	if err := json.Unmarshal(listOut.Bytes(), &pkgs); err != nil {
		return nil, fmt.Errorf("解析 pip list 失败: %v", err)
	}

	// 2. Run python -m pip list --outdated --format=json
	outdatedCmd := exec.Command(pythonPath, "-m", "pip", "list", "--outdated", "--format=json")
	var outdatedOut bytes.Buffer
	outdatedCmd.Stdout = &outdatedOut
	_ = outdatedCmd.Run()

	var outdatedPkgs []PipOutdated
	if outdatedOut.Len() > 0 {
		_ = json.Unmarshal(outdatedOut.Bytes(), &outdatedPkgs)
	}

	outdatedMap := make(map[string]string)
	for _, op := range outdatedPkgs {
		outdatedMap[strings.ToLower(op.Name)] = op.LatestVersion
	}

	var list []PackageInfo
	for _, p := range pkgs {
		current := p.Version
		latest := current
		status := "latest"

		if lv, isOutdated := outdatedMap[strings.ToLower(p.Name)]; isOutdated {
			latest = lv
			status = "outdated"
		}

		list = append(list, PackageInfo{
			Name:           p.Name,
			CurrentVersion: current,
			LatestVersion:  latest,
			Status:         status,
		})
	}

	return list, nil
}
