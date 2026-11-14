// @exports pipeline
function builder(items) { // @fn builder
  return {
    add(item) { // @fn add
      const next = items.concat(item); // @external
      return builder(next); // @calls builder
    },
    done() { // @fn done
      return items.length;
    },
  };
}

function start(value) { // @fn start
  return builder([value]); // @calls builder
}

export const pipeline = start(1).add(2).done(); // @calls start add done
