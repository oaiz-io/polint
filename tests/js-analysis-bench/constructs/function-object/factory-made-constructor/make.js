// @exports $make
function $make(initializer) { // @fn $make
  function made() {} // @fn made
  Object.defineProperty(made, "init", { value: initializer });
  return made;
}

module.exports = { $make };
