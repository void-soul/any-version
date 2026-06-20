package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

type Pnpm struct{}

func (p *Pnpm) Name() string {
	return "pnpm"
}

func (p *Pnpm) Category() string {
	return "build_tool"
}

func (p *Pnpm) ListRemote() ([]string, error) {
	return []string{"9.1.1", "8.15.6", "7.33.7"}, nil
}

func (p *Pnpm) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)
	downloadURL := fmt.Sprintf("https://github.com/pnpm/pnpm/releases/download/v%s/pnpm-win-x64.exe", version)

	fmt.Printf("正在从 %s 下载 Pnpm v%s...\n", downloadURL, version)

	tempDir, cleanup, err := SetupTempDir(baseDir, "pnpm")
	if err != nil {
		return err
	}
	defer cleanup()

	pnpmExeFile := filepath.Join(tempDir, "pnpm.exe")
	if err := DownloadFile(downloadURL, pnpmExeFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, p.Name(), version)
	fmt.Printf("正在安装 Pnpm 到 %s...\n", destDir)
	
	if err := os.RemoveAll(destDir); err != nil {
		return fmt.Errorf("failed to clear destination: %v", err)
	}
	if err := os.MkdirAll(destDir, 0755); err != nil {
		return fmt.Errorf("failed to create destination: %v", err)
	}

	destExePath := filepath.Join(destDir, "pnpm.exe")
	if err := copyFile(pnpmExeFile, destExePath); err != nil {
		return fmt.Errorf("failed to copy executable: %v", err)
	}

	fmt.Printf("Pnpm v%s 安装成功！\n", version)
	return nil
}
