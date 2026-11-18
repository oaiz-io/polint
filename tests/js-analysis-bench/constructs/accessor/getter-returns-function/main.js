// @exports
function handle(value) { // @fn handle
  return value;
}

const source = {
  get handler() { // @fn handler
    return handle;
  },
};

source.handler(1); // @calls handler handle
