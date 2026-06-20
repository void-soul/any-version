package main

import (
	"fmt"
	"path/filepath"
	"strings"
)

type Pub struct{}

func (p *Pub) Name() string {
	return "pub"
}

func (p *Pub) Category() string {
	return "build_tool"
}

func (p *Pub) ListRemote() ([]string, error) {
	return []string{"3.4.1", "3.3.4", "3.2.6", "3.0.7"}, nil
}

func (p *Pub) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)
	downloadURL := fmt.Sprintf("https://storage.googleapis.com/dart-archive/channels/stable/release/%s/sdk/dartsdk-windows-x64-release.zip", version)

	fmt.Printf("正在从 %s 下载 Dart SDK (含有 pub) v%s...\n", downloadURL, version)

	tempDir, cleanup, err := SetupTempDir(baseDir, "pub")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "dart.zip")
	if err := DownloadFile(downloadURL, zipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 Dart SDK 压缩包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, p.Name(), version)
	fmt.Printf("正在安装 Dart/Pub 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	fmt.Printf("Pub v%s 安装成功！\n", version)
	return nil
}
