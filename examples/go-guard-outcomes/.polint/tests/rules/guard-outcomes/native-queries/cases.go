// Package guardoutcomes is a synthetic corpus for guard-outcome policy queries.
//
// CheckAccess is the guard: it returns a non-nil error when the actor may not
// act. SaveRecord is the protected operation. Every function below is one
// documented control-flow shape, chosen so the corpus covers proved, refuted,
// and undecidable answers. None of it is real application code.
package guardoutcomes

import (
	"context"
	"errors"
	"fmt"
)

// Actor is the subject an authorization check decides about.
type Actor struct {
	Name     string
	TenantID string
}

// CheckAccess reports whether actor may act.
func CheckAccess(ctx context.Context, actor Actor) error {
	if actor.Name == "" {
		return errors.New("anonymous actor")
	}
	return nil
}

// SaveRecord is the protected operation.
func SaveRecord(ctx context.Context, actor Actor, record string) {
	_ = ctx
	_ = actor
	_ = record
}

func loadRecord(record string) error {
	if record == "" {
		return errors.New("empty record")
	}
	return nil
}

func logf(format string, args ...any) {
	_ = fmt.Sprintf(format, args...)
}

func withTransaction(ctx context.Context, body func() error) error {
	_ = ctx
	return body()
}

// Covered: the guard's error is tested and the error path returns.
func validGuard(ctx context.Context, actor Actor, record string) error {
	if err := CheckAccess(ctx, actor); err != nil {
		return err
	}
	SaveRecord(ctx, actor, record)
	return nil
}

// Violation: no guard call in this function.
func missingGuard(ctx context.Context, actor Actor, record string) {
	SaveRecord(ctx, actor, record)
}

// Violation: the guard runs after the protected operation.
func writeBeforeGuard(ctx context.Context, actor Actor, record string) error {
	SaveRecord(ctx, actor, record)
	if err := CheckAccess(ctx, actor); err != nil {
		return err
	}
	return nil
}

// Violation: the guard runs but its returned error is never tested.
func ignoredGuardError(ctx context.Context, actor Actor, record string) {
	_ = CheckAccess(ctx, actor)
	SaveRecord(ctx, actor, record)
}

// Violation: the guard only runs on one branch, so it does not dominate.
func conditionalGuard(ctx context.Context, actor Actor, record string, checked bool) error {
	if checked {
		if err := CheckAccess(ctx, actor); err != nil {
			return err
		}
	}
	SaveRecord(ctx, actor, record)
	return nil
}

// Violation: the error is tested but the error path falls through.
func logsErrorWithoutExit(ctx context.Context, actor Actor, record string) {
	if err := CheckAccess(ctx, actor); err != nil {
		logf("access denied: %v", err)
	}
	SaveRecord(ctx, actor, record)
}

// Unknown: the tested value was overwritten by another call's error.
func checksDifferentError(ctx context.Context, actor Actor, record string) error {
	err := CheckAccess(ctx, actor)
	if other := loadRecord(record); other != nil {
		err = other
	}
	if err != nil {
		return err
	}
	SaveRecord(ctx, actor, record)
	return nil
}

// Covered on control flow, unchecked on identity: a different actor is authorized.
func authorizesDifferentActor(ctx context.Context, actor Actor, other Actor, record string) error {
	if err := CheckAccess(ctx, other); err != nil {
		return err
	}
	SaveRecord(ctx, actor, record)
	return nil
}

// Covered on control flow, unchecked on identity: the actor is replaced after the guard.
func replacesActor(ctx context.Context, actor Actor, other Actor, record string) error {
	if err := CheckAccess(ctx, actor); err != nil {
		return err
	}
	actor = other
	SaveRecord(ctx, actor, record)
	return nil
}

// Violation: the guard sits in a closure that never runs.
func guardInUnusedClosure(ctx context.Context, actor Actor, record string) {
	_ = func() error { return CheckAccess(ctx, actor) }
	SaveRecord(ctx, actor, record)
}

// Covered: guard and operation share the callback body.
func validTransactionCallback(ctx context.Context, actor Actor, record string) error {
	return withTransaction(ctx, func() error {
		if err := CheckAccess(ctx, actor); err != nil {
			return err
		}
		SaveRecord(ctx, actor, record)
		return nil
	})
}

// Violation: no guard, and a request-supplied tenant reaches the operation.
func aliasTenantFlow(ctx context.Context, actor Actor, requestedTenant string, record string) {
	tenant := requestedTenant
	actor.TenantID = tenant
	SaveRecord(ctx, actor, record)
}

// Violation: no guard, and the nearby tenant value never reaches the operation.
func unrelatedNearbyTenant(ctx context.Context, actor Actor, requestedTenant string, record string) {
	_ = requestedTenant
	SaveRecord(ctx, actor, record)
}

// No operation: nothing to decide about.
func emptyFunction(ctx context.Context, actor Actor) {
	_ = ctx
	_ = actor
}
