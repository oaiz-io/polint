// @exports
function init(inst) { // @fn init
  return inst;
}

function helper() { // @fn helper
  return "help";
}

function Base() {} // @fn Base

Object.defineProperty(Base, "init", { value: init });
Base.helper = helper;
Base.init({}); // @calls init
Base.helper(); // @calls helper
