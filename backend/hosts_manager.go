package main

import (
	"io/ioutil"
	"os"
	"path/filepath"
)

// ReadHosts reads the Windows hosts file contents
func ReadHosts() (string, error) {
	hostsPath := filepath.Join(os.Getenv("SystemRoot"), "System32", "drivers", "etc", "hosts")
	data, err := ioutil.ReadFile(hostsPath)
	if err != nil {
		return "", err
	}
	return string(data), nil
}

// WriteHosts writes new contents to the Windows hosts file
func WriteHosts(content string) error {
	hostsPath := filepath.Join(os.Getenv("SystemRoot"), "System32", "drivers", "etc", "hosts")
	
	// Try to write to the file
	err := ioutil.WriteFile(hostsPath, []byte(content), 0644)
	if err != nil {
		if os.IsPermission(err) {
			return os.ErrPermission
		}
		return err
	}
	return nil
}
