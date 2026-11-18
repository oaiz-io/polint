// @exports
function fallback(value) { // @fn fallback
  return value;
}

function primary(value) { // @fn primary
  return value + 1;
}

function left() { // @fn left
  return "l";
}

function right() { // @fn right
  return "r";
}

function pick(handler) { // @fn pick
  const run = handler || fallback;
  return run(1); // @calls primary fallback
}

function choose(flag, a, b) { // @fn choose
  const chosen = flag ? a : b;
  return chosen(); // @calls left right
}

const missing = null;
const viaNullish = missing ?? left;
viaNullish(); // @calls left
const viaComma = (0, right);
viaComma(); // @calls right
const viaAnd = primary && fallback;
viaAnd(2); // @calls fallback
pick(primary); // @calls pick
pick(undefined); // @calls pick
choose(true, left, right); // @calls choose
choose(false, left, right); // @calls choose
