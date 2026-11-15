// @exports api
function load(id) { // @fn load
  return { id };
}

function format(item) { // @fn format
  return "item-" + item.id;
}

export const api = {
  get(id) { // @fn get
    return format(load(id)); // @calls format load
  },
  getFirst(ids) { // @fn getFirst
    return api.get(ids[0]); // @calls get
  },
};

api.getFirst([7]); // @calls getFirst
