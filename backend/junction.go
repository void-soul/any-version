package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
)

// CreateJunction deletes the existing link (if any) and creates a Windows Directory Junction
func CreateJunction(linkPath, targetPath string) error {
	// Clean the paths and make them absolute
	var err error
	linkPath, err = filepath.Abs(linkPath)
	if err != nil {
		return err
	}
	targetPath, err = filepath.Abs(targetPath)
	if err != nil {
		return err
	}

	// 1. Remove the link path if it exists
	if _, err := os.Lstat(linkPath); err == nil {
		// It exists. Delete it.
		// If it's a junction, os.Remove removes the junction itself, not target content.
		if err := os.Remove(linkPath); err != nil {
			// If it fails, try os.RemoveAll just in case (e.g. if it's a regular directory somehow)
			if err := os.RemoveAll(linkPath); err != nil {
				return fmt.Errorf("failed to remove existing link path %s: %v", linkPath, err)
			}
		}
	}

	// 2. Ensure parent directory of linkPath exists
	parent := filepath.Dir(linkPath)
	if err := os.MkdirAll(parent, 0755); err != nil {
		return fmt.Errorf("failed to create parent directory %s: %v", parent, err)
	}

	// 3. Ensure target directory exists
	if _, err := os.Stat(targetPath); os.IsNotExist(err) {
		return fmt.Errorf("target path %s does not exist", targetPath)
	}

	// 4. Run cmd.exe /c mklink /J <linkPath> <targetPath>
	// This command creates a directory junction and does NOT require admin privileges.
	cmd := exec.Command("cmd", "/c", "mklink", "/J", linkPath, targetPath)
	output, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("failed to create directory junction: %v (output: %s)", err, string(output))
	}

	return nil
}

// RemoveJunction deletes the junction at linkPath if it exists
func RemoveJunction(linkPath string) error {
	linkPath, err := filepath.Abs(linkPath)
	if err != nil {
		return err
	}

	if _, err := os.Lstat(linkPath); err == nil {
		if err := os.Remove(linkPath); err != nil {
			return fmt.Errorf("failed to remove junction %s: %v", linkPath, err)
		}
	}
	return nil
}
