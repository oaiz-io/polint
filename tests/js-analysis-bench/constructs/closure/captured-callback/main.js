// @exports
function createEmitter() { // @fn createEmitter
  let listener = null;
  return {
    on(fn) { // @fn on
      listener = fn;
    },
    emit(value) { // @fn emit
      return listener(value); // @calls onEvent
    },
  };
}

function onEvent(value) { // @fn onEvent
  return value;
}

const emitter = createEmitter(); // @calls createEmitter
emitter.on(onEvent); // @calls on
emitter.emit(5); // @calls emit
