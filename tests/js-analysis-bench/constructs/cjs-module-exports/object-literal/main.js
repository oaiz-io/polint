// @exports
const math = require("./math"); // @external
const { add } = require("./math"); // @external

math.mul(2, 3); // @calls mul
add(1, 2); // @calls add
