// @exports
function load() { // @fn load
  return Promise.resolve(2); // @external
}

function scale(value) { // @fn scale
  return value * 10;
}

function report(error) { // @fn report
  return String(error);
}

const loaded = load(); // @calls load
const scaled = loaded.then(scale); // @callback scale
scaled.catch(report); // @external
