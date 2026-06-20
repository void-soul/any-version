package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"path/filepath"
	"strings"
)

type JavaSDK struct{}

func (j *JavaSDK) Name() string {
	return "java"
}

func (j *JavaSDK) Category() string {
	return "language"
}

type adoptiumReleases struct {
	Releases []string `json:"releases"`
}

type zuluPackage struct {
	DownloadURL string `json:"download_url"`
	JavaVersion []int  `json:"java_version"`
	Name        string `json:"name"`
}

func (j *JavaSDK) ListRemote() ([]string, error) {
	var allVersions []string

	// 1. Fetch Adoptium (Temurin) stable GA versions for major LTS/GA releases
	majorVersions := []string{"25", "23", "21", "17", "11", "8"}
	for _, mv := range majorVersions {
		var nextMv string
		switch mv {
		case "8":
			nextMv = "9"
		case "11":
			nextMv = "12"
		case "17":
			nextMv = "18"
		case "21":
			nextMv = "22"
		case "23":
			nextMv = "24"
		case "25":
			nextMv = "26"
		}

		urlStr := fmt.Sprintf("https://api.adoptium.net/v3/info/release_names?project=jdk&release_type=ga&os=windows&architecture=x64&image_type=jdk&version=[%s,%s)", mv, nextMv)
		resp, err := http.Get(urlStr)
		if err == nil {
			defer resp.Body.Close()
			if resp.StatusCode == http.StatusOK {
				var data adoptiumReleases
				if err := json.NewDecoder(resp.Body).Decode(&data); err == nil {
					count := 0
					for _, r := range data.Releases {
						v := strings.TrimPrefix(r, "jdk-")
						allVersions = append(allVersions, "adoptium-"+v)
						count++
						if count >= 5 { // Show top 5 stable updates of each major version
							break
						}
					}
				}
			}
		}
	}

	// 2. Fetch Azul Zulu OpenJDK versions
	zuluURL := "https://api.azul.com/metadata/v1/zulu/packages/?os=windows&arch=amd64&archive_type=zip&java_package_type=jdk&release_status=ga&latest=true&page_size=50"
	zuluResp, err := http.Get(zuluURL)
	if err == nil {
		defer zuluResp.Body.Close()
		if zuluResp.StatusCode == http.StatusOK {
			var pkgs []zuluPackage
			if err := json.NewDecoder(zuluResp.Body).Decode(&pkgs); err == nil {
				for _, pkg := range pkgs {
					// Filter to standard JDK (skip FX, CRaC)
					if strings.Contains(pkg.Name, "-ca-jdk") {
						vParts := []string{}
						for _, num := range pkg.JavaVersion {
							vParts = append(vParts, fmt.Sprintf("%d", num))
						}
						vStr := strings.Join(vParts, ".")
						allVersions = append(allVersions, "zulu-"+vStr)
					}
				}
			}
		}
	}

	// 3. Add Microsoft Build of OpenJDK stable versions
	for _, mv := range []string{"25", "21", "17", "11"} {
		allVersions = append(allVersions, "microsoft-"+mv)
	}

	// 4. Add Oracle JDK stable versions
	for _, mv := range []string{"23", "21", "17"} {
		allVersions = append(allVersions, "oracle-"+mv)
	}

	return allVersions, nil
}

