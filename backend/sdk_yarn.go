package main

import (
	"fmt"
	"path/filepath"
	"strings"
)

type Yarn struct{}

func (y *Yarn) Name() string {
	return "yarn"
}

func (y *Yarn) Category() string {
	return "build_tool"
}

func (y *Yarn) ListRemote() ([]string, error) {
	return []string{"1.22.22", "1.22.19", "1.22.10"}, nil
}

func (y *Yarn) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)
	downloadURL := fmt.Sprintf("https://github.com/yarnpkg/yarn/releases/download/v%s/yarn-v%s.tar.gz", version, version)

	fmt.Printf("正在从 %s 下载 Yarn v%s...\n", downloadURL, version)

	tempDir, cleanup, err := SetupTempDir(baseDir, "yarn")
	if err != nil {
		return err
	}
	defer cleanup()

	tarFile := filepath.Join(tempDir, "yarn.tar.gz")
	if err := DownloadFile(downloadURL, tarFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 Yarn 压缩包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := ExtractTarGz(tarFile, extractDir); err != nil {
		return fmt.Errorf("extraction failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, y.Name(), version)
	fmt.Printf("正在安装 Yarn 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	fmt.Printf("Yarn v%s 安装成功！\n", version)
	return nil
}
