package main

import (
	"fmt"
	"path/filepath"
	"strings"
)

type RubySDK struct{}

func (r *RubySDK) Name() string {
	return "ruby"
}

func (r *RubySDK) Category() string {
	return "language"
}

func (r *RubySDK) ListRemote() ([]string, error) {
	return []string{"3.3.1-1", "3.2.4-1", "3.1.5-1", "3.0.7-1"}, nil
}

func (r *RubySDK) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)
	downloadURL := fmt.Sprintf("https://github.com/oneclick/rubyinstaller2/releases/download/RubyInstaller-%s/rubyinstaller-%s-x64.7z", version, version)

	fmt.Printf("正在从 %s 下载 Ruby v%s...\n", downloadURL, version)

	tempDir, cleanup, err := SetupTempDir(baseDir, "ruby")
	if err != nil {
		return err
	}
	defer cleanup()

	sevenZipFile := filepath.Join(tempDir, "ruby.7z")
	if err := DownloadFile(downloadURL, sevenZipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 Ruby 压缩包... (这可能需要半分钟)")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Extract7z(sevenZipFile, extractDir); err != nil {
		return fmt.Errorf("extraction failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, r.Name(), version)
	fmt.Printf("正在安装 Ruby 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	fmt.Printf("Ruby v%s 安装成功！\n", version)
	return nil
}
