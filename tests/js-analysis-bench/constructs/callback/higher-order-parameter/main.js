// @exports
function twice(fn, value) { // @fn twice
  return fn(fn(value)); // @calls double
}

function double(n) { // @fn double
  return n * 2;
}

function compose(f, g) { // @fn compose
  return (x) => f(g(x)); // @fn composed @calls double increment
}

function increment(n) { // @fn increment
  return n + 1;
}

twice(double, 3); // @calls twice
const both = compose(double, increment); // @calls compose
both(1); // @calls composed
