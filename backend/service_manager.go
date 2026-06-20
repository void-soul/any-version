package main

import (
	"bytes"
	"fmt"
	"io/ioutil"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
)

type ServiceInfo struct {
	Name          string
	Status        string // "running" or "stopped"
	ActiveVersion string
	Port          string
	Pid           int
}

// GetRunningServices scans running processes using wmic to find active service instances under our versions dir
func GetRunningServices() (map[string]*ServiceInfo, error) {
	services := map[string]*ServiceInfo{
		"nginx":      {Name: "nginx", Status: "stopped", ActiveVersion: "", Port: "80", Pid: 0},
		"redis":      {Name: "redis", Status: "stopped", ActiveVersion: "", Port: "6379", Pid: 0},
		"mysql":      {Name: "mysql", Status: "stopped", ActiveVersion: "", Port: "3306", Pid: 0},
		"mongodb":    {Name: "mongodb", Status: "stopped", ActiveVersion: "", Port: "27017", Pid: 0},
		"postgresql": {Name: "postgresql", Status: "stopped", ActiveVersion: "", Port: "5432", Pid: 0},
	}

	// Read active versions from junction links (if any)
	for name, svc := range services {
		// Check if the service has any installed versions
		sdkDir := filepath.Join(globalConfig.VersionsDir, name)
		hasInstalled := false
		if entries, err := ioutil.ReadDir(sdkDir); err == nil {
			for _, entry := range entries {
				if entry.IsDir() {
					hasInstalled = true
					break
				}
			}
		}
		if !hasInstalled {
			svc.Status = "not_installed"
		}

		junctionPath := filepath.Join(globalConfig.LinksDir, name)
		if activeDir, err := filepath.EvalSymlinks(junctionPath); err == nil {
			svc.ActiveVersion = filepath.Base(activeDir)
			// Read ports from configuration files if they exist
			if name == "mysql" {
				if port := readPortFromIni(filepath.Join(activeDir, "my.ini"), "port"); port != "" {
					svc.Port = port
				}
			} else if name == "redis" {
				if port := readPortFromConf(filepath.Join(activeDir, "redis.windows.conf"), "port"); port != "" {
					svc.Port = port
				}
			} else if name == "nginx" {
				if port := readNginxPort(filepath.Join(activeDir, "conf", "nginx.conf")); port != "" {
					svc.Port = port
				}
			}
		}
	}

	// Use wmic to find running processes
	cmd := exec.Command("wmic", "process", "get", "ExecutablePath,ProcessId")
	var out bytes.Buffer
	cmd.Stdout = &out
	_ = cmd.Run() // Ignore errors, it may exit with 1 if no output or permissions

	lines := strings.Split(out.String(), "\n")
	versionsDirClean := strings.ToLower(filepath.Clean(globalConfig.VersionsDir))

	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(strings.ToLower(line), "executablepath") {
			continue
		}

		// Parse from end since ExecutablePath might contain spaces
		lastSpaceIdx := strings.LastIndex(line, " ")
		if lastSpaceIdx == -1 {
			continue
		}

		pathPart := strings.TrimSpace(line[:lastSpaceIdx])
		pidPart := strings.TrimSpace(line[lastSpaceIdx:])
		
		pathClean := strings.ToLower(filepath.Clean(pathPart))
		if !strings.Contains(pathClean, versionsDirClean) {
			continue
		}

		pid, err := strconv.Atoi(pidPart)
		if err != nil {
			continue
		}

		// Determine service type based on path
		if strings.HasSuffix(pathClean, "nginx.exe") {
			// Extract version from path e.g. versions/nginx/<version>/nginx.exe
			version := extractVersionFromPath(pathClean, "nginx")
			services["nginx"].Status = "running"
			services["nginx"].ActiveVersion = version
			services["nginx"].Pid = pid
		} else if strings.HasSuffix(pathClean, "redis-server.exe") {
			version := extractVersionFromPath(pathClean, "redis")
			services["redis"].Status = "running"
			services["redis"].ActiveVersion = version
			services["redis"].Pid = pid
		} else if strings.HasSuffix(pathClean, "mysqld.exe") {
			version := extractVersionFromPath(pathClean, "mysql")
			services["mysql"].Status = "running"
			services["mysql"].ActiveVersion = version
			services["mysql"].Pid = pid
		} else if strings.HasSuffix(pathClean, "mongod.exe") {
			version := extractVersionFromPath(pathClean, "mongodb")
			services["mongodb"].Status = "running"
			services["mongodb"].ActiveVersion = version
			services["mongodb"].Pid = pid
		} else if strings.HasSuffix(pathClean, "postgres.exe") {
			version := extractVersionFromPath(pathClean, "postgresql")
			services["postgresql"].Status = "running"
			services["postgresql"].ActiveVersion = version
			services["postgresql"].Pid = pid
		}
	}

	return services, nil
}

func extractVersionFromPath(path, name string) string {
	parts := strings.Split(path, string(filepath.Separator))
	for i, part := range parts {
		if part == name && i+1 < len(parts) {
			return parts[i+1]
		}
	}
	return ""
}

