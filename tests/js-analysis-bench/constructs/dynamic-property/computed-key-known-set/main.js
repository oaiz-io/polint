// @exports
const ops = {
  add(a, b) { // @fn add
    return a + b;
  },
  sub(a, b) { // @fn sub
    return a - b;
  },
};

for (const name of ["add", "sub"]) {
  ops[name](4, 2); // @calls add sub
}
