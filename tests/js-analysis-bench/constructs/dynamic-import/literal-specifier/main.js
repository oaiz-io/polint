// @exports
async function boot() { // @fn boot
  const plugin = await import("./plugin.js"); // @external
  return plugin.activate("demo"); // @calls activate
}

await boot(); // @calls boot
