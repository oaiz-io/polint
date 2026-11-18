// @exports helper
function helper(value) { // @fn helper
  return value;
}
Object.defineProperty(exports, "helper", {
  enumerable: true,
  get: function () { // @fn get
    return helper;
  },
});
