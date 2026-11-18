// @exports
function first() { // @fn first
  return 1;
}

function second() { // @fn second
  return 2;
}

function install(target) { // @fn install
  target.run = second;
}

const table = {};
table["first-handler"] = first;
table[2] = second;
table["first-handler"](); // @calls first
table[2](); // @calls second
const holder = {};
install(holder); // @calls install
holder.run(); // @calls second
