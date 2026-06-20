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

type CacheInfo struct {
	Name       string `json:"name"`
	Installed  bool   `json:"installed"`
	Path       string `json:"path"`
	Size       string `json:"size"`
	IsLink     bool   `json:"is_link"`
	RealTarget string `json:"real_target"`
}

func getCmdOutput(name string, args ...string) string {
	cmd := exec.Command(name, args...)
	var out bytes.Buffer
	cmd.Stdout = &out
	if err := cmd.Run(); err == nil {
		return strings.TrimSpace(out.String())
	}
	return ""
}

func isInstalled(cli string) bool {
	cmd := exec.Command("cmd", "/c", "where", cli)
	if err := cmd.Run(); err == nil {
		return true
	}
	return false
}

func getDirSize(path string) string {
	if path == "" {
		return "0 B"
	}
	path = os.ExpandEnv(path)
	var size int64
	err := filepath.Walk(path, func(_ string, info os.FileInfo, err error) error {
		if err != nil {
			return nil
		}
		if !info.IsDir() {
			size += info.Size()
		}
		return nil
	})
	if err != nil || size == 0 {
		if _, err := os.Stat(path); os.IsNotExist(err) {
			return "0 B"
		}
	}
	return formatBytes(uint64(size))
}

// GetCachesList detects active development package managers, their cache directories and sizes
func GetCachesList() []CacheInfo {
	userProfile := os.Getenv("USERPROFILE")
	localAppData := os.Getenv("LOCALAPPDATA")
	appData := os.Getenv("APPDATA")

	// 1. npm
	npmInstalled := isInstalled("npm")
	npmPath := getCmdOutput("cmd", "/c", "npm", "config", "get", "cache")
	if npmPath == "" {
		npmPath = filepath.Join(localAppData, "npm-cache")
	}

	// 2. yarn
	yarnInstalled := isInstalled("yarn")
	yarnPath := getCmdOutput("cmd", "/c", "yarn", "cache", "dir")
	if yarnPath == "" {
		yarnPath = filepath.Join(localAppData, "Yarn", "Cache")
	}

	// 3. pnpm
	pnpmInstalled := isInstalled("pnpm")
	pnpmPath := getCmdOutput("cmd", "/c", "pnpm", "store", "path")
	if pnpmPath == "" {
		pnpmPath = filepath.Join(localAppData, "pnpm", "store")
	}

	// 4. pip
	pipInstalled := isInstalled("pip")
	pipPath := os.Getenv("PIP_CACHE_DIR")
	if pipPath == "" {
		// Try reading pip.ini
		pipIni := filepath.Join(appData, "pip", "pip.ini")
		if data, err := ioutil.ReadFile(pipIni); err == nil {
			for _, line := range strings.Split(string(data), "\n") {
				line = strings.TrimSpace(line)
				if strings.HasPrefix(strings.ToLower(line), "cache-dir") {
					parts := strings.SplitN(line, "=", 2)
					if len(parts) == 2 {
						pipPath = strings.TrimSpace(parts[1])
						break
					}
				}
			}
		}
	}
	if pipPath == "" {
		pipPath = filepath.Join(localAppData, "pip", "Cache")
	}

	// 5. Maven (mvn)
	mvnInstalled := isInstalled("mvn")
	var mvnPath string
	m2Settings := filepath.Join(userProfile, ".m2", "settings.xml")
	if data, err := ioutil.ReadFile(m2Settings); err == nil {
		content := string(data)
		start := strings.Index(content, "<localRepository>")
		end := strings.Index(content, "</localRepository>")
		if start != -1 && end != -1 && end > start {
			mvnPath = content[start+17 : end]
		}
	}
	if mvnPath == "" {
		mvnPath = filepath.Join(userProfile, ".m2", "repository")
	}

	// 6. .NET (nuget)
	nugetInstalled := isInstalled("dotnet") || isInstalled("nuget")
	nugetPath := os.Getenv("NUGET_PACKAGES")
	if nugetPath == "" {
		nugetPath = filepath.Join(userProfile, ".nuget", "packages")
	}

	rawCaches := []struct {
		name      string
		installed bool
		path      string
	}{
		{"npm", npmInstalled, npmPath},
		{"yarn", yarnInstalled, yarnPath},
		{"pnpm", pnpmInstalled, pnpmPath},
		{"pip", pipInstalled, pipPath},
		{"mvn", mvnInstalled, mvnPath},
		{"nuget", nugetInstalled, nugetPath},
	}

	var list []CacheInfo
	for _, rc := range rawCaches {
		cleanPath := filepath.Clean(rc.path)
		isLink := false
		realTarget := ""
		evalPath, err := filepath.EvalSymlinks(cleanPath)
		if err == nil {
			evalClean := filepath.Clean(evalPath)
			if !strings.EqualFold(evalClean, cleanPath) {
				isLink = true
				realTarget = evalClean
			}
		}

		sizePath := cleanPath
		if isLink {
			sizePath = realTarget
		}

		list = append(list, CacheInfo{
			Name:       rc.name,
			Installed:  rc.installed,
			Path:       cleanPath,
			Size:       getDirSize(sizePath),
			IsLink:     isLink,
			RealTarget: realTarget,
		})
	}

	return list
}

