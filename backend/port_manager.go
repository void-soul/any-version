package main

import (
	"bytes"
	"fmt"
	"os/exec"
	"strconv"
	"strings"
)

type PortOwner struct {
	Port        string
	Pid         string
	ProcessName string
}

type PortStatus struct {
	Port     int
	Free     bool   // true if completely free
	Reserved bool   // true if in Windows excluded port range
	Occupied bool   // true if currently LISTENING
	Owner    *PortOwner
}

// Windows reserved port range
type ExcludedPortRange struct {
	Start int
	End   int
}

// GetExcludedPortRanges retrieves Windows reserved (excluded) port ranges via netsh
func GetExcludedPortRanges() ([]ExcludedPortRange, error) {
	cmd := exec.Command("netsh", "int", "ipv4", "show", "excludedportrange", "protocol=tcp")
	var out bytes.Buffer
	cmd.Stdout = &out
	if err := cmd.Run(); err != nil {
		return nil, fmt.Errorf("failed to query excluded port ranges: %v", err)
	}

	var ranges []ExcludedPortRange
	lines := strings.Split(out.String(), "\n")
	inTable := false

	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		// Look for table header separator
		if strings.Contains(line, "---") {
			inTable = true
			continue
		}
		if !inTable {
			continue
		}

		fields := strings.Fields(line)
		if len(fields) < 2 {
			continue
		}

		start, err1 := strconv.Atoi(fields[0])
		end, err2 := strconv.Atoi(fields[1])
		if err1 != nil || err2 != nil {
			continue
		}

		ranges = append(ranges, ExcludedPortRange{Start: start, End: end})
	}

	return ranges, nil
}

// IsPortReserved checks if a port falls within any Windows excluded port range
func IsPortReserved(port int, ranges []ExcludedPortRange) bool {
	for _, r := range ranges {
		if port >= r.Start && port <= r.End {
			return true
		}
	}
	return false
}

// CheckPortStatus returns comprehensive port status including reserved range info
func CheckPortStatus(portStr string) (*PortStatus, error) {
	port, err := strconv.Atoi(portStr)
	if err != nil {
		return nil, fmt.Errorf("invalid port number: %s", portStr)
	}

	status := &PortStatus{Port: port, Free: true}

	// 1. Check if actively occupied (netstat)
	owner, netErr := FindPortOwner(portStr)
	if netErr == nil && owner != nil {
		status.Occupied = true
		status.Free = false
		status.Owner = owner
	}

	// 2. Check if in Windows reserved range
	ranges, rangeErr := GetExcludedPortRanges()
	if rangeErr == nil && IsPortReserved(port, ranges) {
		status.Reserved = true
		if !status.Occupied {
			status.Free = false
		}
	}

	return status, nil
}

// FindPortOwner scans netstat and tasklist to identify the process occupying a TCP port
func FindPortOwner(port string) (*PortOwner, error) {
	cmd := exec.Command("netstat", "-ano", "-p", "tcp")
	var out bytes.Buffer
	cmd.Stdout = &out
	if err := cmd.Run(); err != nil {
		return nil, fmt.Errorf("failed to run netstat: %v", err)
	}

	lines := strings.Split(out.String(), "\n")
	var pid string

	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" || !strings.HasPrefix(strings.ToUpper(line), "TCP") {
			continue
		}

		fields := strings.Fields(line)
		if len(fields) < 5 {
			continue
		}

		localAddr := fields[1]
		state := fields[3]
		rowPid := fields[4]

		// Port is at the end of the local address
		rowPort := ""
		if strings.Contains(localAddr, "]") {
			// IPv6 e.g., [::]:3306
			parts := strings.Split(localAddr, "]:")
			if len(parts) == 2 {
				rowPort = parts[1]
			}
		} else {
			// IPv4 e.g., 0.0.0.0:3306
			parts := strings.Split(localAddr, ":")
			if len(parts) > 0 {
				rowPort = parts[len(parts)-1]
			}
		}

		if rowPort == port && state == "LISTENING" {
			pid = rowPid
			break
		}
	}

	if pid == "" {
		return nil, fmt.Errorf("端口 %s 当前未被占用", port)
	}

	// Query tasklist to find process name
	nameCmd := exec.Command("tasklist", "/fi", fmt.Sprintf("pid eq %s", pid), "/fo", "csv", "/nh")
	var nameOut bytes.Buffer
	nameCmd.Stdout = &nameOut
	_ = nameCmd.Run()

	procName := "Unknown"
	csvLine := strings.TrimSpace(nameOut.String())
	if csvLine != "" {
		parts := strings.Split(csvLine, ",")
		if len(parts) > 0 {
			// Strip quotes e.g. "nginx.exe" -> nginx.exe
			procName = strings.Trim(parts[0], "\"")
		}
	}

	return &PortOwner{
		Port:        port,
		Pid:         pid,
		ProcessName: procName,
	}, nil
}

// KillPortOwner terminates the process occupying a TCP port
func KillPortOwner(port string) error {
	// First check if port is in reserved range
	portNum, _ := strconv.Atoi(port)
	ranges, _ := GetExcludedPortRanges()
	if IsPortReserved(portNum, ranges) {
		return fmt.Errorf("端口 %s 位于 Windows 系统保留端口范围内，无法释放（请使用 netsh 管理保留端口或更换端口号）", port)
	}

	owner, err := FindPortOwner(port)
	if err != nil {
		return err
	}

	killCmd := exec.Command("taskkill", "/f", "/pid", owner.Pid)
	if output, err := killCmd.CombinedOutput(); err != nil {
		return fmt.Errorf("failed to kill process: %v (output: %s)", err, string(output))
	}

	return nil
}
