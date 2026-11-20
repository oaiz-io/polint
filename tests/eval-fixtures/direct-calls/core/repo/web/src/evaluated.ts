// A direct `eval` can write any binding of its file, so it lives apart from
// the calls whose single target the fixture expects.
export function evaluated(): void {
  // POLINT-FEATURE direct-calls/ts/eval
  eval("direct-call fixture");
}
