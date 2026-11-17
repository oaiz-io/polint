// @exports settings defaults
function make(name) { // @fn make
  return { name };
}

function freeze(value) { // @fn freeze
  return Object.freeze(value);
}

exports.settings = make("settings"); // @calls make
module.exports.defaults = freeze(make("defaults")); // @calls freeze make
