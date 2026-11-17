// @exports render
const util = require("./vendor/util/index.js");

function render(items) { // @fn render
  return util.join(util.compact(items)); // @calls join compact
}

module.exports.render = render;
render(["a", "", "b"]); // @calls render
