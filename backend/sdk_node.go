package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"path/filepath"
	"strings"
)

type NodeJS struct{}

func (n *NodeJS) Name() string {
	return "nodejs"
}

func (n *NodeJS) Category() string {
	return "language"
}

type nodeRelease struct {
	Version string `json:"version"`
	Lts     interface{} `json:"lts"` // Can be bool or string (LTS name)
}

func (n *NodeJS) ListRemote() ([]string, error) {
	resp, err := http.Get("https://nodejs.org/dist/index.json")
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("failed to fetch Node.js releases: %s", resp.Status)
	}

	var releases []nodeRelease
	if err := json.NewDecoder(resp.Body).Decode(&releases); err != nil {
		return nil, err
	}

	var versions []string
	for _, r := range releases {
		// Strip the leading 'v'
		v := strings.TrimPrefix(r.Version, "v")
		
		// Filter out versions older than v10
		parts := strings.Split(v, ".")
		if len(parts) > 0 {
			var major int
			if _, err := fmt.Sscanf(parts[0], "%d", &major); err == nil && major < 10 {
				continue
			}
		}
		
		// Optional: add a label if it is LTS
		ltsLabel := ""
		if isLts, ok := r.Lts.(bool); ok && isLts {
			ltsLabel = " (LTS)"
		} else if ltsStr, ok := r.Lts.(string); ok && ltsStr != "" {
			ltsLabel = fmt.Sprintf(" (LTS: %s)", ltsStr)
		}
		
		versions = append(versions, v+ltsLabel)
	}

	return versions, nil
}

func (n *NodeJS) Install(version string, baseDir string) error {
	// Clean version string (ensure we use clean SemVer, but strip leading 'v' if present)
	version = strings.TrimPrefix(strings.TrimSpace(version), "v")

	// Construct download URL
	// E.g. https://nodejs.org/dist/v20.11.0/node-v20.11.0-win-x64.zip
	downloadURL := fmt.Sprintf("https://nodejs.org/dist/v%s/node-v%s-win-x64.zip", version, version)
	fmt.Printf("正在从 %s 下载 Node.js v%s...\n", downloadURL, version)

	// Setup temp directory
	tempDir, cleanup, err := SetupTempDir(baseDir, "nodejs")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "node.zip")
	if err := DownloadFile(downloadURL, zipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 Node.js 压缩包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, n.Name(), version)
	fmt.Printf("正在安装 Node.js 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	fmt.Printf("Node.js v%s 安装成功！\n", version)
	return nil
}
