package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

type MongoDB struct{}

func (m *MongoDB) Name() string {
	return "mongodb"
}

func (m *MongoDB) Category() string {
	return "service"
}

func (m *MongoDB) ListRemote() ([]string, error) {
	return []string{"7.0.9", "6.0.14", "5.0.26"}, nil
}

func (m *MongoDB) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)
	downloadURL := fmt.Sprintf("https://fastdl.mongodb.org/windows/mongodb-windows-x86_64-%s.zip", version)

	fmt.Printf("正在从 %s 下载 MongoDB...\n", downloadURL)

	tempDir, cleanup, err := SetupTempDir(baseDir, "mongodb")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "mongodb.zip")
	if err := DownloadFile(downloadURL, zipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 MongoDB 压缩包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, m.Name(), version)
	fmt.Printf("正在安装 MongoDB 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	// Create database data folder
	dataDir := filepath.Join(destDir, "data")
	if err := os.MkdirAll(dataDir, 0755); err != nil {
		return fmt.Errorf("failed to create data folder: %v", err)
	}

	fmt.Printf("MongoDB %s 安装成功！\n", version)
	return nil
}
