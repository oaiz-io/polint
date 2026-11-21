// @exports check
function tagTester(name) { // @fn tagTester
  return function (obj) { // @fn tagTest
    return Object.prototype.toString.call(obj) === "[object " + name + "]"; // @external
  };
}

var isFunction = tagTester("Function"); // @calls tagTester
if (typeof /./ !== "function") {
  isFunction = function (obj) { // @fn plainIsFunction
    return typeof obj === "function";
  };
}

function check(obj) { // @fn check
  return isFunction(obj); // @calls plainIsFunction
}

module.exports = { check: check };

check(check); // @calls check
