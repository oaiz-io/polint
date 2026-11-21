// @exports
function plugin() { // @fn plugin
  return "plugin";
}

function getPlugins() { // @fn getPlugins
  return Promise.resolve({ plugin });
}

async function boot() { // @fn boot
  const plugins = await getPlugins(); // @calls getPlugins
  return plugins.plugin(); // @calls plugin
}

await boot(); // @calls boot
