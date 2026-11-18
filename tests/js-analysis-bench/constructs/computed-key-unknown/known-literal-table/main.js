// @exports
function getXhr(config) { // @fn getXhr
  return `xhr ${config}`;
}

function getHttp(config) { // @fn getHttp
  return `http ${config}`;
}

const known = { xhr: { get: getXhr }, http: { get: getHttp } };

function choose(name) { // @fn choose
  const adapter = known[String(name).toLowerCase()];
  return adapter.get(name); // @calls getXhr getHttp
}

choose("XHR"); // @calls choose
choose("http"); // @calls choose
