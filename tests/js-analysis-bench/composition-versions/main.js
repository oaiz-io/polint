// Two installs of one package: `dual` at the top is version 1, and `outer`
// carries its own `dual`, version 2. Each import reaches the install Node
// resolves from the importing file.
const dual = require("dual");
const outer = require("outer");

dual.run(); // @calls runOne
outer.run(); // @calls outerRun
