// @exports stamp
var now = function () { // @fn counterNow
  return 0;
};
if (typeof Date.now === "function") {
  now = function () { // @fn dateNow
    return Date.now(); // @external
  };
}

function stamp(label) { // @fn stamp
  return label + ":" + now(); // @calls dateNow
}

module.exports = { stamp: stamp };

stamp("start"); // @calls stamp
