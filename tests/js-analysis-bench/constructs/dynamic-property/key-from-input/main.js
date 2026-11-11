// @exports
const commands = {
  build() { // @fn build
    return "build";
  },
  test() { // @fn test
    return "test";
  },
};

const requested = process.env.BENCH_COMMAND || "build";
commands[requested](); // @unknown
