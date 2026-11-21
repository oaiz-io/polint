// @exports total
export let total = 0;

function step(n) { // @fn step
  return n + 1;
}

!function () { // @fn bang @calls bang
  total = step(total); // @calls step
}();

void function () { // @fn voided @calls voided
  total = step(total); // @calls step
}();

+function () { // @fn plus @calls plus
  total = step(total); // @calls step
}();

typeof function () { // @fn typed @calls typed
  total = step(total); // @calls step
}();
