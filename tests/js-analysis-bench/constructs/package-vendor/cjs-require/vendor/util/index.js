// @exports compact join
function compact(items) { // @fn compact
  return items.filter(Boolean);
}

function join(items) { // @fn join
  return items.join(",");
}

module.exports = { compact, join };
