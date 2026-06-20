package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"path/filepath"
	"strings"
)

type GoSDK struct{}

func (g *GoSDK) Name() string {
	return "go"
}

func (g *GoSDK) Category() string {
	return "language"
}

type goRelease struct {
	Version string `json:"version"` // "go1.21.5"
	Stable  bool   `json:"stable"`
}

func (g *GoSDK) ListRemote() ([]string, error) {
	resp, err := http.Get("https://go.dev/dl/?mode=json&include=all")
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("failed to fetch Go releases: %s", resp.Status)
	}

	var releases []goRelease
	if err := json.NewDecoder(resp.Body).Decode(&releases); err != nil {
		return nil, err
	}

	var versions []string
	for _, r := range releases {
		if !r.Stable {
			continue
		}
		// Strip the leading "go"
		v := strings.TrimPrefix(r.Version, "go")
		versions = append(versions, v)
	}

	// Limit list to top 100 versions
	if len(versions) > 100 {
		versions = versions[:100]
	}

	return versions, nil
}

func (g *GoSDK) Install(version string, baseDir string) error {
	version = strings.TrimPrefix(strings.TrimSpace(version), "go")

	// Construct download URL
	// E.g. https://go.dev/dl/go1.22.0.windows-amd64.zip
	downloadURL := fmt.Sprintf("https://go.dev/dl/go%s.windows-amd64.zip", version)
	fmt.Printf("正在从 %s 下载 Go v%s...\n", downloadURL, version)

	tempDir, cleanup, err := SetupTempDir(baseDir, "go")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "go.zip")
	if err := DownloadFile(downloadURL, zipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 Go 压缩包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, g.Name(), version)
	fmt.Printf("正在安装 Go 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	fmt.Printf("Go v%s 安装成功！\n", version)
	return nil
}
