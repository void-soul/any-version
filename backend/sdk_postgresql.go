package main

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"strings"
)

type PostgreSQL struct{}

func (p *PostgreSQL) Name() string {
	return "postgresql"
}

func (p *PostgreSQL) Category() string {
	return "service"
}

func (p *PostgreSQL) ListRemote() ([]string, error) {
	return []string{"16.3-1", "15.7-1", "14.12-1"}, nil
}

func (p *PostgreSQL) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)
	downloadURL := fmt.Sprintf("https://sbp.enterprisedb.com/get/db-postgresql-%s-windows-x64-binaries.zip", version)

	fmt.Printf("正在从 %s 下载 PostgreSQL...\n", downloadURL)

	tempDir, cleanup, err := SetupTempDir(baseDir, "postgresql")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "postgresql.zip")
	if err := DownloadFile(downloadURL, zipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 PostgreSQL 压缩包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, p.Name(), version)
	fmt.Printf("正在安装 PostgreSQL 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	// Quietly initialize database data directory using initdb.exe
	dataDir := filepath.Join(destDir, "data")
	fmt.Println("正在初始化 PostgreSQL 数据目录 (这可能需要几秒钟)...")
	initCmd := exec.Command(filepath.Join(destDir, "bin", "initdb.exe"), "-D", dataDir, "-U", "postgres", "--auth-local=trust")
	if output, err := initCmd.CombinedOutput(); err != nil {
		return fmt.Errorf("failed to initialize PostgreSQL database: %v (output: %s)", err, string(output))
	}

	fmt.Printf("PostgreSQL %s 安装并初始化成功！\n", version)
	return nil
}