// StartService launches the Nginx, Redis, or MySQL service in the background
func StartService(name, version string) error {
	services, _ := GetRunningServices()
	svc, exists := services[name]
	if !exists {
		return fmt.Errorf("unknown service: %s", name)
	}

	if svc.Status == "running" {
		return fmt.Errorf("%s is already running (PID: %d)", name, svc.Pid)
	}

	dir := filepath.Join(globalConfig.VersionsDir, name, version)
	if _, err := os.Stat(dir); os.IsNotExist(err) {
		return fmt.Errorf("service version %s is not installed", version)
	}

	// Create a junction link so it matches active version configuration
	junctionPath := filepath.Join(globalConfig.LinksDir, name)
	_ = CreateJunction(junctionPath, dir)

	var cmd *exec.Cmd
	switch name {
	case "nginx":
		cmd = exec.Command("cmd", "/c", "start", "/b", "nginx.exe")
		cmd.Dir = dir
	case "redis":
		confFile := "redis.windows.conf"
		if _, err := os.Stat(filepath.Join(dir, confFile)); os.IsNotExist(err) {
			confFile = ""
		}
		if confFile != "" {
			cmd = exec.Command("cmd", "/c", "start", "/b", "redis-server.exe", confFile)
		} else {
			cmd = exec.Command("cmd", "/c", "start", "/b", "redis-server.exe")
		}
		cmd.Dir = dir
	case "mysql":
		cmd = exec.Command("cmd", "/c", "start", "/b", "bin\\mysqld.exe", "--defaults-file=my.ini", "--console")
		cmd.Dir = dir
	case "mongodb":
		dataDir := filepath.Join(dir, "data")
		cmd = exec.Command("cmd", "/c", "start", "/b", "bin\\mongod.exe", "--port", "27017", "--dbpath", dataDir)
		cmd.Dir = dir
	case "postgresql":
		dataDir := filepath.Join(dir, "data")
		logFile := filepath.Join(dir, "logfile")
		cmd = exec.Command("cmd", "/c", "start", "/b", "bin\\pg_ctl.exe", "-D", dataDir, "-l", logFile, "start")
		cmd.Dir = dir
	}

	if err := cmd.Run(); err != nil {
		return fmt.Errorf("failed to start %s: %v", name, err)
	}

	return nil
}

// StopService safely shuts down the Nginx, Redis, or MySQL service
func StopService(name string) error {
	services, _ := GetRunningServices()
	svc, exists := services[name]
	if !exists {
		return fmt.Errorf("unknown service: %s", name)
	}

	if svc.Status == "stopped" {
		return fmt.Errorf("%s is not running", name)
	}

	dir := filepath.Join(globalConfig.VersionsDir, name, svc.ActiveVersion)

	// Try provider-specific shutdown utilities first
	var shutdownErr error
	switch name {
	case "nginx":
		stopCmd := exec.Command(filepath.Join(dir, "nginx.exe"), "-s", "stop")
		stopCmd.Dir = dir
		shutdownErr = stopCmd.Run()
	case "redis":
		stopCmd := exec.Command(filepath.Join(dir, "redis-cli.exe"), "shutdown")
		stopCmd.Dir = dir
		shutdownErr = stopCmd.Run()
	case "mysql":
		stopCmd := exec.Command(filepath.Join(dir, "bin", "mysqladmin.exe"), "-u", "root", "shutdown")
		stopCmd.Dir = dir
		shutdownErr = stopCmd.Run()
	case "mongodb":
		shutdownErr = fmt.Errorf("taskkill fallback")
	case "postgresql":
		stopCmd := exec.Command(filepath.Join(dir, "bin", "pg_ctl.exe"), "-D", filepath.Join(dir, "data"), "stop")
		stopCmd.Dir = dir
		shutdownErr = stopCmd.Run()
	}

	// Fallback to taskkill if PID is still active or shutdown utility failed
	if shutdownErr != nil || svc.Pid > 0 {
		killCmd := exec.Command("taskkill", "/f", "/pid", strconv.Itoa(svc.Pid))
		_ = killCmd.Run()
	}

	return nil
}

// Configuration Reading Helpers
func readPortFromIni(iniPath, key string) string {
	data, err := ioutil.ReadFile(iniPath)
	if err != nil {
		return ""
	}
	lines := strings.Split(string(data), "\n")
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(strings.ToLower(line), key) {
			parts := strings.SplitN(line, "=", 2)
			if len(parts) == 2 {
				return strings.TrimSpace(parts[1])
			}
		}
	}
	return ""
}

func readPortFromConf(confPath, key string) string {
	data, err := ioutil.ReadFile(confPath)
	if err != nil {
		return ""
	}
	lines := strings.Split(string(data), "\n")
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(strings.ToLower(line), key) {
			// Redis conf format: port 6379
			parts := strings.Fields(line)
			if len(parts) >= 2 && strings.ToLower(parts[0]) == key {
				return parts[1]
			}
		}
	}
	return ""
}

func readNginxPort(confPath string) string {
	data, err := ioutil.ReadFile(confPath)
	if err != nil {
		return ""
	}
	content := string(data)
	// Simple scanner for "listen 80;" or "listen 8080;"
	idx := strings.Index(content, "listen")
	if idx == -1 {
		return ""
	}
	sub := content[idx:]
	semiIdx := strings.Index(sub, ";")
	if semiIdx == -1 {
		return ""
	}
	listenLine := sub[6:semiIdx]
	parts := strings.Fields(listenLine)
	if len(parts) > 0 {
		return strings.TrimSpace(parts[0])
	}
	return ""
}
