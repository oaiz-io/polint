// @exports next reset
let counter = 0;

function next() { // @fn next
  counter += 1;
  return counter;
}

function reset() { // @fn reset
  counter = 0;
}

exports.next = next;
exports.reset = reset;
