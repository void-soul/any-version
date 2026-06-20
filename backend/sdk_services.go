package main

import (
	"fmt"
	"io/ioutil"
	"os/exec"
	"path/filepath"
	"strings"
)

// --- Nginx SDK Handler ---
type NginxSDK struct{}

func (n *NginxSDK) Name() string {
	return "nginx"
}

func (n *NginxSDK) Category() string {
	return "service"
}

func (n *NginxSDK) ListRemote() ([]string, error) {
	// Nginx stable releases
	return []string{"1.26.1", "1.26.0", "1.24.0", "1.22.1"}, nil
}

func (n *NginxSDK) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)
	downloadURL := fmt.Sprintf("https://nginx.org/download/nginx-%s.zip", version)

	fmt.Printf("正在从 %s 下载 Nginx...\n", downloadURL)
	tempDir, cleanup, err := SetupTempDir(baseDir, "nginx")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "nginx.zip")
	if err := DownloadFile(downloadURL, zipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 Nginx 压缩包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, n.Name(), version)
	fmt.Printf("正在安装 Nginx 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	fmt.Printf("Nginx %s 安装成功！\n", version)
	return nil
}

// --- Redis SDK Handler ---
type RedisSDK struct{}

func (r *RedisSDK) Name() string {
	return "redis"
}

func (r *RedisSDK) Category() string {
	return "service"
}

func (r *RedisSDK) ListRemote() ([]string, error) {
	// Stable Redis Windows builds by tporadowski
	return []string{"5.0.14.1", "3.0.504"}, nil
}

func (r *RedisSDK) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)
	downloadURL := fmt.Sprintf("https://github.com/tporadowski/redis/releases/download/v%s/Redis-x64-%s.zip", version, version)
	if version == "3.0.504" {
		// MSOpenTech old release URL pattern
		downloadURL = "https://github.com/microsoftarchive/redis/releases/download/win-3.0.504/Redis-x64-3.0.504.zip"
	}

	fmt.Printf("正在从 %s 下载 Redis...\n", downloadURL)
	tempDir, cleanup, err := SetupTempDir(baseDir, "redis")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "redis.zip")
	if err := DownloadFile(downloadURL, zipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 Redis 压缩包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, r.Name(), version)
	fmt.Printf("正在安装 Redis 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	fmt.Printf("Redis %s 安装成功！\n", version)
	return nil
}

// --- MySQL SDK Handler ---
type MySQLSDK struct{}

func (m *MySQLSDK) Name() string {
	return "mysql"
}

func (m *MySQLSDK) Category() string {
	return "service"
}

func (m *MySQLSDK) ListRemote() ([]string, error) {
	// Stable MySQL releases
	return []string{"8.0.36", "8.4.0", "5.7.44"}, nil
}

func (m *MySQLSDK) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)
	parts := strings.Split(version, ".")
	series := "MySQL-8.0"
	if len(parts) >= 2 {
		series = "MySQL-" + parts[0] + "." + parts[1]
	}

	downloadURL := fmt.Sprintf("https://cdn.mysql.com/Downloads/%s/mysql-%s-winx64.zip", series, version)

	fmt.Printf("正在从 %s 下载 MySQL...\n", downloadURL)
	tempDir, cleanup, err := SetupTempDir(baseDir, "mysql")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "mysql.zip")
	if err := DownloadFile(downloadURL, zipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 MySQL 压缩包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, m.Name(), version)
	fmt.Printf("正在安装 MySQL 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	// Post-Install Configuration: Write my.ini and initialize database
	fmt.Println("正在配置 MySQL (生成 my.ini)...")
	myIniPath := filepath.Join(destDir, "my.ini")
	dataDir := filepath.Join(destDir, "data")
	
	// Escape backslashes for my.ini paths
	cleanBaseDir := strings.Replace(destDir, "\\", "/", -1)
	cleanDataDir := strings.Replace(dataDir, "\\", "/", -1)

	myIniContent := fmt.Sprintf(`[mysqld]
port=3306
basedir=%s
datadir=%s
max_connections=200
character-set-server=utf8mb4
default-storage-engine=INNODB
default_authentication_plugin=mysql_native_password

[mysql]
default-character-set=utf8mb4

[client]
port=3306
default-character-set=utf8mb4
`, cleanBaseDir, cleanDataDir)

	if err := ioutil.WriteFile(myIniPath, []byte(myIniContent), 0644); err != nil {
		return fmt.Errorf("failed to write my.ini: %v", err)
	}

	// Initialize the MySQL database files quietly
	fmt.Println("正在初始化 MySQL 数据目录 (这可能需要几秒钟)...")
	mysqlDaemon := filepath.Join(destDir, "bin", "mysqld.exe")
	initCmd := exec.Command(mysqlDaemon, "--defaults-file="+myIniPath, "--initialize-insecure", "--console")
	if output, err := initCmd.CombinedOutput(); err != nil {
		return fmt.Errorf("failed to initialize MySQL database: %v (output: %s)", err, string(output))
	}

	fmt.Printf("MySQL %s 安装并初始化成功！\n", version)
	return nil
}
