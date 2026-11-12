// @exports
function double(x) { // @fn double
  return x * 2;
}
function pick(mode, fallback) { // @fn pick
  if (mode === "double") return double;
  if (mode === "wrap") {
    return function wrapped(x) { // @fn wrapped
      return [x];
    };
  }
  return fallback;
}
function negate(x) { // @fn negate
  return -x;
}
for (const mode of ["double", "wrap", "other"]) {
  const fn = pick(mode, negate); // @calls pick
  fn(1); // @calls double wrapped negate
}
