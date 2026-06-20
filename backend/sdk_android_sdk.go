package main

import (
	"encoding/xml"
	"fmt"
	"io/ioutil"
	"net/http"
	"os"
	"path/filepath"
	"strings"
)

type AndroidSDK struct{}

func (a *AndroidSDK) Name() string {
	return "android"
}

func (a *AndroidSDK) Category() string {
	return "language"
}

type androidRepoXML struct {
	XMLName xml.Name          `xml:"repository"`
	Items   []androidRepoItem `xml:"remotePackage"`
}

type androidRepoItem struct {
	Path    string `xml:"path,attr"`
	Display string `xml:"display,attr"`
}

func (a *AndroidSDK) ListRemote() ([]string, error) {
	resp, err := http.Get("https://dl.google.com/android/repository/repository2-3.xml")
	if err != nil {
		return []string{"11076708_latest", "10406996_latest", "9862592_latest"}, nil
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return []string{"11076708_latest", "10406996_latest"}, nil
	}

	body, err := ioutil.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}

	var repo androidRepoXML
	if err := xml.Unmarshal(body, &repo); err != nil {
		return []string{"11076708_latest", "10406996_latest"}, nil
	}

	var versions []string
	for _, item := range repo.Items {
		if strings.Contains(item.Path, "cmdline-tools") {
			parts := strings.Split(item.Path, ";")
			if len(parts) >= 2 {
				versions = append(versions, parts[1])
			}
		}
	}

	if len(versions) == 0 {
		versions = []string{"11076708_latest", "10406996_latest"}
	}

	return versions, nil
}

func (a *AndroidSDK) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)

	var downloadURL string
	if strings.Contains(version, "_latest") {
		downloadURL = "https://dl.google.com/android/repository/commandlinetools-win-" + version + ".zip"
	} else {
		downloadURL = "https://dl.google.com/android/repository/commandlinetools-win-11076708_latest.zip"
	}

	fmt.Printf("Downloading Android SDK Command-line Tools from %s...\n", downloadURL)

	tempDir, cleanup, err := SetupTempDir(baseDir, "android")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "android-sdk.zip")
	if err := DownloadFile(downloadURL, zipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("Extracting Android SDK...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	destDir := filepath.Join(globalConfig.VersionsDir, a.Name(), version)
	fmt.Printf("Installing Android SDK to %s...\n", destDir)

	if err := os.MkdirAll(filepath.Join(destDir, "cmdline-tools"), 0755); err != nil {
		return err
	}

	cmdlineSrc := filepath.Join(extractDir, "cmdline-tools")
	if _, err := os.Stat(cmdlineSrc); os.IsNotExist(err) {
		if err := MoveExtractToDest(extractDir, destDir); err != nil {
			return err
		}
	} else {
		cmdlineDest := filepath.Join(destDir, "cmdline-tools", "latest")
		if err := CopyDir(cmdlineSrc, cmdlineDest); err != nil {
			return fmt.Errorf("failed to copy cmdline-tools: %v", err)
		}
	}

	for _, subdir := range []string{"platforms", "build-tools", "platform-tools", "emulator", "system-images", "sources"} {
		os.MkdirAll(filepath.Join(destDir, subdir), 0755)
	}

	fmt.Printf("Android SDK v%s installed successfully!\n", version)
	fmt.Printf("Hint: Use sdkmanager to download additional platforms and build tools.\n")
	return nil
}
