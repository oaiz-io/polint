// @exports
class Command {
  private exitCallback: (code: number) => string = function (code: number): string { // @fn defaultExit
    return "default " + code;
  };

  exitOverride(fn: (code: number) => string): this { // @fn exitOverride
    this.exitCallback = fn;
    return this;
  }

  exit(code: number): string { // @fn exit
    return this.exitCallback(code); // @calls onExit
  }
}

function onExit(code: number): string { // @fn onExit
  return "override " + code;
}

new Command().exitOverride(onExit).exit(1); // @calls exitOverride exit
