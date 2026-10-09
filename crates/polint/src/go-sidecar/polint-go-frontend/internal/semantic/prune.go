package semantic

import (
	"go/constant"
	"go/types"
	"sort"
	"strconv"

	"golang.org/x/tools/go/ssa"
)

// prunedCalls are the calls a constant branch condition removed from fn.
type prunedCalls struct {
	fn    *ssa.Function
	calls []ssa.CallInstruction
}

// pruneProgram prunes every body of the analysed packages (see
// pruneConstantBranches), before anything reads them, and returns the removed
// calls by the function they are written in, ordered by function name.
func pruneProgram(prog *ssa.Program, pkgs []*ssa.Package) []prunedCalls {
	var out []prunedCalls
	for _, fn := range prunableFunctions(prog, pkgs) {
		if calls := pruneConstantBranches(fn); len(calls) > 0 {
			out = append(out, prunedCalls{fn: fn, calls: calls})
		}
	}
	sort.SliceStable(out, func(i, j int) bool { return functionName(out[i].fn) < functionName(out[j].fn) })
	return out
}

// emitDeadCalls emits a `dead_call` row for each call a constant branch
// condition removed, at the span its `callsite` row would have had, so the
// call layer can tell a call that cannot run from one it failed to resolve.
func (e *emitter) emitDeadCalls(pruned []prunedCalls) {
	for _, entry := range pruned {
		index := newCallIndex(entry.fn.Syntax())
		for _, call := range entry.calls {
			syntax := index.syntaxFor(call)
			if syntax == nil {
				continue
			}
			pos := e.positionSpan(syntax.Pos(), syntax.End())
			if pos == nil {
				continue
			}
			file := posFile(e.fset, syntax.Pos(), e.root)
			e.add(Row{
				"kind":         "dead_call",
				"package_id":   packageID(entry.fn.Pkg),
				"package_path": packagePath(entry.fn.Pkg),
				"caller":       functionName(entry.fn),
				"file":         file,
				"span":         pos,
				"stable_key": stableKey(
					packageID(entry.fn.Pkg),
					functionName(entry.fn),
					file,
					strconv.Itoa(pos.StartByte),
					strconv.Itoa(pos.EndByte),
					"dead",
				),
			})
		}
	}
}

// pruneConstantBranches removes from fn the code that a constant branch
// condition rules out, such as the body of `if false { ... }` or of
// `if debug { ... }` with `const debug = false`. go/ssa keeps such blocks on
// purpose, for fidelity to the source, but nothing in them runs in the analysed
// build: a call there is not a call, and a value assigned there flows nowhere.
//
// A block that ends in an `if` on a constant keeps only the successor the
// constant selects. Blocks no longer reachable from the entry (or from the
// recover block) are removed, the remaining blocks are renumbered in their
// original order, and each remaining block forgets the predecessors whose edge
// was removed, together with the matching phi edges. Analyses that only walk
// fn.Blocks, such as variable-type analysis, then never see the pruned code.
//
// It returns the call instructions of the removed blocks, in block order.
func pruneConstantBranches(fn *ssa.Function) []ssa.CallInstruction {
	if len(fn.Blocks) == 0 {
		return nil
	}
	successors := make(map[*ssa.BasicBlock][]*ssa.BasicBlock, len(fn.Blocks))
	constantBranch := false
	for _, block := range fn.Blocks {
		successors[block] = block.Succs
		if taken, ok := constantSuccessor(block); ok {
			successors[block] = []*ssa.BasicBlock{taken}
			constantBranch = true
		}
	}
	if !constantBranch {
		return nil
	}

	live := make(map[*ssa.BasicBlock]bool, len(fn.Blocks))
	var stack []*ssa.BasicBlock
	visit := func(block *ssa.BasicBlock) {
		if block != nil && !live[block] {
			live[block] = true
			stack = append(stack, block)
		}
	}
	visit(fn.Blocks[0])
	visit(fn.Recover)
	for len(stack) > 0 {
		block := stack[len(stack)-1]
		stack = stack[:len(stack)-1]
		for _, next := range successors[block] {
			visit(next)
		}
	}

	edgeKept := func(from, to *ssa.BasicBlock) bool {
		if !live[from] {
			return false
		}
		for _, next := range successors[from] {
			if next == to {
				return true
			}
		}
		return false
	}
	var dead []ssa.CallInstruction
	var kept []*ssa.BasicBlock
	for _, block := range fn.Blocks {
		if !live[block] {
			for _, instr := range block.Instrs {
				if call, ok := instr.(ssa.CallInstruction); ok {
					dead = append(dead, call)
				}
			}
			continue
		}
		var preds []*ssa.BasicBlock
		var keepEdge []bool
		for _, pred := range block.Preds {
			keep := edgeKept(pred, block)
			keepEdge = append(keepEdge, keep)
			if keep {
				preds = append(preds, pred)
			}
		}
		if len(preds) != len(block.Preds) {
			for _, instr := range block.Instrs {
				phi, ok := instr.(*ssa.Phi)
				if !ok {
					break
				}
				var edges []ssa.Value
				for i, edge := range phi.Edges {
					if i < len(keepEdge) && keepEdge[i] {
						edges = append(edges, edge)
					}
				}
				phi.Edges = edges
			}
			block.Preds = preds
		}
		block.Succs = successors[block]
		block.Index = len(kept)
		kept = append(kept, block)
	}
	fn.Blocks = kept
	return dead
}

