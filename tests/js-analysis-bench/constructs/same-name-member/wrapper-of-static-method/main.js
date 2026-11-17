// @exports load
function Loader() {} // @fn Loader

Loader.load = function load(path) { // @fn staticLoad
  return "loaded " + path;
};

export function load(path) { // @fn load
  return Loader.load(path); // @calls staticLoad
}

load("a.txt"); // @calls load
