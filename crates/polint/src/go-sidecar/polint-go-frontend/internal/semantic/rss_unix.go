//go:build unix

package semantic

import (
	"runtime"
	"syscall"
)

// peakRSSBytes is this process's resident-set high-water mark as the kernel
// reports it. Unlike the heap figure it is a true high-water mark, and it counts
// everything the process holds: heap, stacks, runtime overhead, and memory the
// collector has not yet returned to the operating system.
func peakRSSBytes() uint64 {
	var usage syscall.Rusage
	if err := syscall.Getrusage(syscall.RUSAGE_SELF, &usage); err != nil || usage.Maxrss <= 0 {
		return 0
	}
	maxRSS := uint64(usage.Maxrss)
	// Darwin reports bytes; every other Unix kernel reports kibibytes.
	if runtime.GOOS == "darwin" || runtime.GOOS == "ios" {
		return maxRSS
	}
	return maxRSS * 1024
}
