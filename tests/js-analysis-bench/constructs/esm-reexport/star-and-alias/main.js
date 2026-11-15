// @exports
import * as shapes from "./shapes/index.js";
import { squareArea } from "./shapes/index.js";

shapes.area(2); // @calls circleArea
squareArea(3); // @calls squareArea
