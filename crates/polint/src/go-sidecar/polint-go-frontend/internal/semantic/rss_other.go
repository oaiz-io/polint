//go:build !unix

package semantic

// peakRSSBytes reports 0 where the platform exposes no resident-set high-water
// mark to this process; the stage log then shows only the heap figure.
func peakRSSBytes() uint64 {
	return 0
}
