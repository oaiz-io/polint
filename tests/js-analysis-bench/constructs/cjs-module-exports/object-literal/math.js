// @exports add mul
function add(a, b) { // @fn add
  return a + b;
}

function mul(a, b) { // @fn mul
  let total = 0;
  for (let i = 0; i < b; i += 1) {
    total = add(total, a); // @calls add
  }
  return total;
}

module.exports = { add, mul };
