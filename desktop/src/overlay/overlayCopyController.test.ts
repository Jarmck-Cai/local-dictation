import assert from "node:assert/strict";
import {
  OverlayCopyController,
  type OverlayCopyNotice,
  type OverlayCopyState,
} from "./overlayCopyController";

// Copies and dismissals stop at the controller boundary: no native clipboard,
// keyboard, microphone, model, or personal history is touched by these tests.
class ManualClock {
  now = 0;
  tasks: { at: number; callback: () => void; cancelled: boolean }[] = [];

  schedule = (callback: () => void, delay: number) => {
    const task = { at: this.now + delay, callback, cancelled: false };
    this.tasks.push(task);
    return () => {
      task.cancelled = true;
    };
  };

  advance(ms: number) {
    this.now += ms;
    const ready = this.tasks.filter((task) => task.at <= this.now);
    this.tasks = this.tasks.filter((task) => task.at > this.now);
    for (const task of ready) if (!task.cancelled) task.callback();
  }
}

const deferred = () => {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
};

const firstNotice: OverlayCopyNotice = {
  failures: 1,
  failure_generation: 7,
  capture_generation: 4,
};

const setup = () => {
  const clock = new ManualClock();
  const copies: ReturnType<typeof deferred>[] = [];
  const states: OverlayCopyState[] = [];
  const dismissals: number[] = [];
  const controller = new OverlayCopyController({
    copy: () => {
      const result = deferred();
      copies.push(result);
      return result.promise;
    },
    dismiss: async (generation) => {
      dismissals.push(generation);
    },
    onCopyState: (state) => states.push(state),
    schedule: clock.schedule,
  });
  controller.show(firstNotice);
  return { controller, clock, copies, states, dismissals };
};

// Removing the timer, starting it before copy succeeds, or passing a different
// failure generation must break this user-visible acknowledgement contract.
{
  const h = setup();
  const copy = h.controller.copy();
  h.clock.advance(5000);
  assert.deepEqual(h.dismissals, [], "pending copy never starts dismissal");
  h.copies[0].resolve();
  await copy;
  assert.equal(h.states.slice(-1)[0], "copied");
  h.clock.advance(1499);
  assert.deepEqual(
    h.dismissals,
    [],
    "successful copy remains visible for feedback",
  );
  h.clock.advance(1);
  assert.deepEqual(
    h.dismissals,
    [7],
    "successful copy dismisses after 1.5 seconds",
  );
  h.clock.advance(5000);
  assert.deepEqual(h.dismissals, [7], "the timer dismisses only once");
}

// A rejection must leave the recovery card available for a retry or manual close.
{
  const h = setup();
  const copy = h.controller.copy();
  h.copies[0].reject(new Error("synthetic clipboard refusal"));
  await copy;
  h.clock.advance(10000);
  assert.equal(h.states.slice(-1)[0], "failed");
  assert.deepEqual(h.dismissals, []);
  const retry = h.controller.copy();
  h.copies[1].resolve();
  await retry;
  h.clock.advance(1500);
  assert.deepEqual(h.dismissals, [7], "a later successful retry may dismiss");
}

const changes = {
  newFailure: (controller: OverlayCopyController) =>
    controller.show({ ...firstNotice, failure_generation: 8 }),
  newCapture: (controller: OverlayCopyController) =>
    controller.show({ ...firstNotice, capture_generation: 5 }),
  hidden: (controller: OverlayCopyController) => controller.hide(),
  progressRestored: (controller: OverlayCopyController) =>
    controller.show({ ...firstNotice, failures: 0 }),
  sameFailureShownAgain: (controller: OverlayCopyController) => {
    controller.hide();
    controller.show(firstNotice);
  },
};

// A late copy result must not change feedback or schedule dismissal on a newer
// card/capture. Check success and rejection separately, including reused IDs.
for (const [name, change] of Object.entries(changes)) {
  for (const outcome of ["success", "failure"] as const) {
    const h = setup();
    const copy = h.controller.copy();
    change(h.controller);
    const statesAfterChange = [...h.states];
    if (outcome === "success") h.copies[0].resolve();
    else h.copies[0].reject(new Error("synthetic late failure"));
    await copy;
    h.clock.advance(10000);
    assert.deepEqual(
      h.states,
      statesAfterChange,
      `${name}: stale ${outcome} ignored`,
    );
    assert.deepEqual(h.dismissals, [], `${name}: stale copy cannot dismiss`);
  }
}

// Cancellation alone is insufficient: a browser timer may already be queued.
// Execute that captured callback directly to verify the identity guard too.
for (const [name, change] of Object.entries(changes)) {
  const h = setup();
  const copy = h.controller.copy();
  h.copies[0].resolve();
  await copy;
  const queued = h.clock.tasks[0].callback;
  change(h.controller);
  queued();
  h.clock.advance(10000);
  assert.deepEqual(
    h.dismissals,
    [],
    `${name}: stale queued timer cannot dismiss`,
  );
}

// Readiness and underlying recognition/cleanup updates reuse the same notice;
// they must not reset the already-confirmed copy feedback or its timer.
{
  const h = setup();
  const copy = h.controller.copy();
  h.copies[0].resolve();
  await copy;
  h.clock.advance(1000);
  h.controller.show({ ...firstNotice });
  assert.equal(h.states.slice(-1)[0], "copied");
  h.clock.advance(500);
  assert.deepEqual(h.dismissals, [7]);
}

// Manual close acknowledges immediately but invalidates both pending promises
// and scheduled auto-close callbacks, without cancelling the active recording.
for (const copyDone of [false, true]) {
  const h = setup();
  const copy = h.controller.copy();
  if (copyDone) {
    h.copies[0].resolve();
    await copy;
  }
  const queued = h.clock.tasks[0]?.callback;
  await h.controller.dismiss();
  if (!copyDone) {
    h.copies[0].resolve();
    await copy;
  }
  queued?.();
  h.clock.advance(10000);
  assert.deepEqual(h.dismissals, [7], "manual close is the only dismissal");
}

// Repeated clicks while a copy is pending must share that attempt; a new copy
// after success resets the feedback interval from the new successful result.
{
  const h = setup();
  const first = h.controller.copy();
  await h.controller.copy();
  assert.equal(h.copies.length, 1);
  h.copies[0].resolve();
  await first;
  h.clock.advance(1000);
  const oldTimer = h.clock.tasks[0].callback;
  const second = h.controller.copy();
  oldTimer();
  h.clock.advance(1000);
  assert.deepEqual(h.dismissals, []);
  h.copies[1].resolve();
  await second;
  h.clock.advance(1499);
  assert.deepEqual(h.dismissals, []);
  h.clock.advance(1);
  assert.deepEqual(h.dismissals, [7]);
}

{
  const h = setup();
  h.controller.hide();
  await h.controller.copy();
  await h.controller.dismiss();
  assert.deepEqual(h.copies, [], "a hidden card cannot copy");
  assert.deepEqual(h.dismissals, [], "a hidden card cannot dismiss");
}

console.log("overlayCopyController: all timing and race assertions passed");
