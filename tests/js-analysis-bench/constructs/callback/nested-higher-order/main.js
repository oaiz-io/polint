// @exports
function double(n) { // @fn double
  return n * 2;
}

function apply(values, fn) { // @fn apply
  return values.map(fn); // @callback double
}

function applyTwice(values, fn) { // @fn applyTwice
  return apply(apply(values, fn), fn); // @calls apply
}

applyTwice([1, 2], double); // @calls applyTwice
