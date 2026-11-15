// @exports upper shout
exports.upper = function upper(text) { // @fn upper
  return text.toUpperCase(); // @external
};

exports.shout = function shout(text) { // @fn shout
  return exports.upper(text) + "!"; // @calls upper
};
