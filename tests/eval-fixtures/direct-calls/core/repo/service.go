package directcalls

import "reflect"

type worker interface {
	Work(int) int
}

type concreteWorker struct{}

func directFunction(value int) int {
	return value + 1
}

func (concreteWorker) Work(value int) int {
	// POLINT-FEATURE direct-calls/go/direct-function
	return directFunction(value)
}

func reflectInvoke(target any) string {
	// POLINT-FEATURE direct-calls/go/reflection
	// the reflection entry point is a dependency function; the method called on its result
	// has implementations only inside the dependency.
	return reflect.TypeOf(target).String()
}

func dispatch(candidate worker, value int) int {
	// POLINT-FEATURE direct-calls/go/interface-dispatch
	// no caller passes a concrete type, so the candidates come from the type hierarchy.
	return candidate.Work(value)
}

func apply(fn func(int) int, value int) int {
	// POLINT-FEATURE direct-calls/go/function-value
	return fn(value)
}

func Process(worker concreteWorker, maybe worker, value int) int {
	first := directFunction(value)
	// POLINT-FEATURE direct-calls/go/method-call
	second := worker.Work(first)
	third := apply(directFunction, second)
	// POLINT-FEATURE direct-calls/go/goroutine-boundary
	go directFunction(third)
	reflectInvoke(worker)
	return dispatch(maybe, third)
}
