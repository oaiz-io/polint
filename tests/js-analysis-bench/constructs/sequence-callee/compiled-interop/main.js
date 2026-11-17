// @exports run
"use strict";

Object.defineProperty(exports, "__esModule", { value: true });
exports.run = run;
var _double = _interopRequireDefault(require("./double.js")); // @calls interop
var _math = require("./math.js");
function _interopRequireDefault(e) { return e && e.__esModule ? e : { default: e }; } // @fn interop
function run(a) { // @fn run
  return (0, _math.increment)((0, _double.default)(a)); // @calls increment double
}
run(1); // @calls run
