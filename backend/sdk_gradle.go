package main

import (
	"fmt"
	"path/filepath"
	"strings"
)

type Gradle struct{}

func (g *Gradle) Name() string {
	return "gradle"
}

func (g *Gradle) Category() string {
	return "build_tool"
}

func (g *Gradle) ListRemote() ([]string, error) {
	return []string{"8.8", "8.7", "8.5", "7.6.4"}, nil
}

func (g *Gradle) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)
	downloadURL := fmt.Sprintf("https://services.gradle.org/distributions/gradle-%s-bin.zip", version)

	fmt.Printf("正在从 %s 下载 Gradle v%s...\n", downloadURL, version)

	tempDir, cleanup, err := SetupTempDir(baseDir, "gradle")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "gradle.zip")
	if err := DownloadFile(downloadURL, zipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 Gradle 压缩包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, g.Name(), version)
	fmt.Printf("正在安装 Gradle 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	fmt.Printf("Gradle v%s 安装成功！\n", version)
	return nil
}
