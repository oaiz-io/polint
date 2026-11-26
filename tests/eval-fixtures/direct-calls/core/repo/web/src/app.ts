import { importedHelper } from "./helper";

type DynamicBag = {
  [name: string]: (value: string) => string;
};

class Formatter {
  constructor(private readonly prefix: string) {}

  static normalize(value: string): string {
    return value.trim();
  }

  render(value: string): string {
    return `${this.prefix}:${value}`;
  }
}

function localTarget(value: string): string {
  // POLINT-FEATURE direct-calls/ts/static-member
  return Formatter.normalize(value);
}

function dynamicImport(path: string): Promise<unknown> {
  // POLINT-FEATURE direct-calls/ts/dynamic-import
  return import(path);
}

export function handler(input: string, bag: DynamicBag): string {
  // POLINT-FEATURE direct-calls/ts/constructor-as-function
  const ctorValue = Formatter("direct" as never);
  void ctorValue;
  // POLINT-FEATURE direct-calls/ts/constructor
  const formatter = new Formatter("direct");
  // POLINT-FEATURE direct-calls/ts/local-function
  const direct = localTarget(input);
  // POLINT-FEATURE direct-calls/ts/import-binding
  const imported = importedHelper(direct);
  // POLINT-FEATURE direct-calls/ts/instance-member
  const member = formatter.render(imported);
  const callable = localTarget;
  // POLINT-FEATURE direct-calls/ts/function-value
  const functionValue = callable(member);
  // POLINT-FEATURE direct-calls/ts/dynamic-property
  const computed = bag["plugin"](functionValue);
  // POLINT-FEATURE direct-calls/ts/call-apply-bind
  const viaCall = localTarget.call(null, computed);
  const viaApply = localTarget.apply(null, [viaCall]);
  const viaBind = localTarget.bind(null)(viaApply);
  dynamicImport("./lazy");
  return viaBind;
}
