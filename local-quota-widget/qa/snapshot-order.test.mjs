import assert from "node:assert/strict";
import {
  isRecentSnapshot,
  shouldReplaceSnapshot,
  timestampMilliseconds,
} from "../web/snapshot-order.mjs";
import {
  isFiveHourWindow,
  orderedQuotaWindows,
  selectPrimaryQuotaWindow,
} from "../web/quota-display.mjs";
import {
  autoRefreshIntervalLabel,
  normalizeAutoRefreshInterval,
} from "../web/refresh-settings.mjs";

function snapshot(source, observedAt, remainingPercent = 80) {
  return {
    status: "ok",
    source,
    observedAt,
    primary: { remainingPercent },
  };
}

const online = snapshot("online", "2026-08-11T04:00:00.000Z", 92);
const olderLocal = snapshot("local", "2026-08-09T04:00:00.000Z", 70);
const newerLocal = snapshot("local", "2026-08-11T04:01:00.000Z", 91);
const equalLocal = snapshot("local", "2026-08-11T04:00:00.000Z", 80);

assert.equal(shouldReplaceSnapshot(online, olderLocal), false);
assert.equal(shouldReplaceSnapshot(online, newerLocal), true);
assert.equal(shouldReplaceSnapshot(online, equalLocal), false);
assert.equal(
  shouldReplaceSnapshot(newerLocal, olderLocal, { force: true }),
  true,
);
assert.equal(
  shouldReplaceSnapshot(online, { status: "missing", source: "local" }),
  false,
);
assert.equal(
  shouldReplaceSnapshot({ status: "missing" }, newerLocal),
  true,
);
assert.equal(timestampMilliseconds("1786420800"), 1_786_420_800_000);
assert.equal(timestampMilliseconds("1786420800000"), 1_786_420_800_000);
assert.equal(
  isRecentSnapshot(online, {
    now: timestampMilliseconds("2026-08-11T04:01:30.000Z"),
  }),
  true,
);
assert.equal(
  isRecentSnapshot(olderLocal, {
    now: timestampMilliseconds("2026-08-11T04:01:30.000Z"),
  }),
  false,
);

const weeklyWindow = {
  label: "7 天额度",
  remainingPercent: 12,
  windowMinutes: 10_080,
};
const fiveHourWindow = {
  label: "5 小时额度",
  remainingPercent: 76,
  windowMinutes: 300,
};
assert.equal(isFiveHourWindow(fiveHourWindow), true);
assert.equal(isFiveHourWindow(weeklyWindow), false);
assert.deepEqual(orderedQuotaWindows([weeklyWindow, fiveHourWindow]), [
  fiveHourWindow,
  weeklyWindow,
]);
assert.equal(
  selectPrimaryQuotaWindow([weeklyWindow, fiveHourWindow]),
  fiveHourWindow,
);
assert.equal(selectPrimaryQuotaWindow([weeklyWindow]), weeklyWindow);
assert.equal(normalizeAutoRefreshInterval(), 1);
assert.equal(normalizeAutoRefreshInterval("5"), 5);
assert.equal(normalizeAutoRefreshInterval("7"), 1);
assert.equal(autoRefreshIntervalLabel(30), "每 30 分钟");

console.log("snapshot ordering tests passed");
