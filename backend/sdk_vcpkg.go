package main

import (
	"fmt"
	"path/filepath"
	"strings"
)

type Vcpkg struct{}

func (v *Vcpkg) Name() string {
	return "vcpkg"
}

func (v *Vcpkg) Category() string {
	return "build_tool"
}

func (v *Vcpkg) ListRemote() ([]string, error) {
	return []string{"2024.04.26", "2024.02.14", "2023.12.12"}, nil
}

func (v *Vcpkg) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)
	
	mainZipURL := fmt.Sprintf("https://github.com/microsoft/vcpkg/archive/refs/tags/%s.zip", version)
	toolTag := strings.Replace(version, ".", "-", -1)
	toolURL := fmt.Sprintf("https://github.com/microsoft/vcpkg-tool/releases/download/%s/vcpkg.exe", toolTag)

	fmt.Printf("正在从 %s 下载 Vcpkg 仓库包 v%s...\n", mainZipURL, version)

	tempDir, cleanup, err := SetupTempDir(baseDir, "vcpkg")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "vcpkg.zip")
	if err := DownloadFile(mainZipURL, zipFile); err != nil {
		return fmt.Errorf("download of vcpkg repo failed: %v", err)
	}

	fmt.Println("正在解压 Vcpkg 仓库包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, v.Name(), version)
	fmt.Printf("正在安装 Vcpkg 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	fmt.Printf("正在从 %s 下载预编译的 vcpkg.exe...\n", toolURL)
	vcpkgExePath := filepath.Join(destDir, "vcpkg.exe")
	if err := DownloadFile(toolURL, vcpkgExePath); err != nil {
		return fmt.Errorf("download of precompiled vcpkg.exe failed: %v", err)
	}

	fmt.Printf("Vcpkg v%s 安装与配置成功！\n", version)
	return nil
}
