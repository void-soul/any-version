package main

import (
	"fmt"
	"io/ioutil"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type MirrorInfo struct {
	Tool        string `json:"tool"`
	Current     string `json:"current"`
	MirrorName  string `json:"mirror_name"` // e.g. "Aliyun", "Tsinghua", "Official"
}

func GetMirrorsList() []MirrorInfo {
	userProfile := os.Getenv("USERPROFILE")
	appData := os.Getenv("APPDATA")

	// 1. npm
	npmReg := getCmdOutput("cmd", "/c", "npm", "config", "get", "registry")
	if npmReg == "" {
		npmReg = "https://registry.npmjs.org/"
	}
	npmName := classifyMirror(npmReg, "npm")

	// 2. pip
	pipReg := ""
	pipIni := filepath.Join(appData, "pip", "pip.ini")
	if data, err := ioutil.ReadFile(pipIni); err == nil {
		for _, line := range strings.Split(string(data), "\n") {
			line = strings.TrimSpace(line)
			if strings.HasPrefix(strings.ToLower(line), "index-url") {
				parts := strings.SplitN(line, "=", 2)
				if len(parts) == 2 {
					pipReg = strings.TrimSpace(parts[1])
					break
				}
			}
		}
	}
	if pipReg == "" {
		pipReg = "https://pypi.org/simple"
	}
	pipName := classifyMirror(pipReg, "pip")

	// 3. maven
	mvnReg := "https://repo.maven.apache.org/maven2"
	m2Settings := filepath.Join(userProfile, ".m2", "settings.xml")
	if data, err := ioutil.ReadFile(m2Settings); err == nil {
		content := string(data)
		if strings.Contains(content, "maven.aliyun.com") {
			mvnReg = "https://maven.aliyun.com/repository/public"
		}
	}
	mvnName := classifyMirror(mvnReg, "maven")

	// 4. go
	goProxy := getCmdOutput("cmd", "/c", "go", "env", "GOPROXY")
	if goProxy == "" {
		goProxy = "https://proxy.golang.org,direct"
	}
	goName := classifyMirror(goProxy, "go")

	// 5. rust
	rustReg := "https://github.com/rust-lang/crates.io-index"
	cargoConfig := filepath.Join(userProfile, ".cargo", "config.toml")
	if _, err := os.Stat(cargoConfig); os.IsNotExist(err) {
		cargoConfig = filepath.Join(userProfile, ".cargo", "config")
	}
	if data, err := ioutil.ReadFile(cargoConfig); err == nil {
		content := string(data)
		if strings.Contains(content, "rsproxy.cn") {
			rustReg = "https://rsproxy.cn"
		} else if strings.Contains(content, "ustc.edu.cn") {
			rustReg = "https://mirrors.ustc.edu.cn/crates.io-index"
		}
	}
	rustName := classifyMirror(rustReg, "rust")

	return []MirrorInfo{
		{"npm", npmReg, npmName},
		{"pip", pipReg, pipName},
		{"maven", mvnReg, mvnName},
		{"go", goProxy, goName},
		{"rust", rustReg, rustName},
	}
}

func classifyMirror(urlStr, tool string) string {
	urlStr = strings.ToLower(urlStr)
	if strings.Contains(urlStr, "npmmirror.com") || strings.Contains(urlStr, "aliyun.com") || strings.Contains(urlStr, "taobao.org") {
		return "Aliyun / Taobao"
	}
	if strings.Contains(urlStr, "tsinghua.edu.cn") {
		return "Tsinghua"
	}
	if strings.Contains(urlStr, "tencent.com") || strings.Contains(urlStr, "tencentcloud") {
		return "Tencent"
	}
	if strings.Contains(urlStr, "rsproxy.cn") {
		return "Rsproxy"
	}
	if strings.Contains(urlStr, "npmjs.org") || strings.Contains(urlStr, "pypi.org") || strings.Contains(urlStr, "golang.org") || strings.Contains(urlStr, "crates.io-index") || strings.Contains(urlStr, "maven.org") || strings.Contains(urlStr, "apache.org") {
		return "Official"
	}
	return "Custom"
}

func SetMirror(tool, mirrorType string) error {
	userProfile := os.Getenv("USERPROFILE")
	appData := os.Getenv("APPDATA")

	mirrorType = strings.ToLower(mirrorType)

	switch strings.ToLower(tool) {
	case "npm":
		urlVal := "https://registry.npmjs.org/"
		if mirrorType == "aliyun" {
			urlVal = "https://registry.npmmirror.com/"
		} else if mirrorType == "tencent" {
			urlVal = "https://mirrors.cloud.tencent.com/npm/"
		}
		
		// Set NPM
		_ = exec.Command("cmd", "/c", "npm", "config", "set", "registry", urlVal).Run()
		// Set Yarn if installed
		if isInstalled("yarn") {
			_ = exec.Command("cmd", "/c", "yarn", "config", "set", "registry", urlVal).Run()
		}
		// Set Pnpm if installed
		if isInstalled("pnpm") {
			_ = exec.Command("cmd", "/c", "pnpm", "config", "set", "registry", urlVal).Run()
		}

	case "pip":
		urlVal := "https://pypi.org/simple"
		hostVal := "pypi.org"
		if mirrorType == "aliyun" {
			urlVal = "https://mirrors.aliyun.com/pypi/simple/"
			hostVal = "mirrors.aliyun.com"
		} else if mirrorType == "tsinghua" {
			urlVal = "https://pypi.tuna.tsinghua.edu.cn/simple"
			hostVal = "pypi.tuna.tsinghua.edu.cn"
		}
		
		pipIni := filepath.Join(appData, "pip", "pip.ini")
		os.MkdirAll(filepath.Dir(pipIni), 0755)
		content := fmt.Sprintf("[global]\nindex-url = %s\ntrusted-host = %s\n", urlVal, hostVal)
		if mirrorType == "official" {
			_ = os.Remove(pipIni)
		} else {
			_ = ioutil.WriteFile(pipIni, []byte(content), 0644)
		}

	case "maven":
		m2Settings := filepath.Join(userProfile, ".m2", "settings.xml")
		if mirrorType == "official" {
			_ = os.Remove(m2Settings)
		} else if mirrorType == "aliyun" {
			os.MkdirAll(filepath.Dir(m2Settings), 0755)
			xmlContent := `<?xml version="1.0" encoding="UTF-8"?>
<settings xmlns="http://maven.apache.org/SETTINGS/1.0.0"
          xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
          xsi:schemaLocation="http://maven.apache.org/SETTINGS/1.0.0 https://maven.apache.org/xsd/settings-1.0.0.xsd">
  <mirrors>
    <mirror>
      <id>aliyunmaven</id>
      <mirrorOf>central</mirrorOf>
      <name>aliyun maven</name>
      <url>https://maven.aliyun.com/repository/public</url>
    </mirror>
  </mirrors>
</settings>`
			_ = ioutil.WriteFile(m2Settings, []byte(xmlContent), 0644)
		}

	case "go":
		urlVal := "https://proxy.golang.org,direct"
		if mirrorType == "aliyun" {
			urlVal = "https://mirrors.aliyun.com/goproxy/,direct"
		} else if mirrorType == "goproxy" || mirrorType == "tsinghua" {
			urlVal = "https://goproxy.cn,direct"
		}
		
		if isInstalled("go") {
			_ = exec.Command("cmd", "/c", "go", "env", "-w", "GOPROXY="+urlVal).Run()
		}

	case "rust":
		cargoConfig := filepath.Join(userProfile, ".cargo", "config.toml")
		if mirrorType == "official" {
			_ = os.Remove(cargoConfig)
			_ = os.Remove(filepath.Join(userProfile, ".cargo", "config"))
		} else {
			os.MkdirAll(filepath.Dir(cargoConfig), 0755)
			var configContent string
			if mirrorType == "rsproxy" {
				configContent = `[source.crates-io]
replace-with = 'rsproxy'

[source.rsproxy]
registry = "https://rsproxy.cn/crates.io-index"

[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"

[net]
git-fetch-with-cli = true
`
			} else if mirrorType == "tsinghua" {
				configContent = `[source.crates-io]
replace-with = 'tsinghua'

[source.tsinghua]
registry = "https://mirrors.tuna.tsinghua.edu.cn/git/crates.io-index"
`
			}
			_ = ioutil.WriteFile(cargoConfig, []byte(configContent), 0644)
		}

	default:
		return fmt.Errorf("unknown tool: %s", tool)
	}

	return nil
}