func (j *JavaSDK) Install(version string, baseDir string) error {
	version = strings.TrimSpace(version)

	var downloadURL string
	var provider string
	var cleanVersion string

	if strings.HasPrefix(version, "zulu-") {
		provider = "zulu"
		cleanVersion = strings.TrimPrefix(version, "zulu-")
	} else if strings.HasPrefix(version, "adoptium-") {
		provider = "adoptium"
		cleanVersion = strings.TrimPrefix(version, "adoptium-")
	} else if strings.HasPrefix(version, "microsoft-") {
		provider = "microsoft"
		cleanVersion = strings.TrimPrefix(version, "microsoft-")
	} else if strings.HasPrefix(version, "oracle-") {
		provider = "oracle"
		cleanVersion = strings.TrimPrefix(version, "oracle-")
	} else {
		// Default to adoptium if no provider prefix
		provider = "adoptium"
		cleanVersion = version
	}

	if provider == "zulu" {
		// We need to query the Azul Zulu metadata API to get the download URL for the clean version
		// Clean version is e.g. "21.0.2"
		zuluAPI := fmt.Sprintf("https://api.azul.com/metadata/v1/zulu/packages/?os=windows&arch=amd64&archive_type=zip&java_package_type=jdk&release_status=ga&latest=true&page_size=100")
		resp, err := http.Get(zuluAPI)
		if err != nil {
			return fmt.Errorf("failed to fetch Zulu metadata: %v", err)
		}
		defer resp.Body.Close()

		if resp.StatusCode != http.StatusOK {
			return fmt.Errorf("zulu API error: %s", resp.Status)
		}

		var pkgs []zuluPackage
		if err := json.NewDecoder(resp.Body).Decode(&pkgs); err != nil {
			return fmt.Errorf("failed to parse Zulu metadata: %v", err)
		}

		for _, pkg := range pkgs {
			if strings.Contains(pkg.Name, "-ca-jdk") {
				vParts := []string{}
				for _, num := range pkg.JavaVersion {
					vParts = append(vParts, fmt.Sprintf("%d", num))
				}
				vStr := strings.Join(vParts, ".")
				if vStr == cleanVersion {
					downloadURL = pkg.DownloadURL
					break
				}
			}
		}

		if downloadURL == "" {
			return fmt.Errorf("azul zulu version %s not found. Use 'list-remote java' to check versions", cleanVersion)
		}

	} else if provider == "microsoft" {
		downloadURL = fmt.Sprintf("https://aka.ms/download-jdk/microsoft-jdk-%s-windows-x64.zip", cleanVersion)
	} else if provider == "oracle" {
		downloadURL = fmt.Sprintf("https://download.oracle.com/java/%s/latest/jdk-%s_windows-x64_bin.zip", cleanVersion, cleanVersion)
	} else {
		// Adoptium
		isMajor := true
		for _, char := range cleanVersion {
			if char < '0' || char > '9' {
				isMajor = false
				break
			}
		}

		if isMajor {
			downloadURL = fmt.Sprintf("https://api.adoptium.net/v3/binary/latest/%s/ga/windows/x64/jdk/hotspot/normal/eclipse?project=jdk", cleanVersion)
		} else {
			jdkVersion := cleanVersion
			if !strings.HasPrefix(jdkVersion, "jdk") {
				jdkVersion = "jdk-" + jdkVersion
			}
			downloadURL = fmt.Sprintf("https://api.adoptium.net/v3/binary/version/%s/windows/x64/jdk/hotspot/normal/eclipse?project=jdk", url.QueryEscape(jdkVersion))
		}
	}

	fmt.Printf("正在从 %s 下载 Java JDK (供应源: %s, 版本: %s)...\n", downloadURL, provider, cleanVersion)

	tempDir, cleanup, err := SetupTempDir(baseDir, "java")
	if err != nil {
		return err
	}
	defer cleanup()

	zipFile := filepath.Join(tempDir, "java.zip")
	if err := DownloadFile(downloadURL, zipFile); err != nil {
		return fmt.Errorf("download failed: %v", err)
	}

	fmt.Println("正在解压 Java JDK 压缩包...")
	extractDir := filepath.Join(tempDir, "extracted")
	if err := Unzip(zipFile, extractDir); err != nil {
		return fmt.Errorf("unzip failed: %v", err)
	}

	// Install to versions/java/<version> (using the full version name with provider prefix, e.g., adoptium-17.0.2+8 or zulu-17.0.10)
	destDir := filepath.Join(globalConfig.VersionsDir, j.Name(), version)
	fmt.Printf("正在安装 Java JDK 到 %s...\n", destDir)
	if err := MoveExtractToDest(extractDir, destDir); err != nil {
		return err
	}

	fmt.Printf("Java JDK %s 安装成功！\n", version)
	return nil
}
