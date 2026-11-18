// @exports
function task() { // @fn task
  return "task";
}

function other() { // @fn other
  return "other";
}

const run = (fn, value) => fn(value); // @fn run @calls task other

const runner = {
  invoke(fn) { // @fn invoke
    return fn(); // @calls other
  },
};

function identity(fn) { // @fn identity
  return fn;
}

run(task, 1); // @calls run
run(other, 2); // @calls run
runner.invoke(other); // @calls invoke
identity(task)(); // @calls identity task
