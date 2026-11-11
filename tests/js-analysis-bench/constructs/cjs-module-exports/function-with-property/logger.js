// @exports default format
module.exports = function log(message) { // @fn log
  return module.exports.format(message); // @calls format
};

module.exports.format = function format(message) { // @fn format
  return "[log] " + message;
};
