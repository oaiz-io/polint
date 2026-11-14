// @exports
function double(n) { // @fn double
  return n * 2;
}

function sum(a, b) { // @fn sum
  return a + b;
}

const total = sum(double(2), 3); // @calls sum double
const handlers = {
  run: () => sum(total, 1), // @fn run @calls sum
};
const result = handlers.run(); // @calls run