// constantSuccessor is the one successor a block ending in an `if` on a
// boolean constant can take.
func constantSuccessor(block *ssa.BasicBlock) (*ssa.BasicBlock, bool) {
	if len(block.Instrs) == 0 || len(block.Succs) != 2 {
		return nil, false
	}
	branch, ok := block.Instrs[len(block.Instrs)-1].(*ssa.If)
	if !ok {
		return nil, false
	}
	condition, ok := branch.Cond.(*ssa.Const)
	if !ok || condition.Value == nil || condition.Value.Kind() != constant.Bool {
		return nil, false
	}
	if constant.BoolVal(condition.Value) {
		return block.Succs[0], true
	}
	return block.Succs[1], true
}

// prunableFunctions are the bodies of the analysed packages: their functions,
// the methods declared on their types, the closures inside those, and the
// generic instances they call or take as values, transitively. Collecting them
// builds no SSA wrapper, so the program's runtime type set, and with it every
// later row, is the same as without pruning.
func prunableFunctions(prog *ssa.Program, pkgs []*ssa.Package) []*ssa.Function {
	seen := make(map[*ssa.Function]bool)
	var out []*ssa.Function
	var add func(fn *ssa.Function)
	add = func(fn *ssa.Function) {
		if fn == nil || seen[fn] {
			return
		}
		seen[fn] = true
		out = append(out, fn)
		for _, anon := range fn.AnonFuncs {
			add(anon)
		}
	}
	for _, pkg := range pkgs {
		if pkg == nil || pkg.Pkg == nil {
			continue
		}
		names := make([]string, 0, len(pkg.Members))
		for name := range pkg.Members {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			switch member := pkg.Members[name].(type) {
			case *ssa.Function:
				add(member)
			case *ssa.Type:
				named, ok := member.Type().(*types.Named)
				if !ok {
					continue
				}
				for i := 0; i < named.NumMethods(); i++ {
					add(prog.FuncValue(named.Method(i)))
				}
			}
		}
	}
	// Generic instances are bodies of their own; follow the ones the collected
	// bodies reach, transitively.
	for i := 0; i < len(out); i++ {
		for _, block := range out[i].Blocks {
			for _, instr := range block.Instrs {
				if call, ok := instr.(ssa.CallInstruction); ok {
					if callee := call.Common().StaticCallee(); callee != nil && callee.Origin() != nil {
						add(callee)
					}
				}
				for _, operand := range instr.Operands(nil) {
					if operand == nil {
						continue
					}
					if fn, ok := (*operand).(*ssa.Function); ok && fn.Origin() != nil {
						add(fn)
					}
				}
			}
		}
	}
	return out
}
