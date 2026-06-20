package main

import (
	"fmt"
	"path/filepath"
	"strings"
)

type Maven struct{}

func (m *Maven) Name() string {
	return "maven"
}

func (m *Maven) Category() string {
	return "build_tool"
}

func (m *Maven) ListRemote() ([]string, error) {
	return []string{"3.9.6", "3.9.5", "3.8.8", "3.6.3"}, nil
}

func (m *Maven) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)
	downloadURL := fmt.Sprintf("https://archive.apache.org/dist/maven/maven-3/%s/binaries/apache-maven-%s-bin.zip", version, version)

	fmt.Printf("正在从 %s 下载 Maven v%s...\n", downloadURL, version)

	tempDir, cleanup, err := SetupTempDir(baseDir, "maven")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "maven.zip")
	if err := DownloadFile(downloadURL, zipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 Maven 压缩包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, m.Name(), version)
	fmt.Printf("正在安装 Maven 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	fmt.Printf("Maven v%s 安装成功！\n", version)
	return nil
}
