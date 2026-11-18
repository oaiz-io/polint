// @exports
function emit(event) { // @fn emit
  return event;
}

function fire(target) { // @fn fire
  return target?.events?.emit("x"); // @calls emit
}

fire({ events: { emit } }); // @calls fire
fire(undefined); // @calls fire
