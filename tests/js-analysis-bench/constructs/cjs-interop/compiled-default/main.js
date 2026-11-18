// @exports
"use strict";
var __importDefault = function (mod) { // @fn importDefault
  return mod && mod.__esModule ? mod : { default: mod };
};
const greet_1 = __importDefault(require("./greet")); // @calls importDefault
(0, greet_1.default)("a"); // @calls greet
