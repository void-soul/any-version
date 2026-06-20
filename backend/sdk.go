package main

import (
	"fmt"
	"io/ioutil"
	"os"
	"os/exec"
	"path/filepath"
)

// SDK represents a software development kit version manager
type SDK interface {
	Name() string
	Category() string // "language", "service", "build_tool"
	ListRemote() ([]string, error)
	Install(version string, baseDir string) error
}

// SetupTempDir creates a temporary folder in <baseDir>/.tmp and returns its path and a cleanup function
func SetupTempDir(baseDir, prefix string) (string, func(), error) {
	tempRoot := filepath.Join(baseDir, ".tmp")
	if err := os.MkdirAll(tempRoot, 0755); err != nil {
		return "", nil, fmt.Errorf("failed to create temp root: %v", err)
	}

	dir, err := ioutil.TempDir(tempRoot, prefix+"_")
	if err != nil {
		return "", nil, fmt.Errorf("failed to create temp directory: %v", err)
	}

	cleanup := func() {
		os.RemoveAll(dir)
	}

	return dir, cleanup, nil
}

// MoveExtractToDest moves the contents of extractedDir (unwrapping a single top-level folder if present) to destDir
func MoveExtractToDest(extractedDir, destDir string) error {
	entries, err := ioutil.ReadDir(extractedDir)
	if err != nil {
		return fmt.Errorf("failed to read extracted dir: %v", err)
	}

	// Drill down if there's only one directory inside the extracted zip/tar
	srcDir := extractedDir
	if len(entries) == 1 && entries[0].IsDir() {
		srcDir = filepath.Join(extractedDir, entries[0].Name())
	}

	// Ensure destination directory is clean and exists
	if err := os.RemoveAll(destDir); err != nil {
		return fmt.Errorf("failed to clear destination dir %s: %v", destDir, err)
	}
	if err := os.MkdirAll(destDir, 0755); err != nil {
		return fmt.Errorf("failed to create destination dir %s: %v", destDir, err)
	}

	// Move all files and directories from srcDir to destDir
	subEntries, err := ioutil.ReadDir(srcDir)
	if err != nil {
		return fmt.Errorf("failed to read source dir: %v", err)
	}

	for _, entry := range subEntries {
		oldPath := filepath.Join(srcDir, entry.Name())
		newPath := filepath.Join(destDir, entry.Name())

		if err := os.Rename(oldPath, newPath); err != nil {
			// Fallback to manual copying if rename fails (e.g., across disk partitions)
			if entry.IsDir() {
				if err := CopyDir(oldPath, newPath); err != nil {
					return fmt.Errorf("failed to copy dir %s to %s: %v", oldPath, newPath, err)
				}
			} else {
				if err := copyFile(oldPath, newPath); err != nil {
					return fmt.Errorf("failed to copy file %s to %s: %v", oldPath, newPath, err)
				}
			}
		}
	}

	return nil
}

// Extract7z extracts a .7z file using the system's built-in tar utility
func Extract7z(src, dest string) error {
	if err := os.MkdirAll(dest, 0755); err != nil {
		return err
	}
	cmd := exec.Command("tar", "-xf", src, "-C", dest)
	if output, err := cmd.CombinedOutput(); err != nil {
		return fmt.Errorf("7z extraction failed: %v (output: %s)", err, string(output))
	}
	return nil
}

