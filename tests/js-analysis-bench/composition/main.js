// @exports
const liba = require("liba");
const libb = require("libb");

function onItem(item) { // @fn onItem
  return item;
}

function onHook() { // @fn onHook
  return "hook";
}

liba.Widget.prototype.render = function patched() { // @fn patched
  return "patched:" + this.name;
};

liba.each([1, 2], onItem); // @calls each
const next = libb.makeCounter(); // @calls makeCounter
next(); // @calls increment
liba.readHook({ hook: onHook }); // @calls readHook
liba.show(new liba.Widget("w")); // @calls show Widget
liba.runLocal(liba.local); // @calls runLocal
