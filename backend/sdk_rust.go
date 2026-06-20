package main

import (
	"encoding/json"
	"fmt"
	"io/ioutil"
	"net/http"
	"os"
	"path/filepath"
	"strings"
)

type RustSDK struct{}

func (r *RustSDK) Name() string {
	return "rust"
}

func (r *RustSDK) Category() string {
	return "language"
}

func (r *RustSDK) ListRemote() ([]string, error) {
	req, err := http.NewRequest("GET", "https://api.github.com/repos/rust-lang/rust/releases", nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("User-Agent", "Any-Version-Manager")

	resp, clientErr := http.DefaultClient.Do(req)
	if clientErr != nil {
		return nil, clientErr
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("failed to fetch Rust releases: %s (API rate limit exceeded?)", resp.Status)
	}

	var releases []githubRelease
	if err := json.NewDecoder(resp.Body).Decode(&releases); err != nil {
		return nil, err
	}

	var versions []string
	for _, release := range releases {
		v := release.TagName
		// Rust tags are like "1.76.0", filter out beta, nightly, and arbitrary tags
		if !strings.Contains(v, "-") && !strings.Contains(v, "/") && len(v) > 0 && v[0] >= '0' && v[0] <= '9' {
			versions = append(versions, v)
		}
	}

	return versions, nil
}

func (r *RustSDK) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)

	// Construct download URL for the standalone tar.gz package on Windows MSVC x64
	// E.g. https://static.rust-lang.org/dist/rust-1.76.0-x86_64-pc-windows-msvc.tar.gz
	downloadURL := fmt.Sprintf("https://static.rust-lang.org/dist/rust-%s-x86_64-pc-windows-msvc.tar.gz", version)
	fmt.Printf("正在从 %s 下载 Rust v%s 独立工具链...\n", downloadURL, version)

	tempDir, cleanup, err := SetupTempDir(baseDir, "rust")
	if err != nil {
		return err
	}
	defer cleanup()

	tarFile := filepath.Join(tempDir, "rust.tar.gz")
	if err := DownloadFile(downloadURL, tarFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 Rust 工具链组件... (这可能需要一分钟)")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := ExtractTarGz(tarFile, extractDir); err != nil {
		return fmt.Errorf("tar.gz extraction failed: %v", err)
	}

	// Rust standalone distributions have subfolders for each component:
	// - rustc (compiler, standard libraries, etc.)
	// - cargo (package manager)
	// - rust-std-x86_64-pc-windows-msvc (target stdlib)
	// We need to merge all these folders into our target versions/rust/<version> folder.
	destDir := filepath.Join(globalConfig.VersionsDir, r.Name(), version)
	fmt.Printf("正在合并并安装 Rust 组件到 %s...\n", destDir)
	if err := os.RemoveAll(destDir); err != nil {
		return fmt.Errorf("failed to clear destination dir %s: %v", destDir, err)
	}
	if err := os.MkdirAll(destDir, 0755); err != nil {
		return fmt.Errorf("failed to create destination dir %s: %v", destDir, err)
	}

	entries, err := ioutil.ReadDir(extractDir)
	if err != nil {
		return fmt.Errorf("failed to read extracted directory: %v", err)
	}

	// Drill down if there is a wrapper root directory
	var componentsRoot = extractDir
	if len(entries) == 1 && entries[0].IsDir() {
		componentsRoot = filepath.Join(extractDir, entries[0].Name())
	}

	compEntries, err := ioutil.ReadDir(componentsRoot)
	if err != nil {
		return fmt.Errorf("failed to read components directory: %v", err)
	}

	for _, entry := range compEntries {
		if !entry.IsDir() {
			continue // Skip manifest files at root
		}

		compDir := filepath.Join(componentsRoot, entry.Name())
		// Each component contains directories like "bin", "lib", "share"
		// Merge them into the final destDir
		innerEntries, err := ioutil.ReadDir(compDir)
		if err != nil {
			return fmt.Errorf("failed to read component subfolder %s: %v", compDir, err)
		}

		for _, innerEntry := range innerEntries {
			// Skip metadata files of components
			if innerEntry.Name() == "manifest.in" {
				continue
			}

			srcPath := filepath.Join(compDir, innerEntry.Name())
			dstPath := filepath.Join(destDir, innerEntry.Name())

			if innerEntry.IsDir() {
				if err := CopyDir(srcPath, dstPath); err != nil {
					return fmt.Errorf("failed to merge directory %s to %s: %v", srcPath, dstPath, err)
				}
			} else {
				if err := copyFile(srcPath, dstPath); err != nil {
					return fmt.Errorf("failed to merge file %s to %s: %v", srcPath, dstPath, err)
				}
			}
		}
	}

	fmt.Printf("Rust v%s 安装与配置成功！\n", version)
	return nil
}
