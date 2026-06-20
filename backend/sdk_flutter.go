package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"path/filepath"
	"strings"
)

type FlutterSDK struct{}

func (f *FlutterSDK) Name() string {
	return "flutter"
}

func (f *FlutterSDK) Category() string {
	return "language"
}

type flutterRelease struct {
	Hash     string `json:"hash"`
	Channel  string `json:"channel"`
	Version  string `json:"version"`
	Archive  string `json:"archive"`
}

type flutterManifest struct {
	BaseURL  string           `json:"base_url"`
	Releases []flutterRelease `json:"releases"`
}

func (f *FlutterSDK) getManifest() (*flutterManifest, error) {
	resp, err := http.Get("https://storage.googleapis.com/flutter_infra_release/releases/releases_windows.json")
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("failed to fetch Flutter release list: %s", resp.Status)
	}

	var manifest flutterManifest
	if err := json.NewDecoder(resp.Body).Decode(&manifest); err != nil {
		return nil, err
	}
	return &manifest, nil
}

func (f *FlutterSDK) ListRemote() ([]string, error) {
	manifest, err := f.getManifest()
	if err != nil {
		return nil, err
	}

	var versions []string
	seen := make(map[string]bool)
	for _, r := range manifest.Releases {
		// Only list stable releases to keep it clean, and avoid duplicate versions if any
		if r.Channel == "stable" && !seen[r.Version] {
			seen[r.Version] = true
			versions = append(versions, r.Version)
		}
	}

	// Limit to top 100 versions
	if len(versions) > 100 {
		versions = versions[:100]
	}

	return versions, nil
}

func (f *FlutterSDK) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)

	manifest, err := f.getManifest()
	if err != nil {
		return err
	}

	// Find the archive path for the specified version
	var archivePath string
	for _, r := range manifest.Releases {
		if r.Version == version {
			archivePath = r.Archive
			break
		}
	}

	if archivePath == "" {
		return fmt.Errorf("flutter version %s not found in stable releases. Use 'list-remote flutter' to see available versions", version)
	}

	downloadURL := manifest.BaseURL + "/" + archivePath
	fmt.Printf("正在从 %s 下载 Flutter v%s...\n", downloadURL, version)

	tempDir, cleanup, err := SetupTempDir(baseDir, "flutter")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "flutter.zip")
	if err := DownloadFile(downloadURL, zipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 Flutter SDK 压缩包... (这可能需要一分钟)")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, f.Name(), version)
	fmt.Printf("正在安装 Flutter 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	fmt.Printf("Flutter SDK v%s 安装成功！\n", version)
	return nil
}
