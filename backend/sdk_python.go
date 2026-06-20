package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type PythonSDK struct{}

func (p *PythonSDK) Name() string {
	return "python"
}

func (p *PythonSDK) Category() string {
	return "language"
}

type nugetVersions struct {
	Versions []string `json:"versions"`
}

func (p *PythonSDK) ListRemote() ([]string, error) {
	resp, err := http.Get("https://api.nuget.org/v3-flatcontainer/python/index.json")
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("failed to fetch Python releases from NuGet: %s", resp.Status)
	}

	var data nugetVersions
	if err := json.NewDecoder(resp.Body).Decode(&data); err != nil {
		return nil, err
	}

	// Filter out pre-releases (e.g. contain "-") and reverse list for descending order
	var versions []string
	for i := len(data.Versions) - 1; i >= 0; i-- {
		v := data.Versions[i]
		if !strings.Contains(v, "-") {
			versions = append(versions, v)
		}
	}

	// Limit list to top 120 versions
	if len(versions) > 120 {
		versions = versions[:120]
	}

	return versions, nil
}

func (p *PythonSDK) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)

	// Construct download URL using NuGet API
	// E.g. https://www.nuget.org/api/v2/package/python/3.11.4
	downloadURL := fmt.Sprintf("https://www.nuget.org/api/v2/package/python/%s", version)
	fmt.Printf("正在从 NuGet %s 下载 Python v%s...\n", downloadURL, version)

	tempDir, cleanup, err := SetupTempDir(baseDir, "python")
	if err != nil {
		return err
	}
	defer cleanup()

	nupkgFile := filepath.Join(tempDir, "python.nupkg")
	if err := DownloadFile(downloadURL, nupkgFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 Python 包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(nupkgFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	// NuGet Python packages put all Python files in the "tools" directory.
	toolsDir := filepath.Join(extractDir, "tools")
	if _, err := os.Stat(toolsDir); os.IsNotExist(err) {
		return fmt.Errorf("invalid NuGet package: 'tools' folder not found")
	}

	destDir := filepath.Join(globalConfig.VersionsDir, p.Name(), version)
	fmt.Printf("正在安装 Python 到 %s...\n", destDir)
	if err := MoveExtractToDest(toolsDir, destDir); err != nil {
		return err
	}

	// Post-install: Bootstrap pip if it is not present in the destination
	pythonExe := filepath.Join(destDir, "python.exe")
	fmt.Println("正在检测是否已安装 pip...")
	pipCheckCmd := exec.Command(pythonExe, "-m", "pip", "--version")
	if err := pipCheckCmd.Run(); err != nil {
		fmt.Println("未检测到 pip。正在引导安装 pip...")
		getPipURL := "https://bootstrap.pypa.io/get-pip.py"
		getPipPath := filepath.Join(tempDir, "get-pip.py")
		if err := DownloadFile(getPipURL, getPipPath); err != nil {
			fmt.Printf("警告: 下载 get-pip.py 失败: %v。未安装 pip。\n", err)
		} else {
			// Run python.exe get-pip.py --no-warn-script-location
			bootstrapCmd := exec.Command(pythonExe, getPipPath, "--no-warn-script-location")
			bootstrapCmd.Stdout = os.Stdout
			bootstrapCmd.Stderr = os.Stderr
			if err := bootstrapCmd.Run(); err != nil {
				fmt.Printf("警告: 引导安装 pip 失败: %v\n", err)
			} else {
				fmt.Println("pip 引导安装成功！")
			}
		}
	} else {
		fmt.Println("pip 已经安装。")
	}

	fmt.Printf("Python v%s 安装成功！\n", version)
	return nil
}
