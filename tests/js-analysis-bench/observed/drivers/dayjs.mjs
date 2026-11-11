// Exercises the minified dayjs build with fixed instants only, so the recorded
// calls do not depend on the clock.
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const dayjs = require("/bench/pkg/dayjs.min.js");

const step = (name, run) => {
  try {
    run();
  } catch (error) {
    console.error(`dayjs driver step ${name}: ${error.message}`);
  }
};

step("parse and format", () => {
  const base = dayjs("2020-02-29T12:34:56.789Z");
  base.format();
  base.format("YYYY-MM-DD HH:mm:ss.SSS Z ZZ [literal] d dd ddd dddd M MM MMM MMMM A a h hh");
  base.toISOString();
  base.toJSON();
  base.toString();
  base.toDate();
  base.valueOf();
  base.unix();
  dayjs(1582979696789).isValid();
  dayjs(new Date(0)).year();
  dayjs("not a date").isValid();
  dayjs("not a date").format();
  dayjs(base).clone();
  dayjs.unix(0).date();
  dayjs.isDayjs(base);
});
step("arithmetic", () => {
  const base = dayjs("2021-01-31T00:00:00.000Z");
  for (const unit of ["millisecond", "second", "minute", "hour", "day", "week", "month", "quarter", "year"]) {
    base.add(1, unit).subtract(2, unit);
    base.startOf(unit).endOf(unit);
    base.diff("2020-01-01T00:00:00.000Z", unit);
    base.diff("2020-01-01T00:00:00.000Z", unit, true);
  }
  base.set("month", 1).get("date");
  base.month(5).date(3).hour(4).minute(5).second(6).millisecond(7).day(2);
  base.daysInMonth();
  base.isBefore("2022-01-01");
  base.isAfter("2020-01-01", "year");
  base.isSame("2021-01-31", "day");
  base.utcOffset();
  base.locale();
});
step("locale and plugin", () => {
  dayjs.locale("en");
  dayjs.locale({ name: "bench", weekdays: ["S", "M", "T", "W", "T", "F", "S"], months: ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"] }, null, true);
  dayjs("2020-01-01").locale("bench").format("dddd MMMM");
  dayjs.extend((option, Dayjs, factory) => {
    Dayjs.prototype.benchTwice = function () {
      return this.add(2, "day");
    };
    factory.benchLoaded = true;
  });
  dayjs("2020-01-01").benchTwice().format();
});
