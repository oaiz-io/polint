// @exports save remove
function save(record) { // @fn save
  return record;
}

function remove(record) { // @fn remove
  return record;
}

const actions = { save, remove };
module.exports = actions;

function dispatch(action, record) { // @fn dispatch
  return actions[action](record); // @unknown
}

dispatch(process.argv.length > 99 ? "remove" : "save", 1); // @calls dispatch
dispatch("remove", 2); // @calls dispatch
