package main

import (
	"bytes"
	"fmt"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"unsafe"
)

// BroadcastSettingChange notifies Windows Explorer and other processes that environment variables have changed
func BroadcastSettingChange() {
	user32 := syscall.NewLazyDLL("user32.dll")
	sendMessageTimeout := user32.NewProc("SendMessageTimeoutW")

	lParamPtr, _ := syscall.UTF16PtrFromString("Environment")

	var result uintptr
	sendMessageTimeout.Call(
		0xffff, // HWND_BROADCAST
		0x001a, // WM_SETTINGCHANGE
		0,
		uintptr(unsafe.Pointer(lParamPtr)),
		0x0002, // SMTO_ABORTIFHUNG
		5000,   // timeout in ms
		uintptr(unsafe.Pointer(&result)),
	)
}

// GetUserEnv reads an environment variable from HKEY_CURRENT_USER\Environment using PowerShell
func GetUserEnv(name string) (string, error) {
	cmdStr := fmt.Sprintf("[Environment]::GetEnvironmentVariable('%s', 'User')", name)
	cmd := exec.Command("powershell", "-NoProfile", "-NonInteractive", "-Command", cmdStr)
	var out bytes.Buffer
	cmd.Stdout = &out
	err := cmd.Run()
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(out.String()), nil
}

// SetUserEnv writes an environment variable to HKEY_CURRENT_USER\Environment using PowerShell
func SetUserEnv(name, value string) error {
	// We escape single quotes in value by doubling them
	escapedValue := strings.Replace(value, "'", "''", -1)
	cmdStr := fmt.Sprintf("[Environment]::SetEnvironmentVariable('%s', '%s', 'User')", name, escapedValue)
	cmd := exec.Command("powershell", "-NoProfile", "-NonInteractive", "-Command", cmdStr)
	output, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("powershell failed: %v (output: %s)", err, string(output))
	}
	return nil
}

// InitEnvironment configures the User PATH, JAVA_HOME, and GOROOT
func InitEnvironment(baseDir string) error {
	// Target links directories to add to PATH
	linksDir := globalConfig.LinksDir

	// We want to add:
	// - C:\Users\<User>\.any-version\links\nodejs
	// - C:\Users\<User>\.any-version\links\java\bin
	// - C:\Users\<User>\.any-version\links\python
	// - C:\Users\<User>\.any-version\links\python\Scripts
	// - C:\Users\<User>\.any-version\links\go\bin
	// - C:\Users\<User>\.any-version\links\bun
	// - C:\Users\<User>\.any-version\links\flutter\bin
	// - C:\Users\<User>\.any-version\links\rust\bin
	// - C:\Users\<User>\.any-version\links\android\cmdline-tools\latest\bin
	// - C:\Users\<User>\.any-version\links\android\platform-tools

	nodePath := filepath.Join(linksDir, "nodejs")
	javaPath := filepath.Join(linksDir, "java", "bin")
	pythonPath := filepath.Join(linksDir, "python")
	pythonScriptsPath := filepath.Join(linksDir, "python", "Scripts")
	goPath := filepath.Join(linksDir, "go", "bin")
	bunPath := filepath.Join(linksDir, "bun")
	flutterPath := filepath.Join(linksDir, "flutter", "bin")
	rustPath := filepath.Join(linksDir, "rust", "bin")
	androidCmdlinePath := filepath.Join(linksDir, "android", "cmdline-tools", "latest", "bin")
	androidPlatformToolsPath := filepath.Join(linksDir, "android", "platform-tools")

	targetPaths := []string{
		nodePath,
		javaPath,
		pythonPath,
		pythonScriptsPath,
		goPath,
		bunPath,
		flutterPath,
		rustPath,
		androidCmdlinePath,
		androidPlatformToolsPath,
	}

	// 1. Get current User PATH
	currentPath, err := GetUserEnv("PATH")
	if err != nil {
		return fmt.Errorf("failed to read User PATH: %v", err)
	}

	// Split current paths
	pathParts := filepath.SplitList(currentPath)
	pathMap := make(map[string]bool)
	for _, p := range pathParts {
		// Clean and lowercase for windows path comparisons
		pClean := strings.ToLower(filepath.Clean(strings.TrimSpace(p)))
		if pClean != "" {
			pathMap[pClean] = true
		}
	}

	// Determine which paths need to be added
	var addedAny bool
	for _, tp := range targetPaths {
		tpClean := strings.ToLower(filepath.Clean(tp))
		if !pathMap[tpClean] {
			pathParts = append(pathParts, tp)
			addedAny = true
		}
	}

	// 2. Set new User PATH if changed
	if addedAny {
		newPath := strings.Join(pathParts, string(filepath.ListSeparator))
		if err := SetUserEnv("PATH", newPath); err != nil {
			return fmt.Errorf("failed to update User PATH: %v", err)
		}
		fmt.Println("Added Any-Version paths to User PATH environment variable.")
	} else {
		fmt.Println("Any-Version paths are already in User PATH.")
	}

	// 3. Set JAVA_HOME pointing to the java link directory
	javaHomeTarget := filepath.Join(linksDir, "java")
	currentJavaHome, _ := GetUserEnv("JAVA_HOME")
	if strings.ToLower(filepath.Clean(currentJavaHome)) != strings.ToLower(filepath.Clean(javaHomeTarget)) {
		if err := SetUserEnv("JAVA_HOME", javaHomeTarget); err != nil {
			return fmt.Errorf("failed to set JAVA_HOME: %v", err)
		}
		fmt.Printf("Set User environment variable JAVA_HOME to %s\n", javaHomeTarget)
		addedAny = true
	}

	// 4. Set GOROOT pointing to the go link directory
	goRootTarget := filepath.Join(linksDir, "go")
	currentGoRoot, _ := GetUserEnv("GOROOT")
	if strings.ToLower(filepath.Clean(currentGoRoot)) != strings.ToLower(filepath.Clean(goRootTarget)) {
		if err := SetUserEnv("GOROOT", goRootTarget); err != nil {
			return fmt.Errorf("failed to set GOROOT: %v", err)
		}
		fmt.Printf("Set User environment variable GOROOT to %s\n", goRootTarget)
		addedAny = true
	}

	// 5. Set ANDROID_HOME pointing to the android link directory
	androidHomeTarget := filepath.Join(linksDir, "android")
	currentAndroidHome, _ := GetUserEnv("ANDROID_HOME")
	if strings.ToLower(filepath.Clean(currentAndroidHome)) != strings.ToLower(filepath.Clean(androidHomeTarget)) {
		if err := SetUserEnv("ANDROID_HOME", androidHomeTarget); err != nil {
			return fmt.Errorf("failed to set ANDROID_HOME: %v", err)
		}
		if err := SetUserEnv("ANDROID_SDK_ROOT", androidHomeTarget); err != nil {
			return fmt.Errorf("failed to set ANDROID_SDK_ROOT: %v", err)
		}
		fmt.Printf("Set User environment variable ANDROID_HOME to %s\n", androidHomeTarget)
		addedAny = true
	}

	// 5. Broadcast WM_SETTINGCHANGE if anything was updated
	if addedAny {
		fmt.Println("Broadcasting environment variable updates to system... (Please restart your terminal to apply changes)")
		BroadcastSettingChange()
	}

	return nil
}
