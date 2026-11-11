// @exports
const api = {
  get(id) { // @fn get
    return api.format(id); // @calls format
  },
  format: (id) => "item-" + id, // @fn format
};

api.get(7); // @calls get
