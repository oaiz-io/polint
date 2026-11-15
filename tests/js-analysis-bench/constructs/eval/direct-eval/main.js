// @exports
function secret() { // @fn secret
  return 42;
}

const code = "sec" + "ret()";
eval(code); // @unknown
