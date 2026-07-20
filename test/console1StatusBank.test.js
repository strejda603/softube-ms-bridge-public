const test = require("node:test");
const assert = require("node:assert/strict");
const {
  STATUS_BANK_INDICATORS,
  STATUS_BANK_SIZE,
  START_SLOT_OBJECT_ID,
  buildStatusBankSlots,
  startSlotDisplayFor,
  hardwareTriggerTypeFor,
} = require("../console1StatusBank");

test("STATUS_BANK_SIZE and START_SLOT_OBJECT_ID are consistent", () => {
  assert.equal(STATUS_BANK_SIZE, 10);
  assert.equal(START_SLOT_OBJECT_ID, 9);
});

test("buildStatusBankSlots: exactly 10 slots with sequential objectIds", () => {
  const slots = buildStatusBankSlots();
  assert.equal(slots.length, 10);
  slots.forEach((slot, i) => assert.equal(slot.objectId, i));
});

test("buildStatusBankSlots: first 7 slots are status kind, in indicator order", () => {
  const slots = buildStatusBankSlots();
  const statusSlots = slots.slice(0, 7);
  assert.deepEqual(
    statusSlots.map((s) => s.kind),
    Array(7).fill("status")
  );
  assert.deepEqual(
    statusSlots.map((s) => s.statusKey),
    STATUS_BANK_INDICATORS.map((i) => i.key)
  );
  assert.deepEqual(
    statusSlots.map((s) => s.statusLabel),
    STATUS_BANK_INDICATORS.map((i) => i.label)
  );
});

test("buildStatusBankSlots: slots 7-8 are empty, slot 9 is start", () => {
  const slots = buildStatusBankSlots();
  assert.equal(slots[7].kind, "empty");
  assert.equal(slots[8].kind, "empty");
  assert.equal(slots[9].kind, "start");
});

test("buildStatusBankSlots: every slot has no Mixing Station channel mapping", () => {
  const slots = buildStatusBankSlots();
  for (const slot of slots) {
    assert.deepEqual(slot.msChannels, []);
    assert.equal(slot.msPrimary, null);
  }
});

test("startSlotDisplayFor: standby shows Start with the given main color", () => {
  const display = startSlotDisplayFor("standby", 0x00a5ff, 0x0000ff);
  assert.deepEqual(display, { name: "Start", color: 0x00a5ff });
});

test("startSlotDisplayFor: running shows Stop with the given stop color", () => {
  const display = startSlotDisplayFor("running", 0x00a5ff, 0x0000ff);
  assert.deepEqual(display, { name: "Stop", color: 0x0000ff });
});

test("startSlotDisplayFor: any non-'running' value is treated as standby", () => {
  const display = startSlotDisplayFor(undefined, 0x00a5ff, 0x0000ff);
  assert.deepEqual(display, { name: "Start", color: 0x00a5ff });
});

test("hardwareTriggerTypeFor: selected=true while standby means start", () => {
  assert.equal(hardwareTriggerTypeFor("standby", true), "start");
});

test("hardwareTriggerTypeFor: selected=true while running means stop", () => {
  assert.equal(hardwareTriggerTypeFor("running", true), "stop");
});

test("hardwareTriggerTypeFor: selected=false or undefined is not a trigger", () => {
  assert.equal(hardwareTriggerTypeFor("standby", false), null);
  assert.equal(hardwareTriggerTypeFor("standby", undefined), null);
  assert.equal(hardwareTriggerTypeFor("running", false), null);
});
