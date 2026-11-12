// @exports config
function defaultName() { // @fn defaultName
  return "app";
}

function makeConfig(name) { // @fn makeConfig
  return { name };
}

export const config = makeConfig(defaultName()); // @calls makeConfig defaultName
