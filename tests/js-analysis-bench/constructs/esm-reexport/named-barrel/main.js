// @exports
import { parse, render } from "./index.js";

render(parse("a,b")); // @calls render parse
