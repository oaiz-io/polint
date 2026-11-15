// @exports
function square(n) { // @fn square
  return n * n;
}

function isEven(n) { // @fn isEven
  return n % 2 === 0;
}

const values = [1, 2, 3, 4];
const squares = values.map(square); // @callback square
squares.filter(isEven).forEach((n) => isEven(n)); // @callback isEven arrow @fn arrow @calls isEven
