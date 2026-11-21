// @exports
function alpha() { // @fn alpha
  return "a";
}

function beta() { // @fn beta
  return "b";
}

function callAll(...fns) { // @fn callAll
  return fns.map((fn) => fn()); // @fn callEach @calls alpha beta @callback callEach
}

function pair(x, y) { // @fn pair
  x(); // @calls alpha
  return y(); // @calls beta
}

callAll(alpha, beta); // @calls callAll
const args = [alpha, beta];
pair(...args); // @calls pair
