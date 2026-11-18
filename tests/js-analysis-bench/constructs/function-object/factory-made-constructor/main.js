// @exports
const { $make } = require("./make"); // @external

function initError(inst) { // @fn initError
  return inst;
}

const ErrorType = $make(initError); // @calls $make
ErrorType.init({}); // @calls initError
new ErrorType(); // @calls made
