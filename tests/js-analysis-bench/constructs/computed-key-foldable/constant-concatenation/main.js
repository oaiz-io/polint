// @exports
function onOpen() { // @fn onOpen
  return "open";
}

function onClose() { // @fn onClose
  return "close";
}

const PREFIX = "on";
const CLOSE = "Close";
const hooks = {};
hooks[PREFIX + "Open"] = onOpen;
hooks[PREFIX + CLOSE] = onClose;
hooks.onOpen(); // @calls onOpen
hooks["on" + "Close"](); // @calls onClose
