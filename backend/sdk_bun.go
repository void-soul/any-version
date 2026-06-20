package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"path/filepath"
	"strings"
)

type BunSDK struct{}

func (b *BunSDK) Name() string {
	return "bun"
}

func (b *BunSDK) Category() string {
	return "language"
}

type githubRelease struct {
	TagName string `json:"tag_name"`
}

func (b *BunSDK) ListRemote() ([]string, error) {
	req, err := http.NewRequest("GET", "https://api.github.com/repos/oven-sh/bun/releases", nil)
	if err != nil {
		return nil, err
	}
	// Add user-agent to avoid GitHub blocking API requests
	req.Header.Set("User-Agent", "Any-Version-Manager")

	resp, clientErr := http.DefaultClient.Do(req)
	if clientErr != nil {
		return nil, clientErr
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("failed to fetch Bun releases: %s (API rate limit exceeded?)", resp.Status)
	}

	var releases []githubRelease
	if err := json.NewDecoder(resp.Body).Decode(&releases); err != nil {
		return nil, err
	}

	var versions []string
	for _, r := range releases {
		v := r.TagName
		v = strings.TrimPrefix(v, "bun-v")
		v = strings.TrimPrefix(v, "v")
		versions = append(versions, v)
	}

	return versions, nil
}

func (b *BunSDK) Install(version string, baseDir string) error {
	version = strings.TrimPrefix(strings.TrimSpace(version), "v")

	// Construct download URL
	// E.g. https://github.com/oven-sh/bun/releases/download/bun-v1.1.0/bun-windows-x64.zip
	downloadURL := fmt.Sprintf("https://github.com/oven-sh/bun/releases/download/bun-v%s/bun-windows-x64.zip", version)
	fmt.Printf("正在从 %s 下载 Bun v%s...\n", downloadURL, version)

	tempDir, cleanup, err := SetupTempDir(baseDir, "bun")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "bun.zip")
	if err := DownloadFile(downloadURL, zipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 Bun 压缩包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, b.Name(), version)
	fmt.Printf("正在安装 Bun 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	fmt.Printf("Bun v%s 安装成功！\n", version)
	return nil
}
