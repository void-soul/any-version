package main

import (
	"encoding/json"
	"io/ioutil"
	"os"
	"path/filepath"
)

type Config struct {
	VersionsDir string `json:"versions_dir"`
	LinksDir    string `json:"links_dir"`
}

var globalConfig Config

// LoadConfig loads or creates default config settings
func LoadConfig() {
	baseDir := getBaseDir()
	configPath := filepath.Join(baseDir, "config.json")

	// Set defaults
	globalConfig = Config{
		VersionsDir: filepath.Join(baseDir, "versions"),
		LinksDir:    filepath.Join(baseDir, "links"),
	}

	// Read existing file if present
	data, err := ioutil.ReadFile(configPath)
	if err == nil {
		var fileConfig Config
		if err := json.Unmarshal(data, &fileConfig); err == nil {
			if fileConfig.VersionsDir != "" {
				globalConfig.VersionsDir = filepath.Clean(fileConfig.VersionsDir)
			}
			if fileConfig.LinksDir != "" {
				globalConfig.LinksDir = filepath.Clean(fileConfig.LinksDir)
			}
		}
	} else {
		// Save default config if not exists
		os.MkdirAll(baseDir, 0755)
		SaveConfig()
	}
}

// SaveConfig saves current globalConfig settings to registry folder config file
func SaveConfig() error {
	baseDir := getBaseDir()
	configPath := filepath.Join(baseDir, "config.json")

	data, err := json.MarshalIndent(globalConfig, "", "  ")
	if err != nil {
		return err
	}

	return ioutil.WriteFile(configPath, data, 0644)
}
