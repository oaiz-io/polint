// @exports
function fallback(x) { // @fn fallback
  return x;
}
function custom(x) { // @fn custom
  return x + 1;
}
function ternary(flag, fn) { // @fn ternary
  return flag ? fn : fallback;
}
function orDefault(fn) { // @fn orDefault
  return fn || fallback;
}
function nullishDefault(fn) { // @fn nullishDefault
  return fn ?? fallback;
}
for (const flag of [true, false]) {
  const a = ternary(flag, custom); // @calls ternary
  a(1); // @calls custom fallback
  const b = orDefault(flag ? custom : null); // @calls orDefault
  b(2); // @calls custom fallback
  const c = nullishDefault(flag ? custom : undefined); // @calls nullishDefault
  c(3); // @calls custom fallback
}
