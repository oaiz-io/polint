// @exports
function loaded() { // @fn loaded
  return "loaded";
}

async function load() { // @fn load
  return loaded;
}

async function main() { // @fn main
  const fn = await load(); // @calls load
  return fn(); // @calls loaded
}

await main(); // @calls main