// UpdateCachePath migrates the cache using directory junctions
func UpdateCachePath(name, newPath string) error {
	newPath = filepath.Clean(newPath)

	userProfile := os.Getenv("USERPROFILE")
	localAppData := os.Getenv("LOCALAPPDATA")
	appData := os.Getenv("APPDATA")

	var origPath string
	switch strings.ToLower(name) {
	case "npm":
		origPath = getCmdOutput("cmd", "/c", "npm", "config", "get", "cache")
		if origPath == "" {
			origPath = filepath.Join(localAppData, "npm-cache")
		}
	case "yarn":
		origPath = getCmdOutput("cmd", "/c", "yarn", "cache", "dir")
		if origPath == "" {
			origPath = filepath.Join(localAppData, "Yarn", "Cache")
		}
	case "pnpm":
		origPath = getCmdOutput("cmd", "/c", "pnpm", "store", "path")
		if origPath == "" {
			origPath = filepath.Join(localAppData, "pnpm", "store")
		}
	case "pip":
		origPath = os.Getenv("PIP_CACHE_DIR")
		if origPath == "" {
			pipIni := filepath.Join(appData, "pip", "pip.ini")
			if data, err := ioutil.ReadFile(pipIni); err == nil {
				for _, line := range strings.Split(string(data), "\n") {
					line = strings.TrimSpace(line)
					if strings.HasPrefix(strings.ToLower(line), "cache-dir") {
						parts := strings.SplitN(line, "=", 2)
						if len(parts) == 2 {
							origPath = strings.TrimSpace(parts[1])
							break
						}
					}
				}
			}
		}
		if origPath == "" {
			origPath = filepath.Join(localAppData, "pip", "Cache")
		}
	case "mvn":
		m2Settings := filepath.Join(userProfile, ".m2", "settings.xml")
		if data, err := ioutil.ReadFile(m2Settings); err == nil {
			content := string(data)
			start := strings.Index(content, "<localRepository>")
			end := strings.Index(content, "</localRepository>")
			if start != -1 && end != -1 && end > start {
				origPath = content[start+17 : end]
			}
		}
		if origPath == "" {
			origPath = filepath.Join(userProfile, ".m2", "repository")
		}
	case "nuget":
		origPath = os.Getenv("NUGET_PACKAGES")
		if origPath == "" {
			origPath = filepath.Join(userProfile, ".nuget", "packages")
		}
	default:
		return fmt.Errorf("unknown package manager: %s", name)
	}

	origPath = filepath.Clean(origPath)
	if strings.EqualFold(origPath, newPath) {
		return fmt.Errorf("原路径与目标路径相同，无需迁移")
	}

	// 1. Ensure target directory exists
	if err := os.MkdirAll(newPath, 0755); err != nil {
		return fmt.Errorf("无法创建目标缓存目录: %v", err)
	}

	// 2. Check if origPath is already a junction
	evalPath, err := filepath.EvalSymlinks(origPath)
	isJunction := false
	if err == nil {
		if !strings.EqualFold(filepath.Clean(evalPath), origPath) {
			isJunction = true
		}
	}

	if isJunction {
		// If it's already a junction, we just remove the old junction first
		if err := os.Remove(origPath); err != nil {
			return fmt.Errorf("无法移除已有的旧链接: %v", err)
		}
	} else {
		// If it's a normal directory and exists, copy contents and remove it
		if _, err := os.Stat(origPath); err == nil {
			fmt.Printf("正在迁移缓存文件从 %s 到 %s...\n", origPath, newPath)
			if err := CopyDir(origPath, newPath); err != nil {
				return fmt.Errorf("复制缓存文件失败: %v", err)
			}
			if err := os.RemoveAll(origPath); err != nil {
				return fmt.Errorf("清空原缓存目录失败（可能部分文件被占用，请手动删除并重试）: %v", err)
			}
		}
	}

	// 3. Create Windows Directory Junction
	if err := CreateJunction(origPath, newPath); err != nil {
		return fmt.Errorf("创建目录联接失败: %v", err)
	}

	return nil
}
