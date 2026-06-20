package main

import (
	"fmt"
	"path/filepath"
	"strings"
)

type PHPSDK struct{}

func (p *PHPSDK) Name() string {
	return "php"
}

func (p *PHPSDK) Category() string {
	return "language"
}

func (p *PHPSDK) ListRemote() ([]string, error) {
	return []string{"8.3.8", "8.2.20", "8.1.29", "8.0.30", "7.4.33"}, nil
}

func (p *PHPSDK) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)
	
	var downloadURL string
	if version == "8.3.8" || version == "8.2.20" || version == "8.1.29" {
		downloadURL = fmt.Sprintf("https://windows.php.net/downloads/releases/php-%s-nts-Win32-vs16-x64.zip", version)
	} else if version == "8.0.30" {
		downloadURL = fmt.Sprintf("https://windows.php.net/downloads/releases/archives/php-%s-nts-Win32-vs16-x64.zip", version)
	} else if version == "7.4.33" {
		downloadURL = fmt.Sprintf("https://windows.php.net/downloads/releases/archives/php-%s-nts-Win32-vc15-x64.zip", version)
	} else {
		downloadURL = fmt.Sprintf("https://windows.php.net/downloads/releases/php-%s-nts-Win32-vs16-x64.zip", version)
	}

	fmt.Printf("正在从 %s 下载 PHP v%s...\n", downloadURL, version)

	tempDir, cleanup, err := SetupTempDir(baseDir, "php")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "php.zip")
	if err := DownloadFile(downloadURL, zipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 PHP 压缩包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, p.Name(), version)
	fmt.Printf("正在安装 PHP 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	fmt.Printf("PHP v%s 安装成功！\n", version)
	return nil
}
